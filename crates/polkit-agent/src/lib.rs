//! The session's polkit authentication agent, in the launcher's process.
//!
//! polkitd asks the agent registered for a session to authenticate an
//! admin whenever a program in it does something that needs one. This
//! crate holds the whole agent: the D-Bus side ([`agent`]), its
//! registration with polkitd ([`register`]), the helper client that
//! authenticates through polkit's helper ([`helper`]), and later the
//! dialog.

mod agent;
mod helper;
mod register;

use mecha_wayland::prelude::*;

pub use agent::{AGENT_PATH, AuthCancelled, AuthFinished, AuthRequested, Identity, Request};
pub use helper::{
    AuthAnswer, AuthCancel, AuthMessage, AuthPrompt, AuthRetry, AuthUser, MAX_FAILURES,
};

/// The polkit agent. Installs after `RingModule` and `DBusModule` with the
/// system bus.
pub struct PolkitAgentModule;

impl Module for PolkitAgentModule {
    fn install(self, app: &mut App) {
        if !agent::install(app, BusId::System) {
            return;
        }
        let session = std::env::var("XDG_SESSION_ID")
            .ok()
            .filter(|id| !id.is_empty());
        helper::install(app, BusId::System, helper::HELPER_SOCKET.into());
        register::install(app, BusId::System, session.as_deref());
    }
}
