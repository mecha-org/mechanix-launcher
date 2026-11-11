//! The power key: a press while the screen is off turns it on; a press
//! and release while it is on locks it, blanks it and suspends; held for
//! [`LONG_PRESS`] while it is on, it opens the power menu instead.
//!
//! - The key (`XF86PowerOff`) is grabbed through [`services::keys`].
//!   Once the compositor confirms the grab, [`TakePowerKey`] has logind
//!   stop handling the key itself; without the grab, logind keeps it.
//! - With the screen off the compositor passes on nothing but grabbed keys
//!   (see [`screen`]), so the press is what turns it on.
//! - Sleep waits for the release: a key let go while the system is going
//!   to sleep aborts it, and the release never arrives.
//! - The compositor repeats a held grabbed key; only the first press until
//!   its release counts. A release without a press (the wake press's,
//!   whose press the sleeping system swallowed) does nothing.
//! - The power menu is the lock screen's own while locked, since nothing
//!   else shows then; otherwise it is [`power_menu`]'s window. Locking
//!   closes that one.

use std::time::Duration;

use mecha_wayland::prelude::*;
use power_menu::{ShowPowerMenu, hide_power_menu};
use services::{Key, KeyAction, Suspend, TakePowerKey, grab_key};

use crate::lock::{self, LockChanged, LockNow};
use crate::{screen, view};

const POWER: &str = "XF86PowerOff";

/// Held this long, the key opens the power menu.
const LONG_PRESS: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Press {
    /// The press in progress, until its release.
    pressed: Option<Pressed>,
    /// Its [`LONG_PRESS`] timeout, while it holds the screen on.
    hold: Option<Token>,
    /// Every timeout not yet finished: one for each press, let go or not.
    timeouts: Vec<Token>,
}
impl Resource for Press {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pressed {
    /// It turned the screen on; its release does nothing more.
    Woke,
    /// The screen was on: its release sleeps.
    Sleep,
    /// Held long enough to open the power menu; its release does nothing.
    Menu,
}

pub(crate) fn install(app: &mut App) {
    if !grab_key(app, POWER) {
        tracing::warn!("no phosh_private: logind keeps the power key");
        return;
    }
    app.init_resource::<Press>()
        .system(on_key)
        .system(on_timeout)
        .system(|app: &mut App, e: &LockChanged| {
            if e.0 {
                hide_power_menu(app);
            }
        });
}

fn on_key(app: &mut App, e: &Key) {
    if e.accelerator != POWER {
        return;
    }
    match e.action {
        KeyAction::Grabbed => app.signal(TakePowerKey),
        KeyAction::Failed(error) => {
            tracing::warn!("grabbing the power key failed ({error}): logind keeps it");
        }
        KeyAction::Pressed { .. } => {
            if app.resource::<Press>().pressed.is_some() {
                // A repeat.
                return;
            }
            if screen::is_off(app) {
                screen::turn_on(app);
                app.resource_mut::<Press>().pressed = Some(Pressed::Woke);
                return;
            }
            let (mut press, mut ring) = app.query::<(ResMut<Press>, ResMut<Ring>)>();
            let hold = ring.timeout(LONG_PRESS);
            press.pressed = Some(Pressed::Sleep);
            press.hold = Some(hold);
            press.timeouts.push(hold);
        }
        KeyAction::Released { .. } => {
            let pressed = {
                let mut press = app.resource_mut::<Press>();
                press.hold = None;
                press.pressed.take()
            };
            if pressed == Some(Pressed::Sleep) {
                sleep(app);
            }
        }
    }
}

/// Lock, turn the screen off and sleep: the key's press, and idle's.
pub(crate) fn sleep(app: &mut App) {
    app.signal(LockNow);
    screen::turn_off(app);
    suspend(app);
}

/// Ask logind to suspend, unless [`LockSettings::suspend`] is off. logind
/// waits for the lock (the sleep delay inhibitor).
///
/// [`LockSettings::suspend`]: crate::LockSettings::suspend
pub(crate) fn suspend(app: &mut App) {
    if lock::settings(app).suspend {
        app.signal(Suspend);
    }
}

/// A timeout is up: if its press still holds the key, open the power menu.
fn on_timeout(app: &mut App, e: &IoEvent) {
    {
        let (mut press, mut ring) = app.query::<(ResMut<Press>, ResMut<Ring>)>();
        let Some(i) = press.timeouts.iter().position(|t| *t == e.token) else {
            return;
        };
        press.timeouts.swap_remove(i);
        ring.finish(e.token);
        if press.hold != Some(e.token) || press.pressed != Some(Pressed::Sleep) {
            return;
        }
        press.hold = None;
        press.pressed = Some(Pressed::Menu);
    }
    if lock::is_locked(app) {
        view::open_power_menu(app);
    } else {
        app.signal(ShowPowerMenu);
    }
}
