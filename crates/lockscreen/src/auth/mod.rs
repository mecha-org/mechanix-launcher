//! Unlock methods, each its own PAM service, run in worker processes (see
//! [`worker`]): one attempt at a time a method, several methods at once.
//!
//! - [`start`] spawns `/proc/self/exe --auth-worker <service>` with one
//!   end of a `SOCK_SEQPACKET` socketpair as its stdin, and opens a pidfd
//!   on it. The other end has a `recvmsg` kept waiting on the ring; the
//!   pidfd has a one-shot `poll`, which completes when the worker exits.
//!   Nothing else waits on anything.
//! - Each packet a worker sends is an [`AuthEvent`], with its method.
//!   [`answer`] sends a method the answer to its last prompt. [`cancel`]
//!   kills its worker.
//! - An attempt is gone once its socket read has ended and its exit was
//!   seen, and the process is reaped then. Until both, its buffers are
//!   the ring's and its fds stay open.

pub mod wire;
pub mod worker;

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::process::{Child, Command, Stdio};

use mecha_wayland::prelude::*;
use zeroize::Zeroize;

pub use wire::{FromWorker, Outcome};

/// The argument that makes the launcher binary an auth worker.
pub const WORKER_ARG: &str = "--auth-worker";

/// A way to unlock, each with its own PAM service file in `/etc/pam.d`
/// (the crate's `resources/pam.d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// A numeric PIN, typically `pam_pwdfile`.
    Pin,
    /// The account password, typically through `system-auth`.
    Password,
}

impl Method {
    /// Every method.
    pub const ALL: [Method; 2] = [Method::Pin, Method::Password];

    pub fn service(self) -> &'static str {
        match self {
            Method::Pin => "mechanix-pin",
            Method::Password => "mechanix",
        }
    }

    /// Answered by typing. There is one input, so one typed method runs at
    /// a time, the user switching between them. A method that waits on
    /// hardware instead (a fingerprint, a security key) runs alongside it.
    pub fn is_typed(self) -> bool {
        match self {
            Method::Pin | Method::Password => true,
        }
    }
}

/// One message from a method's current attempt's worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEvent {
    pub method: Method,
    pub message: FromWorker,
}
impl Signal for AuthEvent {}

struct Attempt {
    method: Method,
    child: Child,
    sock: OwnedFd,
    /// Kept open while its poll may be in flight.
    _pidfd: OwnedFd,
    /// The socket read in flight, until the worker hangs up.
    reading: Option<Token>,
    /// The pidfd poll in flight, until the worker exits.
    exiting: Option<Token>,
    /// Answers in flight: their buffers come back to be wiped.
    sending: Vec<Token>,
    /// Over: cancelled, or its outcome reported. Nothing more it says
    /// counts; it stays only until the ring hands back its buffers.
    over: bool,
}

impl Attempt {
    fn holds(&self, token: Token) -> bool {
        self.reading == Some(token) || self.exiting == Some(token) || self.sending.contains(&token)
    }

    fn gone(&self) -> bool {
        self.reading.is_none() && self.exiting.is_none() && self.sending.is_empty()
    }
}

/// Every attempt not yet gone. At most one a method is not over: its
/// current one.
#[derive(Default)]
pub(crate) struct Auth {
    attempts: Vec<Attempt>,
}
impl Resource for Auth {}

impl Auth {
    fn current(&mut self, method: Method) -> Option<&mut Attempt> {
        self.attempts
            .iter_mut()
            .find(|a| a.method == method && !a.over)
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Auth>().system(on_io);
}

fn seqpacket_pair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors returned.
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both were just returned to us, owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn pidfd_open(pid: u32) -> std::io::Result<OwnedFd> {
    let pid = libc::pid_t::try_from(pid).map_err(std::io::Error::other)?;
    // SAFETY: a plain syscall; a non-negative result is a new fd we own.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let Ok(fd) = i32::try_from(fd) else {
        return Err(std::io::Error::other("pidfd_open: fd out of range"));
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: as above.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Start an attempt with `method`, ending its current one. False if the
/// worker could not be spawned.
pub(crate) fn start(app: &mut App, method: Method) -> bool {
    cancel(app, method);
    let spawned = seqpacket_pair().and_then(|(ours, theirs)| {
        let mut child = Command::new("/proc/self/exe")
            .arg(WORKER_ARG)
            .arg(method.service())
            .stdin(Stdio::from(theirs))
            .stdout(Stdio::null())
            .spawn()?;
        // Our unreaped child: its pid can't be reused yet.
        match pidfd_open(child.id()) {
            Ok(pidfd) => Ok((ours, child, pidfd)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    });
    let (sock, child, pidfd) = match spawned {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("starting the auth worker: {e}");
            return false;
        }
    };
    let (reading, exiting) = {
        let mut ring = app.resource_mut::<Ring>();
        (
            ring.recvmsg(sock.as_fd(), Vec::with_capacity(wire::MAX_PACKET), 0),
            ring.poll(pidfd.as_fd(), libc::POLLIN as u32),
        )
    };
    app.resource_mut::<Auth>().attempts.push(Attempt {
        method,
        child,
        sock,
        _pidfd: pidfd,
        reading: Some(reading),
        exiting: Some(exiting),
        sending: Vec::new(),
        over: false,
    });
    true
}

/// Answer the last prompt of `method`'s current attempt.
pub(crate) fn answer(app: &mut App, method: Method, text: &[u8]) {
    let packet = wire::encode_answer(text);
    let (mut auth, mut ring) = app.query::<(ResMut<Auth>, ResMut<Ring>)>();
    let Some(a) = auth.current(method) else {
        return;
    };
    let token = ring.sendmsg(a.sock.as_fd(), packet, Vec::new());
    a.sending.push(token);
}

/// End `method`'s current attempt, if any: its worker is killed, and
/// nothing it says is reported.
pub(crate) fn cancel(app: &mut App, method: Method) {
    if let Some(a) = app.resource_mut::<Auth>().current(method) {
        a.over = true;
        let _ = a.child.kill();
    }
}

/// End every current attempt.
pub(crate) fn cancel_all(app: &mut App) {
    for m in Method::ALL {
        cancel(app, m);
    }
}

fn on_io(app: &mut App, e: &IoEvent) {
    let mut reports = Vec::new();
    {
        let (mut auth, mut ring) = app.query::<(ResMut<Auth>, ResMut<Ring>)>();
        let Some(a) = auth.attempts.iter_mut().find(|a| a.holds(e.token)) else {
            return;
        };
        let done = ring.finish(e.token);
        if a.exiting == Some(e.token) {
            a.exiting = None;
            // It exited: reap it.
            let _ = a.child.try_wait();
        } else if a.reading == Some(e.token) {
            a.reading = None;
            let mut buf = done.map(|c| c.buf).unwrap_or_default();
            let message = (e.result > 0)
                .then(|| FromWorker::decode(&buf))
                .flatten()
                // Hung up without a verdict: it crashed, or was killed.
                .or_else(|| (e.result <= 0).then_some(FromWorker::Done(Outcome::Error)));
            let finished = matches!(message, Some(FromWorker::Done(_)));
            if let Some(m) = message
                && !a.over
            {
                reports.push((a.method, m));
            }
            a.over |= finished;
            if !finished {
                buf.clear();
                a.reading = Some(ring.recvmsg(a.sock.as_fd(), buf, 0));
            }
        } else {
            a.sending.retain(|t| *t != e.token);
            if let Some(mut c) = done {
                c.buf.zeroize();
            }
        }
        auth.attempts.retain(|a| !a.gone());
    }
    for (method, message) in reports {
        app.signal(AuthEvent { method, message });
    }
}
