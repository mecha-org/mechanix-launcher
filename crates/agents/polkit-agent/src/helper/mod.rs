//! The helper client: authenticates the active request through polkit's
//! socket-activated helper. `polkit-agent-helper-1` runs PAM as root; the
//! agent only relays PAM's questions and the user's answers.
//!
//! On [`HELPER_SOCKET`] the agent sends the user and the cookie, a line
//! each. The helper replies with prompts, messages, and last `SUCCESS` or
//! `FAILURE`. One connection is one attempt.
//!
//! The cookie and answers are never logged, and their buffers are wiped.

use std::net::Shutdown;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use mecha_wayland::prelude::*;
use zeroize::Zeroize;

use crate::signals::{
    AuthAnswer, AuthCancel, AuthCancelled, AuthFinished, AuthLocked, AuthMessage, AuthPrompt,
    AuthRequested, AuthRetry, AuthUser,
};

mod identity;
pub(crate) mod protocol;

use identity::{choose_uid, own_uid, user_name};
use protocol::{Line, UNSENDABLE, parse_line, sendable};

pub(crate) const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Wrong answers in a row before a lockout.
///
/// The lockout only paces the dialog; it is not a defence against
/// guessing. It is counted per request, in memory: cancelling and asking
/// again starts over, as do restarting the launcher or using another
/// agent (`pkttyagent`). Limiting guesses is PAM's job, `pam_faillock`
/// in polkit's PAM stack, which sees every attempt through the helper.
pub const MAX_FAILURES: u32 = 3;

/// How long a lockout lasts before the next attempt. See [`MAX_FAILURES`].
pub const LOCKOUT: Duration = Duration::from_secs(30);

/// The longest line taken from the helper.
const MAX_LINE: usize = 16 * 1024;

const READ_SIZE: usize = 4096;

struct Session {
    cookie: String,
    user: String,
    failures: u32,
}

/// One connection to the helper: one attempt.
struct Conn {
    stream: UnixStream,
    /// The read in flight, until the helper hangs up.
    reading: Option<Token>,
    /// Sends in flight: their buffers come back to be wiped.
    sending: Vec<Token>,
    pending: Vec<u8>,
    prompt_open: bool,
    /// Ended: kept only until the ring returns its buffers.
    ended: bool,
}

impl Conn {
    fn holds(&self, token: Token) -> bool {
        self.reading == Some(token) || self.sending.contains(&token)
    }

    /// Nothing in flight on the ring: safe to drop.
    fn drained(&self) -> bool {
        self.reading.is_none() && self.sending.is_empty()
    }

    /// End the attempt; the helper sees end of stream.
    fn end(&mut self) {
        self.ended = true;
        self.prompt_open = false;
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

pub(crate) struct Helper {
    bus: BusId,
    socket: PathBuf,
    session: Option<Session>,
    /// Connections not yet drained; at most one is current.
    conns: Vec<Conn>,
    lockout: Duration,
    /// Lockouts' ring timeouts, by cookie. One whose session ended is
    /// ignored when it fires.
    locks: Vec<(Token, String)>,
}
impl Resource for Helper {}

impl Helper {
    fn current(&mut self) -> Option<&mut Conn> {
        self.conns.iter_mut().find(|c| !c.ended)
    }

    fn is_session(&self, cookie: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.cookie == cookie)
    }
}

/// Authenticate requests through the helper at `socket`. Installs after
/// `RingModule`.
pub(crate) fn install(app: &mut App, bus: BusId, socket: PathBuf) {
    if app.has_resource::<Helper>() {
        return;
    }
    app.insert_resource(Helper {
        bus,
        socket,
        session: None,
        conns: Vec::new(),
        lockout: LOCKOUT,
        locks: Vec::new(),
    });
    app.system(on_requested)
        .system(on_answer)
        .system(|app, e: &AuthCancel| {
            if app.resource::<Helper>().is_session(&e.cookie) {
                fail(app, "Dismissed by the user");
            }
        })
        .system(|app, e: &AuthCancelled| {
            if app.resource::<Helper>().is_session(&e.cookie) {
                end_session(app);
            }
        })
        .system(|app, e: &BusDisconnected| {
            if e.bus == app.resource::<Helper>().bus {
                end_session(app);
            }
        })
        .system(on_io);
}

fn on_requested(app: &mut App, e: &AuthRequested) {
    // One request at a time.
    end_session(app);
    let request = &e.0;
    let user = choose_uid(&request.identities, own_uid()).and_then(user_name);
    let Some(user) = user else {
        tracing::error!("polkit: {}: no user to authenticate as", request.action_id);
        app.signal(AuthFinished {
            cookie: request.cookie.clone(),
            result: Err("No user to authenticate as".into()),
        });
        return;
    };
    tracing::info!("polkit: {}: authenticating as {user}", request.action_id);
    app.resource_mut::<Helper>().session = Some(Session {
        cookie: request.cookie.clone(),
        user: user.clone(),
        failures: 0,
    });
    app.signal(AuthUser {
        cookie: request.cookie.clone(),
        user,
    });
    connect(app);
}

/// Start an attempt: connect and send the user and cookie.
fn connect(app: &mut App) {
    let (path, mut hello) = {
        let helper = app.resource::<Helper>();
        let Some(s) = &helper.session else {
            return;
        };
        let mut hello = Vec::with_capacity(s.user.len() + s.cookie.len() + 2);
        for part in [&s.user, &s.cookie] {
            hello.extend_from_slice(part.as_bytes());
            hello.push(b'\n');
        }
        (helper.socket.clone(), hello)
    };
    let stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error) => {
            hello.zeroize();
            tracing::error!(
                "polkit: connecting to {}: {error} (is polkit-agent-helper.socket enabled?)",
                path.display()
            );
            fail(app, "The authentication helper is not available");
            return;
        }
    };
    let (sending, reading) = {
        let mut ring = app.resource_mut::<Ring>();
        (
            ring.sendmsg(stream.as_fd(), hello, Vec::new()),
            ring.recvmsg(stream.as_fd(), Vec::with_capacity(READ_SIZE), 0),
        )
    };
    app.resource_mut::<Helper>().conns.push(Conn {
        stream,
        reading: Some(reading),
        sending: vec![sending],
        pending: Vec::new(),
        prompt_open: false,
        ended: false,
    });
}

/// End the session; returns its cookie.
fn end_session(app: &mut App) -> Option<String> {
    let mut helper = app.resource_mut::<Helper>();
    if let Some(c) = helper.current() {
        c.end();
    }
    helper.session.take().map(|s| s.cookie)
}

/// Drop the session and fail its request with `why`.
fn fail(app: &mut App, why: &str) {
    if let Some(cookie) = end_session(app) {
        app.signal(AuthFinished {
            cookie,
            result: Err(why.into()),
        });
    }
}

fn on_answer(app: &mut App, e: &AuthAnswer) {
    let open = {
        let mut helper = app.resource_mut::<Helper>();
        helper.is_session(&e.cookie) && helper.current().is_some_and(|c| c.prompt_open)
    };
    if !open {
        return;
    }
    let answer = e.answer.as_bytes();
    if !sendable(answer) {
        app.signal(AuthMessage {
            cookie: e.cookie.clone(),
            text: UNSENDABLE.into(),
            error: true,
        });
        return;
    }
    let mut line = Vec::with_capacity(answer.len() + 1);
    line.extend_from_slice(answer);
    line.push(b'\n');
    let (mut helper, mut ring) = app.query::<(ResMut<Helper>, ResMut<Ring>)>();
    let Some(c) = helper.current() else {
        line.zeroize();
        return;
    };
    c.prompt_open = false;
    let token = ring.sendmsg(c.stream.as_fd(), line, Vec::new());
    c.sending.push(token);
}

fn on_io(app: &mut App, e: &IoEvent) {
    if unlocked(app, e.token) {
        return;
    }
    let mut lines = Vec::new();
    // Hung up without a verdict, a send failed, or a line too long.
    let mut no_verdict = false;
    {
        let (mut helper, mut ring) = app.query::<(ResMut<Helper>, ResMut<Ring>)>();
        let Some(c) = helper.conns.iter_mut().find(|c| c.holds(e.token)) else {
            return;
        };
        let done = ring.finish(e.token);
        if c.reading == Some(e.token) {
            c.reading = None;
            let mut buf = done.map(|d| d.buf).unwrap_or_default();
            if c.ended {
                // Ended: ignore.
            } else if e.result <= 0 {
                c.end();
                no_verdict = true;
            } else {
                c.pending.extend_from_slice(&buf);
                while let Some(i) = c.pending.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = c.pending.drain(..=i).collect();
                    line.pop();
                    lines.push(line);
                }
                if c.pending.len() > MAX_LINE {
                    c.end();
                    no_verdict = true;
                } else {
                    buf.clear();
                    c.reading = Some(ring.recvmsg(c.stream.as_fd(), buf, 0));
                }
            }
        } else {
            c.sending.retain(|t| *t != e.token);
            if let Some(mut d) = done {
                d.buf.zeroize();
            }
            if e.result < 0 && !c.ended {
                c.end();
                no_verdict = true;
            }
        }
        helper.conns.retain(|c| !c.drained());
    }
    for line in lines {
        if !on_line(app, &line) {
            return;
        }
    }
    if no_verdict {
        tracing::warn!("polkit: the helper hung up without a verdict");
        fail(app, "The authentication helper failed");
    }
}

/// Handle a line; false once the attempt is over.
fn on_line(app: &mut App, raw: &[u8]) -> bool {
    let Some(cookie) = app
        .resource::<Helper>()
        .session
        .as_ref()
        .map(|s| s.cookie.clone())
    else {
        return false;
    };
    match parse_line(raw) {
        Some(Line::Prompt { text, echo }) => {
            if let Some(c) = app.resource_mut::<Helper>().current() {
                c.prompt_open = true;
            }
            app.signal(AuthPrompt { cookie, text, echo });
        }
        Some(Line::Info(text)) => app.signal(AuthMessage {
            cookie,
            text,
            error: false,
        }),
        Some(Line::Error(text)) => app.signal(AuthMessage {
            cookie,
            text,
            error: true,
        }),
        Some(Line::Success) => {
            end_session(app);
            tracing::info!("polkit: the helper authenticated the request");
            app.signal(AuthFinished {
                cookie,
                result: Ok(()),
            });
            return false;
        }
        Some(Line::Failure) => {
            failed(app, cookie);
            return false;
        }
        None => tracing::warn!("polkit: the helper said something unknown"),
    }
    true
}

/// A lockout's timeout fired: start the next attempt, if its session is
/// still on. True if `token` was a lockout's.
fn unlocked(app: &mut App, token: Token) -> bool {
    let cookie = {
        let (mut helper, mut ring) = app.query::<(ResMut<Helper>, ResMut<Ring>)>();
        let Some(i) = helper.locks.iter().position(|(t, _)| *t == token) else {
            return false;
        };
        ring.finish(token);
        helper.locks.remove(i).1
    };
    if app.resource::<Helper>().is_session(&cookie) {
        tracing::info!("polkit: lockout over");
        connect(app);
    }
    true
}

/// The attempt failed: try again, after a lockout every [`MAX_FAILURES`].
fn failed(app: &mut App, cookie: String) {
    let failures = {
        let mut helper = app.resource_mut::<Helper>();
        if let Some(c) = helper.current() {
            c.end();
        }
        let Some(s) = helper.session.as_mut() else {
            return;
        };
        s.failures += 1;
        s.failures
    };
    tracing::info!("polkit: authentication failed ({failures} of {MAX_FAILURES})");
    if failures >= MAX_FAILURES {
        let wait = {
            let (mut helper, mut ring) = app.query::<(ResMut<Helper>, ResMut<Ring>)>();
            let wait = helper.lockout;
            let token = ring.timeout(wait);
            helper.locks.push((token, cookie.clone()));
            if let Some(s) = helper.session.as_mut() {
                s.failures = 0;
            }
            wait
        };
        tracing::info!("polkit: locked out for {wait:?}");
        app.signal(AuthLocked { cookie, wait });
        return;
    }
    app.signal(AuthRetry { cookie });
    connect(app);
}

#[cfg(test)]
mod tests;
