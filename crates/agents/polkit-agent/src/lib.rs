//! The session's polkit authentication agent, in the launcher's process.
//!
//! polkitd asks the agent registered for a session to authenticate an
//! admin whenever a program in it does something that needs one. This
//! crate holds the whole agent:
//!
//! - `signals`: what the parts below say to each other;
//! - `bus`: the exported agent, and its registration with polkitd;
//! - `helper`: the client that authenticates through polkit's helper;
//! - `dialog`: the password sheet.

mod bus;
mod dialog;
mod helper;
mod signals;

use mecha_wayland::prelude::*;

pub use bus::agent::AGENT_PATH;
pub use helper::{LOCKOUT, MAX_FAILURES};
pub use signals::{
    AuthAnswer, AuthCancel, AuthCancelled, AuthFinished, AuthLocked, AuthMessage, AuthPrompt,
    AuthRequested, AuthRetry, AuthUser, Identity, Request,
};

/// The polkit agent. Installs after `RingModule`, `DBusModule` with the
/// system bus, the window and presentation modules, the theme, the
/// `Atlas` and the `FontBook`; and before the `KeyboardModule`, so the
/// on-screen keyboard shows over the dialog.
pub struct PolkitAgentModule;

impl Module for PolkitAgentModule {
    fn install(self, app: &mut App) {
        if !bus::agent::install(app, BusId::System) {
            return;
        }
        let session = std::env::var("XDG_SESSION_ID")
            .ok()
            .filter(|id| !id.is_empty());
        helper::install(app, BusId::System, helper::HELPER_SOCKET.into());
        dialog::install(app);
        bus::register::install(app, BusId::System, session.as_deref());
    }
}
