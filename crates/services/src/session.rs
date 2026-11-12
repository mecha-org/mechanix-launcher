//! The launcher's logind session on the system bus: lock and unlock
//! requests, the locked and idle hints, and a delay inhibitor so the
//! screen locks before the system sleeps.
//!
//! - The session is `XDG_SESSION_ID`'s, which `mechanix-session` exports
//!   to the user manager. The launcher is a user service, outside the
//!   session's scope, so without it the session is logind's `auto`: the
//!   user's display session.
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

use mecha_wayland::prelude::*;

use crate::power::{LOGIND, MANAGER, MANAGER_PATH};

const SESSION: &str = "org.freedesktop.login1.Session";

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
    Inhibit(Inhibitor),
    Hint(&'static str),
}

struct Session {
    /// The session's object path, once `GetSession` answered.
    path: Option<ObjectPath>,
    /// The `Lock`/`Unlock` match on the session's path.
    lock_match: Option<MatchId>,
    sleep_match: MatchId,
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
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "auto".into());
        let (get, sleep_match) = {
            let mut dbus = app.resource_mut::<DBus>();
            let mut bus = dbus.system();
            let get = bus.send(
                MethodCall::new(LOGIND, MANAGER_PATH, MANAGER, "GetSession").body(&(id.as_str(),)),
            );
            let sleep_match = bus.add_match(
                MatchRule::signal()
                    .sender(LOGIND)
                    .path(MANAGER_PATH)
                    .interface(MANAGER)
                    .member("PrepareForSleep"),
            );
            (get, sleep_match)
        };
        app.insert_resource(Session {
            path: None,
            lock_match: None,
            sleep_match,
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
    let call = app.resource_mut::<DBus>().system().send(
        MethodCall::new(LOGIND, MANAGER_PATH, MANAGER, "Inhibit")
            .body(&(kind, "Mechanix", why, mode)),
    );
    app.resource_mut::<Session>()
        .calls
        .push((call, Pending::Inhibit(what)));
}

/// Release `what`: closing its fd releases it.
fn release(app: &mut App, what: Inhibitor) {
    app.resource_mut::<Session>().holds[what as usize] = Hold::Released;
}

fn hint(app: &mut App, method: &'static str, value: bool) {
    let Some(path) = app.resource::<Session>().path.clone() else {
        let mut session = app.resource_mut::<Session>();
        session.queued.retain(|(m, _)| *m != method);
        session.queued.push((method, value));
        return;
    };
    let call = app
        .resource_mut::<DBus>()
        .system()
        .send(MethodCall::new(LOGIND, path, SESSION, method).body(&(value,)));
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
            if let Pending::Inhibit(what) = pending {
                // Asked for again by the next `take`.
                release(app, what);
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
            let m = app.resource_mut::<DBus>().system().add_match(
                MatchRule::signal()
                    .sender(LOGIND)
                    .path(path.clone())
                    .interface(SESSION),
            );
            let queued = {
                let mut session = app.resource_mut::<Session>();
                session.path = Some(path);
                session.lock_match = Some(m);
                std::mem::take(&mut session.queued)
            };
            for (method, value) in queued {
                hint(app, method, value);
            }
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

fn on_signal(app: &mut App, e: &SignalReceived) {
    let (lock_match, sleep_match) = {
        let session = app.resource::<Session>();
        (session.lock_match, session.sleep_match)
    };
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
