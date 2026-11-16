//! Starting an app as a systemd user service of its own.
//!
//! Each app runs as `app-mechanix-<desktop id>@<random>.service`, the
//! freedesktop naming for apps, started by the user's systemd. It is part
//! of the graphical session, so it stops with it; it is in its own cgroup
//! from its first instruction; its output goes to the journal; and other
//! tools (the portals, resource managers, the running apps) can tell which
//! app a process belongs to. Without a session bus, or when systemd won't
//! create the unit, the app is spawned from the launcher instead.
//!
//! systemd answers a start once its job is queued; whether the program ran
//! comes later, as the job's `JobRemoved`, and a failure is logged.

use std::fmt::Write;
use std::hash::{BuildHasher, RandomState};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use apps::DesktopApp;
use mecha_wayland::prelude::*;

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";

/// Variables systemd sets for each unit itself: the launcher's own must not
/// reach the app's.
const UNIT_VARIABLES: &[&str] = &[
    "INVOCATION_ID",
    "JOURNAL_STREAM",
    "LISTEN_FDNAMES",
    "LISTEN_FDS",
    "LISTEN_PID",
    "MAINPID",
    "MANAGERPID",
    "NOTIFY_SOCKET",
    "SYSTEMD_EXEC_PID",
    "WATCHDOG_PID",
    "WATCHDOG_USEC",
];

/// Starts in flight: services asked of systemd and not yet answered, with
/// the app to spawn if it refuses, and queued jobs not yet done.
#[derive(Default)]
struct Starting {
    calls: Vec<(CallId, DesktopApp)>,
    jobs: Vec<(ObjectPath, String)>,
    /// The match for systemd's `JobRemoved`, once subscribed.
    jobs_removed: Option<MatchId>,
}
impl Resource for Starting {}

/// Registers the launcher's systems. Safe to call more than once.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Starting>() {
        return;
    }
    app.init_resource::<Starting>()
        .system(on_reply)
        .system(on_signal);
}

/// Start `entry`: as a service when there is a session bus, spawned from
/// the launcher otherwise.
pub(crate) fn start(app: &mut App, entry: &DesktopApp) {
    let Some(call) = service_call(entry) else {
        return spawn_logged(entry);
    };
    subscribe_to_jobs(app);
    match send(app, call) {
        Some(id) => app
            .resource_mut::<Starting>()
            .calls
            .push((id, entry.clone())),
        None => spawn_logged(entry),
    }
}

/// Send `call` on the session bus. `None` without one.
fn send(app: &mut App, call: MethodCall) -> Option<CallId> {
    if !app.has_resource::<DBus>() {
        return None;
    }
    let mut dbus = app.resource_mut::<DBus>();
    dbus.bus(BusId::Session).map(|mut bus| bus.send(call))
}

/// Ask systemd for its `JobRemoved`, which it sends only to subscribers.
fn subscribe_to_jobs(app: &mut App) {
    if app.resource::<Starting>().jobs_removed.is_some() || !app.has_resource::<DBus>() {
        return;
    }
    let matched = {
        let mut dbus = app.resource_mut::<DBus>();
        dbus.bus(BusId::Session).map(|mut bus| {
            bus.send_no_reply(MethodCall::new(
                SYSTEMD,
                SYSTEMD_PATH,
                SYSTEMD_MANAGER,
                "Subscribe",
            ));
            bus.add_match(
                MatchRule::signal()
                    .sender(SYSTEMD)
                    .interface(SYSTEMD_MANAGER)
                    .member("JobRemoved"),
            )
        })
    };
    app.resource_mut::<Starting>().jobs_removed = matched;
}

/// systemd's answer to a service start: queued, it is a job to watch;
/// refused, the app is spawned instead.
fn on_reply(app: &mut App, e: &MethodReply) {
    let entry = {
        let mut starting = app.resource_mut::<Starting>();
        let Some(at) = starting.calls.iter().position(|(id, _)| *id == e.call) else {
            return;
        };
        starting.calls.swap_remove(at).1
    };
    match &e.result {
        Ok(reply) => watch_job(app, reply, entry.id),
        Err(error) => {
            let id = &entry.id;
            tracing::warn!(%id, "systemd did not start it, spawning it instead: {error:?}");
            spawn_logged(&entry);
        }
    }
}

/// A unit start queued: its job, watched until done.
fn watch_job(app: &mut App, reply: &Message, id: String) {
    match reply.body::<(ObjectPath,)>() {
        Ok((job,)) => app.resource_mut::<Starting>().jobs.push((job, id)),
        Err(error) => tracing::warn!(%id, "systemd's answer is unreadable: {error:?}"),
    }
}

/// A start job is done: logged if the app didn't start.
fn on_signal(app: &mut App, e: &SignalReceived) {
    let ours = app
        .resource::<Starting>()
        .jobs_removed
        .is_some_and(|id| e.matches.contains(&id));
    if !ours {
        return;
    }
    let Ok((_, job, _, result)) = e.msg.body::<(u32, ObjectPath, String, String)>() else {
        return;
    };
    let id = {
        let mut starting = app.resource_mut::<Starting>();
        let Some(at) = starting.jobs.iter().position(|(path, _)| *path == job) else {
            return;
        };
        starting.jobs.swap_remove(at).1
    };
    if result != "done" {
        tracing::warn!(%id, %result, "the app did not start");
    }
}

/// The session's target, which an app's unit is part of, so it stops with
/// the session.
fn session_target() -> Value {
    Value::Array {
        elem: Signature::new("s").expect("a valid signature"),
        items: vec![Value::Str("graphical-session.target".to_owned())],
    }
}

/// `StartTransientUnit` for the app's service. `None` when its program is
/// not found on `PATH`.
fn service_call(entry: &DesktopApp) -> Option<MethodCall> {
    let program = executable(entry.exec.first()?)?;
    let signature = |s| Signature::new(s).expect("a valid signature");
    let strings = |items: Vec<String>| Value::Array {
        elem: signature("s"),
        items: items.into_iter().map(Value::Str).collect(),
    };
    let exec = Value::Array {
        elem: signature("(sasb)"),
        items: vec![Value::Struct(vec![
            Value::Str(program),
            strings(exec_args(&entry.exec)),
            Value::Bool(false),
        ])],
    };
    let text = |s: &str| Value::Str(s.to_owned());
    let mut properties: Vec<(&str, Value)> = vec![
        ("Description", text(&entry.name)),
        ("ExecStart", exec),
        // A program that can't be run fails the start job, rather than the
        // service after it.
        ("Type", text("exec")),
        // Running while any of its processes is: an app whose first process
        // hands over to another and exits isn't killed with it.
        ("ExitType", text("cgroup")),
        ("CollectMode", text("inactive-or-failed")),
        ("Slice", text("app.slice")),
        // Stopped with the session, and before it.
        ("PartOf", session_target()),
        ("After", session_target()),
        ("WorkingDirectory", text("~")),
        ("Environment", strings(environment())),
    ];
    if let Some(path) = entry.path.to_str() {
        properties.push(("SourcePath", text(path)));
    }
    let aux: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
    let name = service_name(&entry.id);
    let call = MethodCall::new(SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER, "StartTransientUnit")
        .body(&(name.as_str(), "fail", properties, aux));
    Some(call)
}

/// `argv` for `ExecStart`: systemd expands `$` in arguments, and `$$` keeps
/// one.
fn exec_args(argv: &[String]) -> Vec<String> {
    argv.iter().map(|arg| arg.replace('$', "$$")).collect()
}

/// The launcher's environment, as the app would have inherited it, less
/// what systemd sets for each unit and what it would refuse: names other
/// than ASCII letters, digits and `_`, or starting with a digit, and
/// anything that isn't UTF-8.
fn environment() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(key, _)| valid_name(key) && !UNIT_VARIABLES.contains(&key.as_str()))
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

/// Whether systemd takes `name` as an environment variable's.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `name` as systemd needs it, an absolute path: as given when it has a
/// `/`, else the first executable of that name on `PATH`.
fn executable(name: &str) -> Option<String> {
    if name.contains('/') {
        return Some(name.to_owned());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
        .and_then(|found| found.into_os_string().into_string().ok())
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The service's name: the desktop id without `.desktop`, escaped as systemd
/// escapes unit names (`-` is `\x2d`), then a random instance so the app
/// can run more than once.
fn service_name(desktop_id: &str) -> String {
    let id = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    let mut name = String::from("app-mechanix-");
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b"_.:".contains(&b) {
            name.push(char::from(b));
        } else {
            let _ = write!(name, "\\x{b:02x}");
        }
    }
    let random = RandomState::new().hash_one(desktop_id);
    let _ = write!(name, "@{random:016x}.service");
    name
}

/// Spawn the app from the launcher, logging a failure.
fn spawn_logged(entry: &DesktopApp) {
    if let Err(e) = spawn(&entry.exec) {
        tracing::warn!(id = %entry.id, "launch failed: {e}");
    }
}

/// Runs `argv` in its own process group with the launcher's environment and
/// no stdio. A thread reaps the child so it never lingers as a zombie.
fn spawn(argv: &[String]) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_or_reports() {
        assert!(spawn(&["true".to_string()]).is_ok());
        assert!(spawn(&[]).is_err());
        assert!(spawn(&["/nonexistent/app-drawer-test".to_string()]).is_err());
    }

    #[test]
    fn service_names_escape_the_desktop_id_and_differ() {
        let name = service_name("org.gnome.Nautilus.desktop");
        let instance = name
            .strip_prefix("app-mechanix-org.gnome.Nautilus@")
            .and_then(|rest| rest.strip_suffix(".service"))
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(instance.len(), 16);
        assert!(service_name("foot-server.desktop").starts_with("app-mechanix-foot\\x2dserver@"));
        assert_ne!(service_name("a.desktop"), service_name("a.desktop"));
    }

    #[test]
    fn service_calls_build() {
        // Building a call encodes its body, which checks every value's type.
        let entry = DesktopApp {
            id: "org.example.Notes.desktop".into(),
            name: "Notes".into(),
            exec: vec!["sh".into()],
            icon: None,
            startup_wm_class: None,
            path: "/usr/share/applications/notes.desktop".into(),
        };
        assert!(service_call(&entry).is_some());
    }

    #[test]
    fn programs_are_found_on_path() {
        let sh = executable("sh").expect("sh is on PATH");
        assert!(sh.starts_with('/') && sh.ends_with("/sh"), "{sh}");
        assert_eq!(executable("/bin/sh").as_deref(), Some("/bin/sh"));
        assert_eq!(executable("no-such-program-for-app-drawer"), None);
    }

    #[test]
    fn the_units_own_variables_stay_behind() {
        let env = environment();
        assert!(env.iter().any(|v| v.starts_with("PATH=")));
        for name in UNIT_VARIABLES {
            assert!(!env.iter().any(|v| v.starts_with(&format!("{name}="))));
        }
    }

    #[test]
    fn dollars_survive_systemds_expansion() {
        let argv = ["sh", "-c", "echo $HOME $$"].map(String::from);
        assert_eq!(exec_args(&argv), ["sh", "-c", "echo $$HOME $$$$"]);
    }

    #[test]
    fn only_names_systemd_takes_are_passed() {
        for good in ["PATH", "_x", "XDG_SESSION_ID", "a1"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "1A", "FOO-BAR", "a.b", "ÄRGER", "has space"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
