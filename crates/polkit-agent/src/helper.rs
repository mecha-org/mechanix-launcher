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
use nix::unistd::{Uid, User};
use zeroize::Zeroize;

use crate::agent::{AuthCancelled, AuthFinished, AuthRequested, Identity};

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

/// The longest answer the helper reads whole (512 bytes with newline and NUL).
const MAX_ANSWER: usize = 510;

/// Said when an answer can't be sent.
pub(crate) const UNSENDABLE: &str = "That answer is too long, or has a line break in it";

/// Whether the helper can take `answer` as one line.
pub(crate) fn sendable(answer: &[u8]) -> bool {
    answer.len() <= MAX_ANSWER && !answer.contains(&b'\n') && !answer.contains(&0)
}

/// The longest line taken from the helper.
const MAX_LINE: usize = 16 * 1024;

const READ_SIZE: usize = 4096;

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Prompt { text: String, echo: bool },
    Info(String),
    Error(String),
    Success,
    Failure,
}

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
    over: bool,
}

impl Conn {
    fn holds(&self, token: Token) -> bool {
        self.reading == Some(token) || self.sending.contains(&token)
    }

    fn gone(&self) -> bool {
        self.reading.is_none() && self.sending.is_empty()
    }

    /// End the attempt; the helper sees end of stream.
    fn end(&mut self) {
        self.over = true;
        self.prompt_open = false;
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

pub(crate) struct Helper {
    bus: BusId,
    socket: PathBuf,
    session: Option<Session>,
    /// Connections not yet gone; at most one is current.
    conns: Vec<Conn>,
    lockout: Duration,
    /// Lockouts' ring timeouts, by cookie. One whose session ended is
    /// ignored when it fires.
    locks: Vec<(Token, String)>,
}
impl Resource for Helper {}

impl Helper {
    fn current(&mut self) -> Option<&mut Conn> {
        self.conns.iter_mut().find(|c| !c.over)
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
        over: false,
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
    let mut broke = false;
    {
        let (mut helper, mut ring) = app.query::<(ResMut<Helper>, ResMut<Ring>)>();
        let Some(c) = helper.conns.iter_mut().find(|c| c.holds(e.token)) else {
            return;
        };
        let done = ring.finish(e.token);
        if c.reading == Some(e.token) {
            c.reading = None;
            let mut buf = done.map(|d| d.buf).unwrap_or_default();
            if c.over {
                // Ended: ignore.
            } else if e.result <= 0 {
                c.end();
                broke = true;
            } else {
                c.pending.extend_from_slice(&buf);
                while let Some(i) = c.pending.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = c.pending.drain(..=i).collect();
                    line.pop();
                    lines.push(line);
                }
                if c.pending.len() > MAX_LINE {
                    c.end();
                    broke = true;
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
            if e.result < 0 && !c.over {
                c.end();
                broke = true;
            }
        }
        helper.conns.retain(|c| !c.gone());
    }
    for line in lines {
        if !on_line(app, &line) {
            return;
        }
    }
    if broke {
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

fn parse_line(raw: &[u8]) -> Option<Line> {
    let line = unescape(raw);
    let text = |rest: &[u8]| String::from_utf8_lossy(rest).into_owned();
    if let Some(rest) = line.strip_prefix(b"PAM_PROMPT_ECHO_OFF ") {
        return Some(Line::Prompt {
            text: text(rest),
            echo: false,
        });
    }
    if let Some(rest) = line.strip_prefix(b"PAM_PROMPT_ECHO_ON ") {
        return Some(Line::Prompt {
            text: text(rest),
            echo: true,
        });
    }
    if let Some(rest) = line.strip_prefix(b"PAM_ERROR_MSG ") {
        return Some(Line::Error(text(rest)));
    }
    if let Some(rest) = line.strip_prefix(b"PAM_TEXT_INFO ") {
        return Some(Line::Info(text(rest)));
    }
    match line.as_slice() {
        b"SUCCESS" => Some(Line::Success),
        b"FAILURE" => Some(Line::Failure),
        _ => None,
    }
}

/// Undo `g_strescape`, as `g_strcompress` does.
fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        i += 1;
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let Some(&c) = s.get(i) else {
            break;
        };
        i += 1;
        let byte = match c {
            b'0'..=b'7' => {
                let mut value = u32::from(c - b'0');
                for _ in 0..2 {
                    match s.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                // As g_strcompress: past `\377`, the low byte.
                (value & 0xff) as u8
            }
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            other => other,
        };
        out.push(byte);
    }
    out
}

/// Who to authenticate as: `own`, else root, else the first listed.
fn choose_uid(identities: &[Identity], own: u32) -> Option<u32> {
    let users: Vec<u32> = identities
        .iter()
        .filter_map(|i| match i {
            Identity::User(uid) => Some(*uid),
            _ => None,
        })
        .collect();
    [own, 0]
        .into_iter()
        .find(|u| users.contains(u))
        .or_else(|| users.first().copied())
}

fn own_uid() -> u32 {
    Uid::current().as_raw()
}

fn user_name(uid: u32) -> Option<String> {
    User::from_uid(Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|u| u.name)
        .filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests;
