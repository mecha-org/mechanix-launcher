//! Force closing, as systemd stops a unit: the app is asked to end with
//! SIGTERM, and whatever is left after [`KILL_WAIT`] is killed with
//! SIGKILL. An app that handles SIGTERM gets to save on its way out.
//!
//! A process in an app's own systemd unit (see `apps::app_unit`) ends with
//! the whole unit, helpers and all; any other is signalled alone, since its
//! unit may hold more than the app (a terminal, the launcher), through a
//! pidfd, so the late SIGKILL never reaches a process that took its pid
//! since. Never the launcher itself.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use mecha_wayland::prelude::*;

/// How long an app has to end on SIGTERM before it is killed.
pub(crate) const KILL_WAIT: Duration = Duration::from_secs(5);

/// What a force close signals.
enum Target {
    /// An app's own unit, every process in it.
    Unit(String),
    /// One process, by its pidfd.
    Process(OwnedFd),
}

impl Target {
    fn process(pid: u32) -> Option<Target> {
        let pid = libc::pid_t::try_from(pid).ok()?;
        // SAFETY: pidfd_open(2) takes plain integers and returns a new fd.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        let fd = i32::try_from(fd).ok().filter(|fd| *fd >= 0)?;
        // SAFETY: the fd was just opened and is owned by nothing else.
        Some(Target::Process(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn signal(&self, app: &mut App, signal: i32) {
        match self {
            Target::Unit(unit) => {
                if !app.has_resource::<DBus>() {
                    return;
                }
                let mut dbus = app.resource_mut::<DBus>();
                let Some(mut bus) = dbus.bus(BusId::Session) else {
                    return;
                };
                let call = MethodCall::new(
                    "org.freedesktop.systemd1",
                    "/org/freedesktop/systemd1",
                    "org.freedesktop.systemd1.Manager",
                    "KillUnit",
                )
                .body(&(unit.as_str(), "all", signal));
                bus.send_no_reply(call);
            }
            Target::Process(fd) => {
                // SAFETY: pidfd_send_signal(2) reads no memory with a null
                // info. A process that has gone fails it, harmlessly.
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    );
                }
            }
        }
    }
}

/// Force closes waiting out [`KILL_WAIT`], by their ring timeouts.
#[derive(Default)]
struct Killing {
    pending: Vec<(Token, Vec<Target>)>,
}
impl Resource for Killing {}

/// Registers the SIGKILL that follows. Safe to call more than once.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Killing>() {
        return;
    }
    app.init_resource::<Killing>().system(on_timeout);
}

/// Production killer: SIGTERM to the app's processes now, SIGKILL to
/// what is left after [`KILL_WAIT`]. Without a ring to wait on, SIGKILL
/// at once.
pub(crate) fn kill_processes(app: &mut App, pids: &[u32]) {
    let has_bus =
        app.has_resource::<DBus>() && app.resource_mut::<DBus>().bus(BusId::Session).is_some();
    let mut targets: Vec<Target> = Vec::new();
    for &pid in pids {
        if pid <= 1 || pid == std::process::id() {
            continue;
        }
        let target = match apps::app_unit(pid).filter(|_| has_bus) {
            Some(unit) => {
                let seen = targets
                    .iter()
                    .any(|t| matches!(t, Target::Unit(u) if *u == unit));
                if seen {
                    continue;
                }
                Some(Target::Unit(unit))
            }
            None => Target::process(pid),
        };
        targets.extend(target);
    }
    if !(app.has_resource::<Ring>() && app.has_resource::<Killing>()) {
        for target in &targets {
            target.signal(app, libc::SIGKILL);
        }
        return;
    }
    for target in &targets {
        target.signal(app, libc::SIGTERM);
    }
    let token = app.resource_mut::<Ring>().timeout(KILL_WAIT);
    app.resource_mut::<Killing>().pending.push((token, targets));
}

/// The wait is over: SIGKILL to what is left.
fn on_timeout(app: &mut App, e: &IoEvent) {
    let targets = {
        let (mut killing, mut ring) = app.query::<(ResMut<Killing>, ResMut<Ring>)>();
        let Some(at) = killing.pending.iter().position(|(t, _)| *t == e.token) else {
            return;
        };
        let _ = ring.finish(e.token);
        killing.pending.swap_remove(at).1
    };
    for target in &targets {
        target.signal(app, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    use super::*;

    #[test]
    fn a_process_that_ignores_sigterm_ends_on_sigkill() {
        let mut child = Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 30"])
            .spawn()
            .unwrap();
        // Let the shell set its trap.
        std::thread::sleep(Duration::from_millis(200));
        let target = Target::process(child.id()).unwrap();
        let mut app = App::new();
        target.signal(&mut app, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(200));
        assert!(child.try_wait().unwrap().is_none(), "SIGTERM is ignored");
        target.signal(&mut app, libc::SIGKILL);
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn a_process_gone_is_signalled_no_more() {
        let mut child = Command::new("true").spawn().unwrap();
        let target = Target::process(child.id()).unwrap();
        child.wait().unwrap();
        let Target::Process(fd) = &target else {
            unreachable!()
        };
        // SAFETY: as in `Target::signal`.
        let sent = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        assert_eq!(sent, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
