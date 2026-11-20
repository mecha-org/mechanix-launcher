//! The launcher's logind session on the system bus: lock and unlock
//! requests, the locked and idle hints, and a delay inhibitor so the
//! screen locks before the system sleeps.
//!
//! - The session is `XDG_SESSION_ID`'s, which `mechanix-session` exports
//!   to the user manager. The launcher is a user service, outside the
//!   session's scope, so without it the session is logind's `auto`: the
//!   user's display session.
//! - Either way, the session is used only once it checks out as this
//!   user's Wayland session, not closing: a stale or wrong
//!   `XDG_SESSION_ID` leaves the shell without a session (logged), rather
//!   than locking, hinting or answering for someone else's.
//! - When the session is gone (logind's `SessionRemoved`, or no such
//!   session to begin with), [`SessionEnded`] is signalled and
//!   `mechanix-session-shutdown.target` started, which stops the session's
//!   units, this one among them. `mechanix-session` does that itself when
//!   the session ends; this covers it being killed before it could, which
//!   would otherwise leave the compositor and shell running without a
//!   seat.
//! - logind's `Lock` and `Unlock` on that session (`loginctl lock-session`,
//!   `HandlePowerKey=lock`, `IdleAction=lock`) arrive as [`LockRequested`]
//!   and [`UnlockRequested`].
//! - A `delay` inhibitor for `sleep` is held while awake. `PrepareForSleep`
//!   arrives as [`PrepareForSleep`]: going to sleep, the lockscreen locks
//!   and then answers with [`SleepReady`], which drops the inhibitor so the
//!   system goes on. logind waits for that at most `InhibitDelayMaxSec`.
//!   Waking up takes the inhibitor again.
//! - [`SessionLockChanged`] sets the session's `LockedHint`, and
//!   [`SetIdleHint`] its `IdleHint`.
//! - [`TakePowerKey`] takes a `block` inhibitor for `handle-power-key`, so
//!   logind leaves the power key to the shell. Send it only once the shell
//!   can see the key itself.
//!
//! Everything is best effort: a call that fails is logged, and the shell
//! keeps working without logind.

use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;

use mecha_wayland::prelude::*;

use crate::logind::{LOGIND, MANAGER, MANAGER_PATH, NO_SUCH_SESSION, SESSION};

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// Stops the session's units, as `mechanix-session` does on its way out.
const SHUTDOWN_TARGET: &str = "mechanix-session-shutdown.target";

/// The logind session is gone; its units are being stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEnded;
impl Signal for SessionEnded {}

/// logind asked the session to lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockRequested;
impl Signal for LockRequested {}

/// logind asked the session to unlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlockRequested;
impl Signal for UnlockRequested {}

/// The system is about to sleep (`true`) or has woken (`false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrepareForSleep(pub bool);
impl Signal for PrepareForSleep {}

/// Whatever had to happen before sleep has: let the system sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepReady;
impl Signal for SleepReady {}

/// The session is locked, the compositor having confirmed it (`true`), or
/// unlocked (`false`). logind's `LockedHint` follows, and so does the
/// screen saver's `GetActive` (see [`crate::keep_awake`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLockChanged(pub bool);
impl Signal for SessionLockChanged {}

/// The shell handles the power key now: keep logind's hands off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakePowerKey;
impl Signal for TakePowerKey {}

/// Set the session's `IdleHint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetIdleHint(pub bool);
impl Signal for SetIdleHint {}

/// The logind inhibitors the shell holds, each taken and released on its
/// own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inhibitor {
    /// `sleep`, `delay`: held while awake, released once locked for sleep.
    Sleep,
    /// `handle-power-key`, `block`: the shell handles the key.
    PowerKey,
}

impl Inhibitor {
    fn args(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Inhibitor::Sleep => ("sleep", "Lock the screen before sleep", "delay"),
            Inhibitor::PowerKey => (
                "handle-power-key",
                "The shell handles the power key",
                "block",
            ),
        }
    }
}

/// Where an inhibitor stands.
#[derive(Debug, Default)]
enum Hold {
    #[default]
    Released,
    /// Asked for; the reply is in flight.
    Asking,
    /// Kept for its fd: dropping it releases the inhibitor.
    Held { _fd: OwnedFd },
}

/// A call in flight, by what its reply is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    GetSession,
    /// The session's properties, for [`check`].
    Check,
    Inhibit(Inhibitor),
    Hint(&'static str),
}

/// What the session is to the shell, by its properties (see [`check`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Standing {
    /// Not known yet: `GetSession` or its check is in flight.
    Asking,
    /// This user's Wayland session: hints go to it, and its `Lock` and
    /// `Unlock` are heeded, through `lock_match`.
    Ours { lock_match: MatchId },
    /// Ours, but closing: left alone, though its end is still ours.
    Closing,
    /// Not this user's Wayland session, for the reason given: left alone,
    /// its end too.
    Foreign(String),
    /// Ended, and its units asked to stop.
    Gone,
}

struct Session {
    /// What `GetSession` was asked for: `XDG_SESSION_ID`, or `auto`.
    id: String,
    /// The session's object path, once `GetSession` answered.
    path: Option<ObjectPath>,
    standing: Standing,
    sleep_match: MatchId,
    /// logind's `SessionRemoved`, for the session's end.
    removed_match: MatchId,
    /// Each inhibitor's state, by [`Inhibitor`]'s order. A reply that comes
    /// after a release finds it `Released`, and its fd is dropped.
    holds: [Hold; 2],
    /// Hints asked for before the session's path was known.
    queued: Vec<(&'static str, bool)>,
    calls: Vec<(CallId, Pending)>,
}
impl Resource for Session {}

/// Install after `RingModule` and `DBusModule::new().system()`.
pub struct SessionModule;

impl Module for SessionModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<Session>() {
            return;
        }
        let id = std::env::var("XDG_SESSION_ID")
            .ok()
            .filter(|id| !id.is_empty());
        if id.is_none() {
            eprintln!("[session] XDG_SESSION_ID is not set: using logind's display session");
        }
        install(app, id.unwrap_or_else(|| "auto".into()));
    }
}

fn install(app: &mut App, id: String) {
    let (get, sleep_match, removed_match) = {
        let mut dbus = app.resource_mut::<DBus>();
        let mut bus = dbus.system();
        let manager = || {
            MatchRule::signal()
                .sender(LOGIND)
                .path(MANAGER_PATH)
                .interface(MANAGER)
        };
        // The matches first, so no removal goes unseen once the path is known.
        let sleep_match = bus.add_match(manager().member("PrepareForSleep"));
        let removed_match = bus.add_match(manager().member("SessionRemoved"));
        let get = bus
            .object(LOGIND, MANAGER_PATH)
            .call(MANAGER, "GetSession", &(id.as_str(),));
        (get, sleep_match, removed_match)
    };
    app.insert_resource(Session {
        id,
        path: None,
        standing: Standing::Asking,
        sleep_match,
        removed_match,
        holds: Default::default(),
        queued: Vec::new(),
        calls: vec![(get, Pending::GetSession)],
    });
    take(app, Inhibitor::Sleep);
    app.system(on_reply)
        .system(on_signal)
        .system(|app, _: &SleepReady| release(app, Inhibitor::Sleep))
        .system(|app, e: &SessionLockChanged| hint(app, "SetLockedHint", e.0))
        .system(|app, e: &SetIdleHint| hint(app, "SetIdleHint", e.0))
        .system(|app, _: &TakePowerKey| take(app, Inhibitor::PowerKey))
        .system(on_disconnect);
}

/// Whether the session's properties (`GetAll`) make it `uid`'s Wayland
/// session, not closing; if not, what it is instead.
fn check(props: &PropMap, uid: u32) -> Result<(), Standing> {
    let foreign = Standing::Foreign;
    match props.get_as::<(u32, ObjectPath)>("User") {
        Some(Ok((user, _))) if user == uid => {}
        Some(Ok((user, _))) => return Err(foreign(format!("it is user {user}'s"))),
        _ => return Err(foreign("its user is unknown".into())),
    }
    match props.get_as::<String>("Type") {
        Some(Ok(kind)) if kind == "wayland" => {}
        Some(Ok(kind)) => return Err(foreign(format!("it is a {kind} session, not wayland"))),
        _ => return Err(foreign("its type is unknown".into())),
    }
    match props.get_as::<String>("State") {
        Some(Ok(state)) if state == "closing" => Err(Standing::Closing),
        Some(Ok(_)) => Ok(()),
        _ => Err(foreign("its state is unknown".into())),
    }
}

/// The session is gone: let go of it, and stop its units. Not for a
/// session that was never ours, nor twice.
fn ended(app: &mut App) {
    let was = {
        let mut session = app.resource_mut::<Session>();
        if matches!(session.standing, Standing::Foreign(_) | Standing::Gone) {
            return;
        }
        std::mem::replace(&mut session.standing, Standing::Gone)
    };
    eprintln!("[session] the session is gone: starting {SHUTDOWN_TARGET}");
    {
        let mut dbus = app.resource_mut::<DBus>();
        if let Standing::Ours { lock_match } = was {
            dbus.system().remove_match(lock_match);
        }
        if let Some(bus) = dbus.bus(BusId::Session) {
            bus.object(SYSTEMD, SYSTEMD_PATH).call_no_reply(
                SYSTEMD_MANAGER,
                "StartUnit",
                &(SHUTDOWN_TARGET, "replace-irreversibly"),
            );
        }
    }
    app.signal(SessionEnded);
}

/// Take `what`, unless held or asked for already.
fn take(app: &mut App, what: Inhibitor) {
    {
        let mut session = app.resource_mut::<Session>();
        let hold = &mut session.holds[what as usize];
        if !matches!(hold, Hold::Released) {
            return;
        }
        *hold = Hold::Asking;
    }
    let (kind, why, mode) = what.args();
    let call = app
        .resource_mut::<DBus>()
        .system()
        .object(LOGIND, MANAGER_PATH)
        .call(MANAGER, "Inhibit", &(kind, "Mechanix", why, mode));
    app.resource_mut::<Session>()
        .calls
        .push((call, Pending::Inhibit(what)));
}

/// Release `what`: closing its fd releases it.
fn release(app: &mut App, what: Inhibitor) {
    app.resource_mut::<Session>().holds[what as usize] = Hold::Released;
}

fn hint(app: &mut App, method: &'static str, value: bool) {
    let path = {
        let session = app.resource::<Session>();
        let ours = matches!(session.standing, Standing::Ours { .. });
        session.path.clone().filter(|_| ours)
    };
    let Some(path) = path else {
        let mut session = app.resource_mut::<Session>();
        session.queued.retain(|(m, _)| *m != method);
        session.queued.push((method, value));
        return;
    };
    let call = app
        .resource_mut::<DBus>()
        .system()
        .object(LOGIND, path)
        .call(SESSION, method, &(value,));
    app.resource_mut::<Session>()
        .calls
        .push((call, Pending::Hint(method)));
}

fn on_reply(app: &mut App, e: &MethodReply) {
    let pending = {
        let mut session = app.resource_mut::<Session>();
        let Some(i) = session.calls.iter().position(|(c, _)| *c == e.call) else {
            return;
        };
        session.calls.swap_remove(i).1
    };
    let msg = match &e.result {
        Ok(msg) => msg,
        Err(error) => {
            eprintln!("[session] {pending:?} failed: {error:?}");
            match pending {
                // Asked for again by the next `take`.
                Pending::Inhibit(what) => release(app, what),
                // The session named is gone already. (`auto` finding none
                // is no session to end.)
                Pending::GetSession
                    if error.is(NO_SUCH_SESSION) && app.resource::<Session>().id != "auto" =>
                {
                    ended(app)
                }
                _ => {}
            }
            return;
        }
    };
    match pending {
        Pending::GetSession => {
            let Ok((path,)) = msg.body::<(ObjectPath,)>() else {
                eprintln!("[session] GetSession: unexpected reply");
                return;
            };
            let call = app
                .resource_mut::<DBus>()
                .system()
                .object(LOGIND, path.clone())
                .get_all(SESSION);
            let mut session = app.resource_mut::<Session>();
            session.path = Some(path);
            session.calls.push((call, Pending::Check));
        }
        Pending::Check => {
            let Ok((props,)) = msg.body::<(PropMap,)>() else {
                eprintln!("[session] the session's properties: unexpected reply");
                return;
            };
            let uid = std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid());
            on_check(app, check(&props, uid));
        }
        Pending::Inhibit(what) => {
            let fd = match msg.body::<(UnixFd,)>().map(|(fd,)| fd.try_clone_owned()) {
                Ok(Ok(fd)) => Some(fd),
                Ok(Err(error)) => {
                    eprintln!("[session] {pending:?}: {error}");
                    None
                }
                Err(_) => {
                    eprintln!("[session] {pending:?}: unexpected reply");
                    None
                }
            };
            let mut session = app.resource_mut::<Session>();
            let hold = &mut session.holds[what as usize];
            // Released while the call was out: the fd drops here.
            if matches!(hold, Hold::Asking) {
                *hold = fd.map_or(Hold::Released, |fd| Hold::Held { _fd: fd });
            }
        }
        Pending::Hint(_) => {}
    }
}

/// The session's check came back: [`check`]'s answer.
fn on_check(app: &mut App, checked: Result<(), Standing>) {
    let path = {
        let session = app.resource::<Session>();
        // Ended while the check was out.
        if session.standing == Standing::Gone {
            return;
        }
        session.path.clone()
    };
    let Some(path) = path else {
        return;
    };
    let standing = match checked {
        Ok(()) => {
            let lock_match = app.resource_mut::<DBus>().system().add_match(
                MatchRule::signal()
                    .sender(LOGIND)
                    .path(path)
                    .interface(SESSION),
            );
            Standing::Ours { lock_match }
        }
        Err(standing) => {
            match &standing {
                Standing::Closing => eprintln!("[session] {path} is closing: not using it"),
                Standing::Foreign(why) => eprintln!("[session] not using {path}: {why}"),
                _ => {}
            }
            standing
        }
    };
    let ours = matches!(standing, Standing::Ours { .. });
    let queued = {
        let mut session = app.resource_mut::<Session>();
        session.standing = standing;
        if ours {
            std::mem::take(&mut session.queued)
        } else {
            Vec::new()
        }
    };
    for (method, value) in queued {
        hint(app, method, value);
    }
}

fn on_signal(app: &mut App, e: &SignalReceived) {
    let (lock_match, sleep_match, removed_match) = {
        let session = app.resource::<Session>();
        let lock_match = match session.standing {
            Standing::Ours { lock_match } => Some(lock_match),
            _ => None,
        };
        (lock_match, session.sleep_match, session.removed_match)
    };
    if e.matches.contains(&removed_match) {
        let ours = e
            .msg
            .body::<(String, ObjectPath)>()
            .is_ok_and(|(_, path)| app.resource::<Session>().path.as_ref() == Some(&path));
        if ours {
            ended(app);
        }
        return;
    }
    if e.matches.contains(&sleep_match) {
        let Ok((start,)) = e.msg.body::<(bool,)>() else {
            return;
        };
        if !start {
            take(app, Inhibitor::Sleep);
        }
        app.signal(PrepareForSleep(start));
        return;
    }
    if lock_match.is_none_or(|m| !e.matches.contains(&m)) {
        return;
    }
    match e.msg.member().map(MemberName::as_str) {
        Some("Lock") => app.signal(LockRequested),
        Some("Unlock") => app.signal(UnlockRequested),
        _ => {}
    }
}

fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    if e.bus == BusId::System {
        let mut session = app.resource_mut::<Session>();
        session.calls.clear();
        session.holds = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use dbus::{CallError, ErrorName};

    use super::*;

    const PATH: &str = "/org/freedesktop/login1/session/_31";

    #[derive(Default)]
    struct Ended(usize);
    impl Resource for Ended {}

    /// The module on a test system bus (see [`crate::test_bus`]). `_peer`
    /// keeps the bus open.
    struct Harness {
        app: App,
        _peer: UnixStream,
    }

    impl Harness {
        fn new(id: &str) -> Harness {
            let (mut app, peer) = crate::test_bus::system_bus();
            app.init_resource::<Ended>()
                .system(|app, _: &SessionEnded| app.resource_mut::<Ended>().0 += 1);
            install(&mut app, id.into());
            Harness { app, _peer: peer }
        }

        fn ours(&self) -> bool {
            matches!(self.standing(), Standing::Ours { .. })
        }

        /// The call in flight for `pending`.
        fn call(&self, pending: Pending) -> CallId {
            let session = self.app.resource::<Session>();
            let found = session.calls.iter().find(|(_, p)| *p == pending);
            found.map(|(c, _)| *c).unwrap()
        }

        fn has_call(&self, pending: Pending) -> bool {
            let session = self.app.resource::<Session>();
            session.calls.iter().any(|(_, p)| *p == pending)
        }

        fn reply<B: dbus::Body>(&mut self, pending: Pending, body: &B) {
            let call = self.call(pending);
            self.app.signal(MethodReply {
                call,
                result: Ok(Message::method_return(call.serial).with_body(body)),
            });
            self.app.flush();
        }

        fn get_session(&mut self) {
            let path = ObjectPath::new(PATH).unwrap();
            self.reply(Pending::GetSession, &(path,));
        }

        fn check(&mut self, props: PropMap) {
            self.get_session();
            self.reply(Pending::Check, &(props,));
        }

        fn removed(&mut self, path: &str) {
            let m = self.app.resource::<Session>().removed_match;
            self.app.signal(SignalReceived {
                bus: BusId::System,
                matches: [m].into_iter().collect(),
                msg: Message::signal(MANAGER_PATH, MANAGER, "SessionRemoved")
                    .with_body(&("1", ObjectPath::new(path).unwrap())),
            });
            self.app.flush();
        }

        fn standing(&self) -> Standing {
            self.app.resource::<Session>().standing.clone()
        }

        fn ended(&self) -> usize {
            self.app.resource::<Ended>().0
        }
    }

    fn props(uid: u32, kind: &str, state: &str) -> PropMap {
        let user = Value::Struct(vec![
            Value::U32(uid),
            Value::ObjectPath(ObjectPath::new("/org/freedesktop/login1/user/_1000").unwrap()),
        ]);
        [
            ("User".to_string(), user),
            ("Type".to_string(), Value::Str(kind.into())),
            ("State".to_string(), Value::Str(state.into())),
        ]
        .into_iter()
        .collect()
    }

    fn uid() -> u32 {
        std::fs::metadata("/proc/self").unwrap().uid()
    }

    #[test]
    fn only_this_users_open_wayland_session_is_ours() {
        let to_1000 = |uid, kind, state| check(&props(uid, kind, state), 1000);
        assert_eq!(to_1000(1000, "wayland", "active"), Ok(()));
        assert_eq!(to_1000(1000, "wayland", "online"), Ok(()));
        assert_eq!(to_1000(1000, "wayland", "closing"), Err(Standing::Closing));
        let foreign = |s| matches!(s, Err(Standing::Foreign(_)));
        assert!(foreign(to_1000(1001, "wayland", "active")));
        assert!(foreign(to_1000(1000, "tty", "active")));
        assert!(foreign(check(&PropMap::new(), 1000)));
    }

    #[test]
    fn hints_wait_for_the_check_and_go_to_our_session() {
        let mut h = Harness::new("1");
        h.app.signal(SessionLockChanged(true));
        h.app.flush();
        h.get_session();
        let hinted = |h: &Harness| h.has_call(Pending::Hint("SetLockedHint"));
        assert!(!hinted(&h), "not checked yet");
        h.reply(Pending::Check, &(props(uid(), "wayland", "active"),));
        assert!(h.ours());
        assert!(h.has_call(Pending::Hint("SetLockedHint")));
    }

    #[test]
    fn a_session_not_ours_is_left_alone() {
        let mut h = Harness::new("1");
        h.app.signal(SessionLockChanged(true));
        h.app.flush();
        h.check(props(uid(), "tty", "active"));
        assert!(matches!(h.standing(), Standing::Foreign(_)));
        assert!(!h.has_call(Pending::Hint("SetLockedHint")));
        h.removed(PATH);
        assert_eq!(h.ended(), 0, "its end is not ours");
    }

    #[test]
    fn our_session_removed_ends_once() {
        let mut h = Harness::new("1");
        h.check(props(uid(), "wayland", "active"));
        h.removed("/org/freedesktop/login1/session/_32");
        assert_eq!(h.ended(), 0, "another session");
        h.removed(PATH);
        assert_eq!(h.ended(), 1);
        assert_eq!(h.standing(), Standing::Gone);
        h.app.signal(SetIdleHint(true));
        h.app.flush();
        assert!(!h.has_call(Pending::Hint("SetIdleHint")), "gone");
        h.removed(PATH);
        assert_eq!(h.ended(), 1);
    }

    #[test]
    fn a_closing_session_is_not_used_but_its_end_is_ours() {
        let mut h = Harness::new("1");
        h.check(props(uid(), "wayland", "closing"));
        assert_eq!(h.standing(), Standing::Closing);
        h.removed(PATH);
        assert_eq!(h.ended(), 1);
    }

    #[test]
    fn removed_while_checking_stays_ended() {
        let mut h = Harness::new("1");
        h.get_session();
        h.removed(PATH);
        assert_eq!(h.ended(), 1);
        h.reply(Pending::Check, &(props(uid(), "wayland", "active"),));
        assert_eq!(h.standing(), Standing::Gone);
    }

    fn no_such_session(h: &mut Harness) {
        let call = h.call(Pending::GetSession);
        h.app.signal(MethodReply {
            call,
            result: Err(CallError::Remote {
                name: ErrorName::new(NO_SUCH_SESSION).unwrap(),
                message: "No session '1' known".into(),
            }),
        });
        h.app.flush();
    }

    #[test]
    fn a_named_session_already_gone_ends() {
        let mut h = Harness::new("1");
        no_such_session(&mut h);
        assert_eq!(h.ended(), 1);
    }

    #[test]
    fn no_display_session_is_nothing_to_end() {
        let mut h = Harness::new("auto");
        no_such_session(&mut h);
        assert_eq!(h.ended(), 0);
    }
}
