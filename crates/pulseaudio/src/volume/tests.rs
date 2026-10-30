
use super::*;

#[derive(Default)]
struct Seen(Vec<VolumeState>);
impl Resource for Seen {}

fn record_change(app: &mut App, _: &OnChanged<VolumeState>) {
    let state = *app.resource::<VolumeState>();
    app.resource_mut::<Seen>().0.push(state);
}

#[test]
fn volumes_are_percentages_of_normal_level_not_clamped_at_100() {
    assert_eq!(volume_percent(0), 0);
    assert_eq!(volume_percent(PA_VOLUME_NORM / 2), 50);
    assert_eq!(volume_percent(PA_VOLUME_NORM), 100);
    assert_eq!(volume_percent(PA_VOLUME_NORM * 3 / 2), 150);
}

#[test]
fn slider_percentages_create_valid_mono_volumes() {
    for (percent, expected) in [
        (0, 0),
        (50, PA_VOLUME_NORM / 2),
        (100, PA_VOLUME_NORM),
        (u32::MAX, PA_VOLUME_NORM),
    ] {
        let volume = cvolume_for_percent(percent);
        assert_eq!(volume.channels, 1);
        assert_eq!(volume.values[0], expected);
        // SAFETY: the helper built a C volume with one valid channel.
        assert_ne!(unsafe { pa_cvolume_valid(&volume) }, 0);
    }
}

#[test]
fn sink_snapshot_publishes_only_after_the_reply_finishes() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);
    {
        let mut backend = app.resource_mut::<VolumeBackend>();
        backend.started = true;
        backend.loading = true;
    }
    app.resource::<VolumeBackend>()
        .updates
        .borrow_mut()
        .push(VolumeUpdate::Sink(VolumeState::Ready {
            volume_percent: 37,
            muted: true,
        }));

    on_tick(&mut app, &Tick);
    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Loading);

    app.resource::<VolumeBackend>()
        .updates
        .borrow_mut()
        .push(VolumeUpdate::SinkDone);
    on_tick(&mut app, &Tick);
    assert_eq!(
        *app.resource::<VolumeState>(),
        VolumeState::Ready {
            volume_percent: 37,
            muted: true,
        }
    );
}

#[test]
fn a_new_snapshot_emits_on_changed_for_volume_state() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule)
        .init_resource::<Seen>()
        .system(record_change);
    {
        let mut backend = app.resource_mut::<VolumeBackend>();
        backend.started = true;
        backend.loading = true;
    }
    let state = VolumeState::Ready {
        volume_percent: 62,
        muted: true,
    };
    app.resource::<VolumeBackend>()
        .updates
        .borrow_mut()
        .extend([VolumeUpdate::Sink(state), VolumeUpdate::SinkDone]);

    app.tick();
    assert_eq!(app.resource::<Seen>().0, [state]);
    app.tick();
    assert_eq!(app.resource::<Seen>().0, [state]);
}

#[test]
fn only_sink_and_server_subscriptions_request_a_refresh() {
    let backend = VolumeBackend::default();
    let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
    on_subscription(ptr::null_mut(), PA_SUBSCRIPTION_EVENT_SINK, 1, userdata);
    on_subscription(ptr::null_mut(), PA_SUBSCRIPTION_EVENT_SERVER, 0, userdata);
    on_subscription(ptr::null_mut(), 1, 2, userdata); // A source change.
    assert_eq!(backend.updates.borrow().len(), 2);
}

#[test]
fn connection_failure_resets_volume_without_stopping_the_app() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);
    app.resource_mut::<VolumeState>()
        .set_if_neq(VolumeState::Ready {
            volume_percent: 37,
            muted: true,
        });

    on_connection(
        &mut app,
        &PulseAudioContextConnectionStateChanged(ConnectionState::Failed("audio went away".into())),
    );

    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
}

#[test]
fn installing_after_a_connection_failure_reports_unavailable() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>();
    app.resource_mut::<PulseAudio>().connection_state =
        ConnectionState::Failed("audio went away".into());

    app.add_module(VolumeControlModule);

    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
}

#[test]
fn duplicate_snapshots_do_not_flag_the_resource_as_changed() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);
    let value = VolumeState::Ready {
        volume_percent: 64,
        muted: false,
    };
    app.resource_mut::<VolumeState>().set_if_neq(value);
    app.take_resource_changed::<VolumeState>();
    {
        let mut backend = app.resource_mut::<VolumeBackend>();
        backend.started = true;
        backend.loading = true;
    }
    app.resource::<VolumeBackend>()
        .updates
        .borrow_mut()
        .extend([VolumeUpdate::Sink(value), VolumeUpdate::SinkDone]);

    on_tick(&mut app, &Tick);

    assert!(!app.take_resource_changed::<VolumeState>());
    assert_eq!(*app.resource::<VolumeState>(), value);
}

#[test]
fn null_sink_reply_sets_unavailable_instead_of_panicking() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);
    {
        let mut backend = app.resource_mut::<VolumeBackend>();
        backend.started = true;
        backend.loading = true;
    }
    let backend = app.resource::<VolumeBackend>();
    let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
    on_sink(ptr::null_mut(), ptr::null(), -1, userdata);

    on_tick(&mut app, &Tick);

    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
}

#[test]
fn shared_connection_keeps_callback_data_alive_after_volume_backend_drops() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);
    let weak = Rc::downgrade(&app.resource::<VolumeBackend>().updates);

    // The volume backend can be dropped first during App teardown.
    let backend = app.insert_resource(VolumeBackend::default());
    drop(backend);
    assert!(weak.upgrade().is_some());

    drop(app);
    assert!(weak.upgrade().is_none());
}

#[test]
fn set_muted_is_a_no_op_without_a_ready_output_or_connection() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);

    app.signal(SetMuted(true));
    app.flush();
    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Loading);

    app.resource_mut::<VolumeState>()
        .set_if_neq(VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        });
    app.signal(SetMuted(false));
    app.flush();
    assert_eq!(
        *app.resource::<VolumeState>(),
        VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        }
    );
}

#[test]
fn set_volume_is_a_no_op_without_a_ready_output_or_connection() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>()
        .add_module(VolumeControlModule);

    app.signal(SetVolume(75));
    app.flush();
    assert_eq!(*app.resource::<VolumeState>(), VolumeState::Loading);

    app.resource_mut::<VolumeState>()
        .set_if_neq(VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        });
    app.signal(SetVolume(75));
    app.flush();
    assert_eq!(
        *app.resource::<VolumeState>(),
        VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        }
    );
}

#[test]
fn mute_replies_request_refresh_without_changing_observed_mute() {
    for success in [0, 1] {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        let old = VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        };
        app.resource_mut::<VolumeState>().set_if_neq(old);
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.loading = true; // Wait until the current sink query completes.
        }
        let backend = app.resource::<VolumeBackend>();
        let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
        on_mute_done(ptr::null_mut(), success, userdata);

        on_tick(&mut app, &Tick);

        assert!(app.resource::<VolumeBackend>().dirty);
        assert_eq!(*app.resource::<VolumeState>(), old);
    }
}

#[test]
fn volume_replies_request_refresh_without_changing_observed_volume() {
    for success in [0, 1] {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        let old = VolumeState::Ready {
            volume_percent: 50,
            muted: false,
        };
        app.resource_mut::<VolumeState>().set_if_neq(old);
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.loading = true;
        }
        let backend = app.resource::<VolumeBackend>();
        let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
        on_volume_done(ptr::null_mut(), success, userdata);

        on_tick(&mut app, &Tick);

        assert!(app.resource::<VolumeBackend>().dirty);
        assert_eq!(*app.resource::<VolumeState>(), old);
    }
}
