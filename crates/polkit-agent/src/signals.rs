//! The signals between the agent's parts, and the request they carry.
//!
//! The bus side raises [`AuthRequested`] and [`AuthCancelled`] and takes
//! [`AuthFinished`]. The helper client takes the request, reports PAM's
//! side with [`AuthUser`], [`AuthPrompt`], [`AuthMessage`], [`AuthRetry`]
//! and [`AuthLocked`], and takes the dialog's [`AuthAnswer`] and
//! [`AuthCancel`].

use std::collections::HashMap;
use std::time::Duration;

use mecha_wayland::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Group(u32),
    /// Unknown kind, or missing its id.
    Other(String),
}

/// One `BeginAuthentication`.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<Identity>,
}

impl std::fmt::Debug for Request {
    /// Leaves out the cookie.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("action_id", &self.action_id)
            .field("message", &self.message)
            .field("icon_name", &self.icon_name)
            .field("details", &self.details)
            .field("identities", &self.identities)
            .finish_non_exhaustive()
    }
}

/// The active request: answer it with [`AuthFinished`].
#[derive(Debug, Clone)]
pub struct AuthRequested(pub Request);
impl Signal for AuthRequested {}

/// polkitd cancelled the active request.
#[derive(Debug, Clone)]
pub struct AuthCancelled {
    pub cookie: String,
}
impl Signal for AuthCancelled {}

/// Answer the active request with this cookie.
#[derive(Debug, Clone)]
pub struct AuthFinished {
    pub cookie: String,
    pub result: Result<(), String>,
}
impl Signal for AuthFinished {}

/// The request with this cookie is authenticated as `user`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub cookie: String,
    pub user: String,
}
impl Signal for AuthUser {}

/// PAM asks a question: answer it with [`AuthAnswer`].
#[derive(Debug, Clone)]
pub struct AuthPrompt {
    pub cookie: String,
    /// PAM's words, e.g. `Password: `.
    pub text: String,
    /// Show what is typed. Off for a password.
    pub echo: bool,
}
impl Signal for AuthPrompt {}

/// PAM says something to show: an error, or information.
#[derive(Debug, Clone)]
pub struct AuthMessage {
    pub cookie: String,
    pub text: String,
    pub error: bool,
}
impl Signal for AuthMessage {}

/// Too many wrong answers: the next attempt starts after `wait`.
#[derive(Debug, Clone)]
pub struct AuthLocked {
    pub cookie: String,
    pub wait: Duration,
}
impl Signal for AuthLocked {}

/// The answer was wrong; a new attempt starts.
#[derive(Debug, Clone)]
pub struct AuthRetry {
    pub cookie: String,
}
impl Signal for AuthRetry {}

/// Answer the open prompt. A too long answer, or one with a line break,
/// is refused with an [`AuthMessage`].
#[derive(Debug)]
pub struct AuthAnswer {
    pub cookie: String,
    pub answer: Secret,
}
impl Signal for AuthAnswer {}

/// The user gave up on the request with this cookie.
#[derive(Debug, Clone)]
pub struct AuthCancel {
    pub cookie: String,
}
impl Signal for AuthCancel {}
