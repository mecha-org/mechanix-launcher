//! The helper client: authenticates the active request through polkit's
//! socket-activated helper, the way libpolkit-agent's `PolkitAgentSession`
//! does.
//!
//! The agent never checks a password. `polkit-agent-helper-1` runs PAM as
//! root and tells polkitd itself whether the request's cookie was
//! authenticated; the agent only carries PAM's questions to the user and
//! the answers back.
//!
//! # The helper
//!
//! systemd's `polkit-agent-helper.socket` listens on [`HELPER_SOCKET`],
//! one helper per connection (`Accept=yes`), the connection its stdin and
//! stdout. It takes the agent's uid and process from the socket
//! (`SO_PEERCRED`, `SO_PEERPIDFD`: Linux 6.5 and later), so the agent
//! connects itself. On the connection, a line each:
//!
//! - The agent sends the user to authenticate as, then the cookie.
//! - The helper sends `PAM_PROMPT_ECHO_OFF <text>` or
//!   `PAM_PROMPT_ECHO_ON <text>`, a question answered with one line;
//!   `PAM_ERROR_MSG <text>` or `PAM_TEXT_INFO <text>`, to show; and last
//!   `SUCCESS` or `FAILURE`, then hangs up. Its lines are escaped as
//!   `g_strescape` does (every byte outside printable ASCII as octal), and
//!   unescaped here.
//!
//! One connection is one PAM attempt. Closing it ends the attempt: the
//! helper reads the end of the stream as no answer, and fails.
//!
//! # A request
//!
//! - On [`AuthRequested`], the user is chosen from the request's
//!   identities: the launcher's own user if listed, else root, else the
//!   first.
//!   (polkitd has already expanded groups into their users.)
//!   [`AuthUser`] names it; with none, the request fails at once.
//! - What the helper says goes out as [`AuthPrompt`] and [`AuthMessage`];
//!   [`AuthAnswer`] answers the open prompt.
//! - `SUCCESS` answers the request with [`AuthFinished`]. `FAILURE` is
//!   [`AuthRetry`] and a new attempt, up to [`MAX_FAILURES`], after which
//!   the request fails. A helper that hangs up or breaks the protocol
//!   fails it too.
//! - [`AuthCancel`], the user's, ends the attempt and fails the request.
//!   [`AuthCancelled`], polkitd's, ends the attempt and says nothing more:
//!   the request is gone already.
//!
//! The cookie and the answers are never logged; buffers that held them
//! are wiped once sent.
//!
//! # References
//!
//! polkit's documentation covers agents and their D-Bus interface, not
//! the helper's protocol: that is in its source, as of polkit 127 (the
//! device's Fedora 44 and this code follow it).
//!
//! - polkit(8), the architecture: agents, the helper, polkitd:
//!   <https://www.freedesktop.org/software/polkit/docs/latest/polkit.8.html>
//! - The `AuthenticationAgent` D-Bus interface:
//!   <https://www.freedesktop.org/software/polkit/docs/latest/eggdbus-interface-org.freedesktop.PolicyKit1.AuthenticationAgent.html>
//! - `PolkitAgentSession`, libpolkit-agent's helper client, what this
//!   module does in its place:
//!   <https://www.freedesktop.org/software/polkit/docs/latest/PolkitAgentSession.html>
//! - Its source: connecting to the socket, sending the user and the cookie
//!   (L609-L676), reading and unescaping lines (L430-L500):
//!   <https://github.com/polkit-org/polkit/blob/127/src/polkitagent/polkitagentsession.c#L609-L676>
//! - The helper: socket activation, the user read from the socket and
//!   the caller from `SO_PEERCRED`/`SO_PEERPIDFD` (L140-L180), escaped
//!   output (`send_to_helper`, L49-L80), answers read with `fgets` into
//!   `PAM_MAX_RESP_SIZE` bytes (L316, L341):
//!   <https://github.com/polkit-org/polkit/blob/127/src/polkitagent/polkitagenthelper-pam.c#L140-L180>
//! - The systemd units:
//!   <https://github.com/polkit-org/polkit/blob/127/data/polkit-agent-helper.socket>,
//!   <https://github.com/polkit-org/polkit/blob/127/data/polkit-agent-helper@.service.in>
//! - polkitd expanding groups into users before calling the agent
//!   (L2508-L2523):
//!   <https://github.com/polkit-org/polkit/blob/127/src/polkitbackend/polkitbackendinteractiveauthority.c#L2508-L2523>
//! - `PAM_MAX_MSG_SIZE` and `PAM_MAX_RESP_SIZE` (512), Linux-PAM:
//!   <https://github.com/linux-pam/linux-pam/blob/master/libpam/include/security/_pam_types.h>
//! - The escaping, GLib's `g_strescape` and its inverse `g_strcompress`:
//!   <https://docs.gtk.org/glib/func.strescape.html>,
//!   <https://docs.gtk.org/glib/func.strcompress.html>
//! - `SO_PEERCRED`, unix(7):
//!   <https://man7.org/linux/man-pages/man7/unix.7.html>

use std::ffi::CStr;
use std::net::Shutdown;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use mecha_wayland::prelude::*;
use zeroize::Zeroize;

use crate::agent::{AuthCancelled, AuthFinished, AuthRequested, Identity};

/// Where polkit's socket-activated helper listens.
pub(crate) const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Wrong answers in a row before the request fails.
pub const MAX_FAILURES: u32 = 3;

/// The longest answer the helper takes whole: it reads one with `fgets`
/// into `PAM_MAX_RESP_SIZE` (512) bytes, newline and NUL included. A
/// longer one would be split into two answers. See the helper's
/// conversation function:
const MAX_ANSWER: usize = 510;

/// The longest line taken from the helper, before it is unescaped. PAM's
/// messages are at most `PAM_MAX_MSG_SIZE` (512) bytes, and `g_strescape`
/// writes a byte as at most 4 (`\ooo`): 2 KiB, with room to spare.
const MAX_LINE: usize = 16 * 1024;

/// How much one read asks for. Our choice: a line can span reads.
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

/// The answer was wrong. A new attempt starts: its prompts follow.
#[derive(Debug, Clone)]
pub struct AuthRetry {
    pub cookie: String,
}
impl Signal for AuthRetry {}

/// Answer the open prompt of the request with this cookie. Nothing when
/// none is open. One too long to send, or with a line break in it, is
/// refused with an [`AuthMessage`], and the prompt stays open.
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

/// One line from the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Prompt { text: String, echo: bool },
    Info(String),
    Error(String),
    Success,
    Failure,
}

/// The request being authenticated.
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
    /// Bytes read after the last line break.
    pending: Vec<u8>,
    /// A prompt waits for its answer.
    prompt_open: bool,
    /// Ended: nothing it says counts. It stays only until the ring hands
    /// back its buffers.
    over: bool,
}

impl Conn {
    fn holds(&self, token: Token) -> bool {
        self.reading == Some(token) || self.sending.contains(&token)
    }

    fn gone(&self) -> bool {
        self.reading.is_none() && self.sending.is_empty()
    }

    /// End the attempt: the helper reads the end of the stream, and our
    /// read in flight completes empty.
    fn end(&mut self) {
        self.over = true;
        self.prompt_open = false;
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

pub(crate) struct Helper {
    /// The agent's bus: losing it drops the request.
    bus: BusId,
    socket: PathBuf,
    session: Option<Session>,
    /// Every connection not yet gone. At most one is not over: the
    /// current attempt's.
    conns: Vec<Conn>,
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

/// Authenticate the requests of the agent on `bus` through the helper at
/// `socket`. Installs after `RingModule`.
pub(crate) fn install(app: &mut App, bus: BusId, socket: PathBuf) {
    if app.has_resource::<Helper>() {
        return;
    }
    app.insert_resource(Helper {
        bus,
        socket,
        session: None,
        conns: Vec::new(),
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
    // One request at a time: one left over is gone.
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

/// Start an attempt for the session: connect, and say who and which
/// request.
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

/// Drop the session, ending its attempt. Its cookie, if there was one.
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
    if answer.len() > MAX_ANSWER || answer.contains(&b'\n') || answer.contains(&0) {
        app.signal(AuthMessage {
            cookie: e.cookie.clone(),
            text: "That answer is too long, or has a line break in it".into(),
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
    let mut lines = Vec::new();
    // The current attempt broke: hung up without a verdict, a send
    // failed, or a line too long.
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
                // Ended: what it says no longer counts.
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

/// A line from the current attempt. False once the attempt is over:
/// what follows it doesn't count.
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

/// The attempt failed: try again, or give up after [`MAX_FAILURES`].
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
        fail(app, "Authentication failed");
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

/// Undo `g_strescape`, as `g_strcompress` does: `\b \f \n \r \t \v`, up
/// to three octal digits as one byte, and any other escaped byte as
/// itself. A lone `\` at the end is dropped.
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

/// Who to authenticate as: `own` if it may, else root, else the first user
/// listed. GNOME Shell's order, and COSMIC's.
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
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// The name of the user with `uid`, from the passwd database.
fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: an all-zero passwd is a valid value to be overwritten.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: `pwd`, `buf` and `result` outlive the call, and `buf`'s
        // length is what is passed.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return None;
        }
        // SAFETY: on success `pw_name` points to a NUL-terminated string
        // in `buf`, which is still alive.
        let name = unsafe { CStr::from_ptr(pwd.pw_name) };
        return name.to_str().ok().map(str::to_owned);
    }
}

#[cfg(test)]
mod tests;
