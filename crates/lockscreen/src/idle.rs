//! Idle: after a while without input, the screen goes off and the system
//! sleeps. Its own module, [`IdleModule`]: leave it out and another idle
//! daemon (swayidle, hypridle) can take over, through logind's lock and
//! the screen's power control as this does.
//!
//! - One ext-idle-notify notification at the timeout for the lock state:
//!   [`IdleSettings::blank_after`], or [`IdleSettings::blank_after_locked`]
//!   while locked. A lock or unlock remakes it at the other timeout; one
//!   that comes while idle (the lock on blank) waits for the next
//!   activity, so the idle period and its hint carry on.
//! - `idled` turns the screen off ([`crate::screen`]), raises the session's
//!   `IdleHint`, locks if [`IdleSettings::lock_on_blank`], and asks logind
//!   to suspend, as [`LockSettings::suspend`](crate::LockSettings::suspend)
//!   says. A suspend that is refused (inhibited) leaves the screen
//!   off, the system up.
//! - The screen coming back on, however it does, starts the timer over:
//!   the input in the dark that led there was not activity.
//! - While anyone keeps the screen awake ([`KeepAwake`]: the user from quick
//!   settings, or an app), there is no notification: nothing goes off, and
//!   the session never reports idle. With the screen off, that waits until
//!   it is next on. An idle daemon in this module's place doesn't see
//!   [`KeepAwake`].
//!
//! Without ext-idle-notify, nothing goes off on its own.

use std::time::Duration;

use mecha_wayland::prelude::*;
use services::{KeepAwake, SetIdleHint};

use crate::lock::{self, LockChanged};
use crate::screen::{self, ScreenPowered};

/// When the screen goes off on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct IdleSettings {
    /// Turn the screen off and sleep after this long without input; `None`
    /// never.
    pub blank_after: Option<Duration>,
    /// [`IdleSettings::blank_after`], while locked.
    pub blank_after_locked: Option<Duration>,
    /// Lock when going idle, before the sleep.
    pub lock_on_blank: bool,
}

impl Default for IdleSettings {
    fn default() -> Self {
        IdleSettings {
            blank_after: Some(Duration::from_secs(30)),
            blank_after_locked: Some(Duration::from_secs(10)),
            lock_on_blank: true,
        }
    }
}

/// The shell's idle policy. Installs after the
/// [`LockscreenModule`](crate::LockscreenModule).
pub struct IdleModule {
    pub settings: IdleSettings,
}

struct Idle {
    notifier: Option<ExtIdleNotifierV1>,
    seat: WlSeat,
    settings: IdleSettings,
    is_locked: bool,
    /// Idle after the lock state's timeout.
    notification: Option<ExtIdleNotificationV1>,
    is_idle: bool,
    /// The lock state changed while idle: remake the notification at the
    /// next activity.
    stale: bool,
}
impl Resource for Idle {}

impl Module for IdleModule {
    fn install(self, app: &mut App) {
        let notifier = {
            let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
            globals.bind_if_offered::<ExtIdleNotifierV1>(&mut wl, 1)
        };
        if notifier.is_none() {
            tracing::warn!("no ext_idle_notifier_v1: the screen never goes off on its own");
        }
        let seat = *app.resource::<WlSeat>();
        let is_locked = lock::is_locked(app);
        app.insert_resource(Idle {
            notifier,
            seat,
            settings: self.settings,
            is_locked,
            notification: None,
            is_idle: false,
            stale: false,
        });
        // A launcher that died idle left the hint up.
        app.signal(SetIdleHint(false));
        renotify(app);
        app.system(on_idle)
            .system(on_lock_changed)
            .system(on_keep_awake)
            .system(|app, e: &ScreenPowered| {
                if e.0 {
                    renotify(app);
                }
            });
    }
}

/// Make the notification for the current lock state, dropping the old.
/// The session is active again: logind counts an idle period from when
/// the hint went up, so it must come down whenever the timer starts over.
fn renotify(app: &mut App) {
    let was_idle = {
        let (mut idle, mut wl) = app.query::<(ResMut<Idle>, ResMut<Wayland>)>();
        if let Some(n) = idle.notification.take() {
            n.destroy(&mut wl);
        }
        std::mem::take(&mut idle.is_idle)
    };
    if was_idle {
        app.signal(SetIdleHint(false));
    }
    let kept_awake = app.has_resource::<KeepAwake>() && app.resource::<KeepAwake>().any();
    let (mut idle, mut wl) = app.query::<(ResMut<Idle>, ResMut<Wayland>)>();
    idle.stale = false;
    let timeout = if kept_awake {
        None
    } else if idle.is_locked {
        idle.settings.blank_after_locked
    } else {
        idle.settings.blank_after
    };
    let (Some(notifier), Some(timeout)) = (idle.notifier, timeout) else {
        return;
    };
    let ms = u32::try_from(timeout.as_millis())
        .unwrap_or(u32::MAX)
        .max(1);
    idle.notification = Some(notifier.get_idle_notification(&mut wl, ms, idle.seat));
}

/// Who keeps the screen awake changed. With the screen off, nothing changes
/// until it is next on: that starts the timer over anyway.
fn on_keep_awake(app: &mut App, _: &OnChanged<KeepAwake>) {
    if !screen::is_off(app) {
        renotify(app);
    }
}

fn on_idle(app: &mut App, e: &ExtIdleNotificationV1Event) {
    let (n, idled) = match e {
        ExtIdleNotificationV1Event::Idled { idle_notification } => (*idle_notification, true),
        ExtIdleNotificationV1Event::Resumed { idle_notification } => (*idle_notification, false),
    };
    if app.resource::<Idle>().notification != Some(n) {
        return;
    }
    app.resource_mut::<Idle>().is_idle = idled;
    app.signal(SetIdleHint(idled));
    if !idled {
        if app.resource::<Idle>().stale {
            renotify(app);
        }
        return;
    }
    if app.resource::<Idle>().settings.lock_on_blank {
        crate::power::sleep(app);
    } else {
        screen::turn_off(app);
        crate::power::suspend(app);
    }
}

fn on_lock_changed(app: &mut App, e: &LockChanged) {
    let idle_now = {
        let mut idle = app.resource_mut::<Idle>();
        idle.is_locked = e.0;
        // Locked for being idle: the period goes on. The new timeout
        // applies from the next activity.
        idle.stale = idle.is_idle;
        idle.is_idle
    };
    if !idle_now {
        renotify(app);
    }
}
