//! The lock screen, in the launcher's process: an `ext_session_lock_v1`
//! lock with a surface on every output, unlocked through PAM. The lock
//! screen itself, input and all, is on the first output only; the others
//! stay black.
//!
//! # Locking
//!
//! [`LockNow`] locks. So do logind's `Lock` for the session
//! ([`services::LockRequested`]: `loginctl lock-session`,
//! `HandlePowerKey=lock`), the system going to sleep
//! ([`services::PrepareForSleep`], which waits for the lock before letting
//! the system sleep), going idle when [`IdleSettings::lock_on_blank`]
//! says so, and the start of the launcher when
//! [`LockSettings::lock_at_start`] says so; none of them when
//! [`LockSettings::enabled`] is off. logind's `Unlock` unlocks. The
//! session's `LockedHint` follows the compositor's answer.
//!
//! # Unlocking
//!
//! Each [`Method`] is its own PAM service, run in a worker process (see
//! [`auth`]) as soon as the lock goes up, so its first prompt is ready.
//! The UI is the method's: a numpad for [`Method::Pin`], the keyboard for
//! [`Method::Password`]. What PAM says (prompts, faillock's lockout text,
//! errors) is shown as it says it. A method whose stack reports it
//! unavailable, or fails without asking anything, is skipped until the
//! next lock; with no method left, nothing unlocks but logind. No methods
//! configured at all is a swipe up to unlock.
//!
//! The PIN and the password are typed, into one input, so one of them runs
//! at a time and the user switches. A method that isn't typed (one that
//! waits on a fingerprint or a security key, see [`Method::is_typed`])
//! runs alongside from the lock on, in its own worker, with its own
//! stack; whichever succeeds first unlocks.
//!
//! The password's stack includes `system-auth`, so what the system asks of
//! every login it asks here too. The PIN's doesn't: a factor added to
//! `system-auth` doesn't guard it (see `resources/pam.d/mechanix-pin`).
//!
//! # The power key
//!
//! Grabbed from the compositor, and taken from logind once grabbed (see
//! [`power`]): a press while the screen is off turns it on; a press and
//! release while it is on locks, turns the screen off and suspends;
//! holding it while it is on opens the power menu, in the lock screen
//! while locked. The screen's power is [`screen`]'s, which follows what
//! the compositor reports, whoever turned it off.
//!
//! # Idle
//!
//! [`IdleModule`], a module of its own: after
//! [`IdleSettings::blank_after`] idle (ext-idle-notify), the screen goes
//! off, locks if [`IdleSettings::lock_on_blank`] says so, and the system is
//! asked to sleep (see [`idle`]). logind's `IdleAction` is left alone.
//! Without the module, another idle daemon can lock through logind and
//! turn the screen off through wlr-output-power-management; the power key
//! still turns it back on.
//!
//! The settings are plain structs for now. Which methods are on belongs
//! in the system configuration namespace (written only through polkit),
//! the timeouts in the desktop namespace.

mod auth;
mod idle;
mod lock;
mod power;
mod screen;
mod view;

use mecha_wayland::prelude::*;

pub use auth::Method;
pub use idle::{IdleModule, IdleSettings};
pub use lock::{LockChanged, LockNow};

/// How the lock screen behaves.
#[derive(Debug, Clone, PartialEq)]
pub struct LockSettings {
    /// Whether the session locks at all. Off, nothing locks it: not
    /// [`LockNow`], logind, sleep or idle.
    pub enabled: bool,
    /// The ways to unlock, in order: the first available is offered
    /// first. Empty is tap to unlock.
    pub methods: Vec<Method>,
    /// How many digits the PIN has: the PIN pad takes that many and submits
    /// on the last. It must match the PIN that was set.
    pub pin_length: PinLength,
    /// Lock when the launcher starts.
    pub lock_at_start: bool,
    /// Suspend when the power key or idle puts the device to sleep. Off
    /// (the default, as not every device resumes), it locks and turns the
    /// screen off, and the system stays up.
    pub suspend: bool,
}

impl Default for LockSettings {
    fn default() -> Self {
        LockSettings {
            enabled: true,
            methods: vec![Method::Pin, Method::Password],
            pin_length: PinLength::Four,
            lock_at_start: true,
            suspend: false,
        }
    }
}

/// How many digits a PIN has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinLength {
    #[default]
    Four,
    Six,
}

impl PinLength {
    pub fn digits(self) -> usize {
        match self {
            PinLength::Four => 4,
            PinLength::Six => 6,
        }
    }
}

/// The lock screen. Installs after `PresentationModule`, the
/// `KeyboardModule`, `TimerModule`, `DBusModule` with the system bus and
/// `services::SessionModule`, and after the theme and the `FontBook`.
pub struct LockscreenModule {
    pub settings: LockSettings,
}

impl Module for LockscreenModule {
    fn install(self, app: &mut App) {
        app.add_module(OutputsModule)
            .add_module(services::PowerModule);
        auth::install(app);
        view::install(app);
        screen::install(app);
        lock::install(app, self.settings);
        power::install(app);
    }
}

/// Run the auth worker if this process was started as one. Call first
/// thing in `main`: it does not return when it was.
pub fn run_worker_if_asked() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(auth::WORKER_ARG) {
        return;
    }
    // Only the services the lock screen's own methods name.
    let service = args
        .next()
        .and_then(|s| Method::ALL.into_iter().find(|m| m.service() == s));
    let Some(method) = service else {
        std::process::exit(2);
    };
    auth::worker::main(method.service());
}
