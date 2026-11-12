//! The auth worker: a child process that runs one PAM transaction for one
//! service and talks to the launcher over its stdin, a `SOCK_SEQPACKET`
//! socket (see [`wire`](super::wire)).
//!
//! PAM modules are arbitrary C code, so they run here rather than in the
//! launcher: a crash or a hang takes only this process, and cancelling an
//! attempt is killing it. It dies with the launcher (`PR_SET_PDEATHSIG`)
//! and is never dumpable, so a core never holds a secret.
//!
//! The transaction: `pam_authenticate`, then a best-effort
//! `pam_setcred(PAM_REFRESH_CRED)`. There
//! is no `pam_acct_mgmt`: the account was checked when the session began,
//! and unlocking only proves the user is the one logged in. (It would also
//! need `pam_unix`'s setuid helper, for `/etc/shadow`, from here.) Every
//! prompt waits for an answer from the launcher; info and error text is
//! passed on. The failure delay a module asks for is reported, then slept
//! here.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

use mecha_wayland::prelude::Secret;
use zeroize::Zeroize;

use super::wire::{self, FromWorker, Outcome};

// ── libpam ─────────────────────────────────────────────────────────────

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConvFn = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<ConvFn>,
    appdata_ptr: *mut c_void,
}

type DelayFn = unsafe extern "C" fn(c_int, c_uint, *mut c_void);

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_end(pamh: *mut PamHandle, status: c_int) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_set_item(pamh: *mut PamHandle, item_type: c_int, item: *const c_void) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
}

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_PERM_DENIED: c_int = 6;
const PAM_AUTH_ERR: c_int = 7;
const PAM_CRED_INSUFFICIENT: c_int = 8;
const PAM_AUTHINFO_UNAVAIL: c_int = 9;
const PAM_USER_UNKNOWN: c_int = 10;
const PAM_MAXTRIES: c_int = 11;
const PAM_CONV_ERR: c_int = 19;
const PAM_MODULE_UNKNOWN: c_int = 28;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;

const PAM_FAIL_DELAY: c_int = 10;
const PAM_REFRESH_CRED: c_int = 0x0010;

// ── the conversation ───────────────────────────────────────────────────

struct Conversation {
    sock: UnixDatagram,
}

impl Conversation {
    fn send(&self, m: &FromWorker) -> bool {
        self.sock.send(&m.encode()).is_ok()
    }

    /// Ask, and wait for the answer. `None` if the launcher is gone or
    /// sent something else.
    fn ask(&self, secret: bool, text: String) -> Option<Secret> {
        if !self.send(&FromWorker::Prompt { secret, text }) {
            return None;
        }
        let mut packet = Secret::from(vec![0u8; wire::MAX_PACKET]);
        let n = self.sock.recv(packet.as_bytes_mut()).ok()?;
        let answer = wire::decode_answer(&packet.as_bytes()[..n])?;
        // A NUL inside an answer would truncate it; refuse it rather.
        (!answer.contains(&0)).then(|| Secret::from(answer.to_vec()))
    }

    /// Handle one message: a prompt's answer for libpam, `None` for the
    /// rest, or the PAM error that ends the conversation.
    fn reply(&self, m: &PamMessage) -> Result<Option<Secret>, c_int> {
        let text = if m.msg.is_null() {
            String::new()
        } else {
            // SAFETY: a NUL-terminated string owned by the module.
            unsafe { CStr::from_ptr(m.msg) }
                .to_string_lossy()
                .into_owned()
        };
        match m.msg_style {
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => self
                .ask(m.msg_style == PAM_PROMPT_ECHO_OFF, text)
                .map(Some)
                .ok_or(PAM_CONV_ERR),
            PAM_ERROR_MSG => {
                self.send(&FromWorker::Error(text));
                Ok(None)
            }
            PAM_TEXT_INFO => {
                self.send(&FromWorker::Info(text));
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

/// `answer` as a `malloc`ed C string, which libpam frees.
fn c_string(answer: &Secret) -> Result<*mut c_char, c_int> {
    let bytes = answer.as_bytes();
    // SAFETY: room for the answer and its NUL, checked for null.
    let s: *mut u8 = unsafe { libc::malloc(bytes.len() + 1) }.cast();
    if s.is_null() {
        return Err(PAM_BUF_ERR);
    }
    // SAFETY: `s` holds `bytes.len() + 1` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), s, bytes.len());
        *s.add(bytes.len()) = 0;
    }
    Ok(s.cast())
}

/// # Safety
///
/// Called by libpam with `n` messages and the `appdata_ptr` set up in
/// [`run`]: a live `Conversation`.
unsafe extern "C" fn converse(
    n: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    let Ok(count) = usize::try_from(n) else {
        return PAM_CONV_ERR;
    };
    if count == 0 || msg.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: `appdata` is the `Conversation` `run` lent libpam.
    let conv = unsafe { &*appdata.cast::<Conversation>() };
    // libpam frees the responses and their strings with `free`.
    // SAFETY: a plain allocation, checked for null below.
    let replies: *mut PamResponse =
        unsafe { libc::calloc(count, std::mem::size_of::<PamResponse>()) }.cast();
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..count {
        // Linux-PAM passes an array of pointers to messages.
        // SAFETY: libpam hands `count` valid message pointers.
        let m = unsafe { &**msg.add(i) };
        let filled = conv
            .reply(m)
            .and_then(|answer| answer.as_ref().map(c_string).transpose());
        match filled {
            // SAFETY: `i` is within the `count` responses.
            Ok(Some(s)) => unsafe { (*replies.add(i)).resp = s },
            Ok(None) => {}
            Err(rc) => {
                // SAFETY: frees what this call allocated so far.
                unsafe { free_replies(replies, i) };
                return rc;
            }
        }
    }
    // SAFETY: `resp` is libpam's out pointer.
    unsafe { *resp = replies };
    PAM_SUCCESS
}

/// Free the first `filled` responses' strings, wiped, and the array.
///
/// # Safety
///
/// `replies` is the `calloc`ed array of [`converse`].
unsafe fn free_replies(replies: *mut PamResponse, filled: usize) {
    for i in 0..filled {
        // SAFETY: each `resp` is null or `malloc`ed by `c_string`.
        unsafe {
            let r = (*replies.add(i)).resp;
            if !r.is_null() {
                std::slice::from_raw_parts_mut(r.cast::<u8>(), libc::strlen(r)).zeroize();
                libc::free(r.cast());
            }
        }
    }
    // SAFETY: `calloc`ed by `converse`.
    unsafe { libc::free(replies.cast()) };
}

/// # Safety
///
/// Called by libpam with the conversation's `appdata_ptr`.
unsafe extern "C" fn fail_delay(retval: c_int, usec: c_uint, appdata: *mut c_void) {
    if retval == PAM_SUCCESS || appdata.is_null() || usec == 0 {
        return;
    }
    // SAFETY: as in `converse`.
    let conv = unsafe { &*appdata.cast::<Conversation>() };
    let d = Duration::from_micros(usec.into());
    conv.send(&FromWorker::Delay(d));
    std::thread::sleep(d);
}

// ── the transaction ────────────────────────────────────────────────────

fn outcome(rc: c_int) -> Outcome {
    match rc {
        PAM_SUCCESS => Outcome::Success,
        PAM_AUTH_ERR | PAM_PERM_DENIED | PAM_CRED_INSUFFICIENT | PAM_MAXTRIES => Outcome::Failed,
        PAM_AUTHINFO_UNAVAIL | PAM_USER_UNKNOWN | PAM_MODULE_UNKNOWN => Outcome::Unavailable,
        _ => Outcome::Error,
    }
}

fn strerror(h: *mut PamHandle, rc: c_int) -> String {
    // SAFETY: `pam_strerror` returns a static string, never null in
    // Linux-PAM.
    let s = unsafe { pam_strerror(h, rc) };
    if s.is_null() {
        return format!("PAM error {rc}");
    }
    // SAFETY: NUL-terminated and static.
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

fn run(service: &str, user: &str, conv: &Conversation) -> Outcome {
    let (Ok(service), Ok(user)) = (CString::new(service), CString::new(user)) else {
        return Outcome::Error;
    };
    let pam_conv = PamConv {
        conv: Some(converse),
        appdata_ptr: std::ptr::from_ref(conv).cast_mut().cast(),
    };
    let mut h: *mut PamHandle = std::ptr::null_mut();
    // SAFETY: valid strings and conversation, which outlive the handle.
    let rc = unsafe {
        pam_start(
            service.as_ptr(),
            user.as_ptr(),
            &raw const pam_conv,
            &raw mut h,
        )
    };
    if rc != PAM_SUCCESS || h.is_null() {
        eprintln!("[auth-worker] pam_start: {rc}");
        return Outcome::Unavailable;
    }
    // SAFETY: `h` is live; the item is a function pointer libpam calls
    // with the conversation's `appdata_ptr`.
    unsafe { pam_set_item(h, PAM_FAIL_DELAY, fail_delay as DelayFn as *const c_void) };
    // SAFETY: `h` is live for every call until `pam_end`.
    let rc = unsafe { pam_authenticate(h, 0) };
    let result = outcome(rc);
    if result == Outcome::Success {
        // SAFETY: as above.
        unsafe { pam_setcred(h, PAM_REFRESH_CRED) };
    } else {
        let error = strerror(h, rc);
        eprintln!("[auth-worker] {}: {error}", service.to_string_lossy());
        if result == Outcome::Error {
            conv.send(&FromWorker::Error(error));
        }
    }
    // SAFETY: ends the transaction; `h` is not used again.
    unsafe { pam_end(h, rc) };
    result
}

/// The user this process runs as.
fn user_name() -> Option<String> {
    // SAFETY: `getpwuid_r` with a buffer it may use; the name is copied
    // out before the buffer goes.
    unsafe {
        let uid = libc::geteuid();
        let mut pwd = std::mem::zeroed::<libc::passwd>();
        let mut buf = vec![0 as c_char; 4096];
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(uid, &raw mut pwd, buf.as_mut_ptr(), buf.len(), &raw mut out);
        if rc != 0 || out.is_null() || pwd.pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned())
    }
}

/// The worker's `main`: one transaction for `service`, then exit.
pub fn main(service: &str) -> ! {
    // SAFETY: plain prctl calls on this process.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
    }
    // SAFETY: the launcher hands the socket over as stdin; nothing else
    // in this process owns fd 0.
    let sock = UnixDatagram::from(unsafe { OwnedFd::from_raw_fd(0) });
    let conv = Conversation { sock };
    let outcome = match user_name() {
        Some(user) => run(service, &user, &conv),
        None => Outcome::Error,
    };
    conv.send(&FromWorker::Done(outcome));
    std::process::exit(0);
}
