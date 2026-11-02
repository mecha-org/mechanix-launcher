use std::time::Instant;

use super::*;

#[derive(Default)]
struct TimerData {
    calls: Cell<usize>,
    timer_id: Cell<u64>,
    api: Cell<*const pa_mainloop_api>,
    event: Cell<*mut pa_time_event>,
    deadline: Cell<Option<(libc::time_t, libc::suseconds_t)>>,
    disabled_before_call: Cell<bool>,
    created: Cell<*mut pa_time_event>,
}

extern "C" fn record_timer(
    api: *const pa_mainloop_api,
    event: *mut pa_time_event,
    deadline: *const timeval,
    userdata: *mut c_void,
) {
    // SAFETY: the test owns TimerData, and the dispatcher owns a timestamp copy
    // valid throughout this callback. No state borrow survives invocation.
    let data = unsafe { &*userdata.cast::<TimerData>() };
    let deadline = unsafe { *deadline };
    data.calls.set(data.calls.get() + 1);
    data.api.set(api);
    data.event.set(event);
    data.deadline.set(Some((deadline.tv_sec, deadline.tv_usec)));
    // SAFETY: the API's state is live and the dispatcher released its borrows.
    let state = unsafe { &*(*api).userdata.cast::<mainloop::MainloopState>() };
    data.disabled_before_call.set(
        !state
            .enabled_timers()
            .any(|(id, _, _)| id == data.timer_id.get()),
    );
}

fn add_timer(
    app: &App,
    deadline: Option<&timeval>,
    callback: pa_time_event_cb_t,
    data: &TimerData,
) -> *mut pa_time_event {
    let audio = app.resource::<PulseAudio>();
    audio.mainloop_api.time_new.unwrap()(
        audio.mainloop_api.as_ref(),
        deadline.map_or(ptr::null(), ptr::from_ref),
        callback,
        ptr::from_ref(data).cast_mut().cast(),
    )
}

fn pending_timer(app: &App, data: &TimerData) -> Token {
    let audio = app.resource::<PulseAudio>();
    assert_eq!(audio.pending_timers.len(), 1);
    let (&token, timer) = audio.pending_timers.iter().next().unwrap();
    data.timer_id.set(timer.timer_id);
    token
}

fn arm_timer(
    app: &mut App,
    deadline: &timeval,
    callback: pa_time_event_cb_t,
    data: &TimerData,
) -> (*mut pa_time_event, Token) {
    let event = add_timer(app, Some(deadline), callback, data);
    before_wait(app);
    (event, pending_timer(app, data))
}

// Dispatch the whole completion batch, including obsolete timers, with a bounded
// test deadline. The guard itself belongs to this helper, not PulseAudio.
fn dispatch_timer(app: &mut App, token: Token) {
    let guard = app.resource_mut::<Ring>().timeout(Duration::from_secs(1));
    loop {
        let events = app.resource_mut::<Ring>().wait();
        let done = events.iter().any(|event| event.token == token);
        for event in events {
            if event.token == guard {
                app.resource_mut::<Ring>().finish(guard).unwrap();
                assert!(done, "timer did not complete");
            } else {
                app.signal(event);
            }
        }
        app.flush();
        if done {
            return;
        }
    }
}

fn after(delay: Duration) -> timeval {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap() + delay;
    timeval {
        tv_sec: timestamp.as_secs().try_into().unwrap(),
        tv_usec: timestamp.subsec_micros().into(),
    }
}

#[test]
fn absolute_timer_deadlines_become_relative_delays() {
    let now = UNIX_EPOCH + Duration::from_millis(100_250);
    let deadline = timeval {
        tv_sec: 102,
        tv_usec: 750_000,
    };
    assert_eq!(
        timer_delay(deadline, now).unwrap(),
        Duration::from_millis(2500)
    );
    assert_eq!(
        timer_delay(deadline, now + Duration::from_secs(3)).unwrap(),
        Duration::ZERO
    );

    let before_epoch = timeval {
        tv_sec: -1,
        tv_usec: 500_000,
    };
    assert_eq!(
        timer_delay(before_epoch, UNIX_EPOCH).unwrap(),
        Duration::ZERO
    );
    assert_eq!(
        timer_delay(before_epoch, UNIX_EPOCH - Duration::from_secs(2)).unwrap(),
        Duration::from_millis(1500)
    );
}

#[test]
fn invalid_timer_microseconds_are_rejected() {
    for tv_usec in [-1, 1_000_000] {
        let error = timer_delay(timeval { tv_sec: 0, tv_usec }, UNIX_EPOCH).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

#[test]
fn timers_expire_once_and_are_finished_before_the_callback() {
    let data = TimerData::default();
    let mut app = polling_app();
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 123,
    };
    let (event, token) = arm_timer(&mut app, &deadline, Some(record_timer), &data);
    before_wait(&mut app);
    assert_eq!(app.resource::<PulseAudio>().pending_timers.len(), 1);

    dispatch_timer(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.event.get(), event);
    assert_eq!(data.deadline.get(), Some((0, 123)));
    assert!(data.disabled_before_call.get());
    assert_eq!(
        data.api.get(),
        ptr::from_ref(app.resource::<PulseAudio>().mainloop_api.as_ref())
    );
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
    assert!(app.resource_mut::<Ring>().finish(token).is_none());
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
    assert_eq!(data.calls.get(), 1);
}

#[test]
fn future_deadlines_do_not_fire_immediately() {
    let data = TimerData::default();
    let mut app = polling_app();
    let start = Instant::now();
    let deadline = after(Duration::from_millis(20));
    let (_, token) = arm_timer(&mut app, &deadline, Some(record_timer), &data);

    dispatch_timer(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert!(start.elapsed() >= Duration::from_millis(10));
    assert_eq!(
        data.deadline.get(),
        Some((deadline.tv_sec, deadline.tv_usec))
    );
}

#[test]
fn restarted_generations_get_a_new_timeout_and_obsolete_completions_are_finished() {
    let data = TimerData::default();
    let mut app = polling_app();
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let (event, old) = arm_timer(&mut app, &deadline, Some(record_timer), &data);
    let replacement = timeval {
        tv_sec: 0,
        tv_usec: 1,
    };
    app.resource::<PulseAudio>()
        .mainloop_api
        .time_restart
        .unwrap()(event, &replacement);
    before_wait(&mut app);
    before_wait(&mut app);

    let audio = app.resource::<PulseAudio>();
    assert_eq!(audio.pending_timers.len(), 2);
    let current = *audio
        .pending_timers
        .iter()
        .find(|(_, timer)| timer.generation == 1)
        .unwrap()
        .0;
    dispatch_timer(&mut app, current);
    if app
        .resource::<PulseAudio>()
        .pending_timers
        .contains_key(&old)
    {
        dispatch_timer(&mut app, old);
    }

    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.deadline.get(), Some((0, 1)));
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
    assert!(app.resource_mut::<Ring>().finish(old).is_none());
}

#[test]
fn disabled_timers_are_not_submitted_and_old_completions_are_ignored() {
    let data = TimerData::default();
    let mut app = polling_app();
    let event = add_timer(&app, None, Some(record_timer), &data);
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());

    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    app.resource::<PulseAudio>()
        .mainloop_api
        .time_restart
        .unwrap()(event, &deadline);
    before_wait(&mut app);
    let token = pending_timer(&app, &data);
    app.resource::<PulseAudio>()
        .mainloop_api
        .time_restart
        .unwrap()(event, ptr::null());

    dispatch_timer(&mut app, token);

    assert_eq!(data.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
    assert!(app.resource_mut::<Ring>().finish(token).is_none());
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
}

#[test]
fn freed_timers_are_not_called_but_their_timeouts_are_finished() {
    let data = TimerData::default();
    let mut app = polling_app();
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let (event, token) = arm_timer(&mut app, &deadline, Some(record_timer), &data);
    app.resource::<PulseAudio>().mainloop_api.time_free.unwrap()(event);

    dispatch_timer(&mut app, token);

    assert_eq!(data.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
    assert!(app.resource_mut::<Ring>().finish(token).is_none());
}

extern "C" fn restart_once(
    api: *const pa_mainloop_api,
    event: *mut pa_time_event,
    deadline: *const timeval,
    userdata: *mut c_void,
) {
    record_timer(api, event, deadline, userdata);
    // SAFETY: the test owns the data and the dispatcher keeps the API and its
    // copied timestamp alive. Restarting copies the timestamp again.
    let data = unsafe { &*userdata.cast::<TimerData>() };
    if data.calls.get() == 1 {
        unsafe { &*api }.time_restart.unwrap()(event, deadline);
    }
}

#[test]
fn a_timer_callback_can_rearm_itself_at_the_same_deadline() {
    let data = TimerData::default();
    let mut app = polling_app();
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let (_, first) = arm_timer(&mut app, &deadline, Some(restart_once), &data);

    dispatch_timer(&mut app, first);
    assert_eq!(data.calls.get(), 1);
    assert!(data.disabled_before_call.get());
    before_wait(&mut app);
    let second = pending_timer(&app, &data);
    assert_ne!(first, second);
    dispatch_timer(&mut app, second);

    assert_eq!(data.calls.get(), 2);
    assert!(data.disabled_before_call.get());
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
}

extern "C" fn free_on_expiry(
    api: *const pa_mainloop_api,
    event: *mut pa_time_event,
    deadline: *const timeval,
    userdata: *mut c_void,
) {
    record_timer(api, event, deadline, userdata);
    // SAFETY: these arguments are live, and the test owns userdata independently
    // of the registration. The copied timestamp outlives the freed timer.
    unsafe { &*api }.time_free.unwrap()(event);
    let deadline = unsafe { *deadline };
    let data = unsafe { &*userdata.cast::<TimerData>() };
    data.deadline.set(Some((deadline.tv_sec, deadline.tv_usec)));
}

#[test]
fn a_timer_callback_can_free_itself_without_invalidating_the_timestamp() {
    let data = TimerData::default();
    let mut app = polling_app();
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 123,
    };
    let (_, token) = arm_timer(&mut app, &deadline, Some(free_on_expiry), &data);

    dispatch_timer(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.deadline.get(), Some((0, 123)));
    assert!(data.disabled_before_call.get());
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_timers.is_empty());
}

extern "C" fn create_timer_from_deferred(
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    let deadline = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    // SAFETY: the test owns TimerData and keeps the API alive during dispatch.
    let api_ref = unsafe { &*api };
    let timer = api_ref.time_new.unwrap()(api, &deadline, Some(record_timer), userdata);
    unsafe { &*userdata.cast::<TimerData>() }.created.set(timer);
    api_ref.defer_enable.unwrap()(event, 0);
}

#[test]
fn timers_created_by_deferred_work_are_armed_before_waiting() {
    let data = TimerData::default();
    let mut app = polling_app();
    let audio = app.resource::<PulseAudio>();
    audio.mainloop_api.defer_new.unwrap()(
        audio.mainloop_api.as_ref(),
        Some(create_timer_from_deferred),
        ptr::from_ref(&data).cast_mut().cast(),
    );

    before_wait(&mut app);

    assert!(!data.created.get().is_null());
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
    let token = pending_timer(&app, &data);
    dispatch_timer(&mut app, token);
    assert_eq!(data.calls.get(), 1);
}
