//! Starting an app.
//!
//! 1. An activation token is asked of the compositor first, when it offers
//!    them, so the app's window may take focus. The app starts without one
//!    if it doesn't come within [`TOKEN_WAIT`].
//! 2. An app that is `DBusActivatable` is started through D-Bus
//!    (`org.freedesktop.Application.Activate`), with the token in the
//!    platform data. The bus starts it, or reaches the one already running.
//!    A bus that starts apps in units of their own leaves it there; one
//!    that runs it in its own unit, among the bus's other services, has it
//!    moved into a scope of its own, `app-mechanix-<desktop id>-<random>.scope`,
//!    so it can be told apart and handled alone.
//! 3. Any other app, or one whose activation fails, runs as a systemd user
//!    service of its own, `app-mechanix-<desktop id>@<random>.service`, the
//!    freedesktop naming for apps, with the token in its environment. It is
//!    part of the graphical session, so it stops with it; it is in its own
//!    cgroup from its first instruction; its output goes to the journal;
//!    and other tools (the portals, resource managers, the running apps)
//!    can tell which app a process belongs to.
//! 4. Without a session bus, or when systemd won't create the unit, the app
//!    is spawned from the launcher.
//!
//! systemd answers a start once its job is queued; whether the program ran
//! comes later, as the job's `JobRemoved`. A start that failed ends its
//! launch at once, as does a spawn that failed, and the app's unit going
//! away (`UnitRemoved`) before its window came: an app that exited or
//! crashed first.
//!
//! Only the app's own unit is stopped when its launch is called off: one
//! made for it here, or one named for it (`app-…`, or the unit the bus
//! started it in). Never a unit it merely runs in, like the launcher's own
//! or the bus's.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use apps::{DesktopApp, LaunchId, Launches};
use mecha_wayland::prelude::*;

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const APPLICATION: &str = "org.freedesktop.Application";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// How long an app waits for its activation token before starting without.
const TOKEN_WAIT: Duration = Duration::from_millis(500);

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

/// One app on its way to starting.
struct AppStart {
    entry: DesktopApp,
    launch: LaunchId,
    token: Option<String>,
}

/// An app waiting for its activation token.
struct Waiting {
    request: TokenRequest,
    /// The ring timeout that starts it without the token.
    timeout: Token,
    start: AppStart,
}

/// What stopping a launch stops.
enum Unit {
    /// The unit its app runs in.
    Known(String),
    /// Called off before the unit was known: stopped once it is.
    StopWhenKnown,
}

/// What a call in flight asked for.
enum Asked {
    /// The app's D-Bus activation.
    Activate(AppStart),
    /// The app's own service.
    Service(AppStart),
    /// The process of an app D-Bus activation reached.
    AppProcess(AppStart),
    /// The bus's own process.
    BusProcess,
    /// A scope for the launch's app, which the bus runs among its own
    /// services.
    Scope(LaunchId, String),
}

/// What a queued systemd start job starts.
enum Job {
    /// The launch's service: its failure is the app's.
    Service(LaunchId),
    /// The scope the launch's app is moved into: the app's unit once done.
    Scope(LaunchId, String),
}

/// Starts in flight: apps waiting for their token, calls not yet answered,
/// and queued systemd jobs not yet done.
#[derive(Default)]
struct Starting {
    waiting: Vec<Waiting>,
    /// Every ring timeout not finished yet, including those whose app got
    /// its token first.
    timeouts: Vec<Token>,
    calls: Vec<(CallId, Asked)>,
    jobs: Vec<(ObjectPath, Job)>,
    /// The matches for systemd's signals, once subscribed.
    subscribed: Option<Subscription>,
    /// The unit the bus itself runs in, once known.
    bus_unit: Option<String>,
    /// What stopping each launch stops, while it goes on.
    units: HashMap<LaunchId, Unit>,
}
impl Resource for Starting {}

/// The matches for systemd's `JobRemoved` and `UnitRemoved`.
#[derive(Clone, Copy)]
struct Subscription {
    jobs_removed: MatchId,
    units_removed: MatchId,
}

impl Starting {
    /// Whether a call or job still in flight may yet name `launch`'s unit.
    fn may_learn_unit(&self, launch: LaunchId) -> bool {
        let asked = self.calls.iter().any(|(_, asked)| match asked {
            Asked::Activate(s) | Asked::Service(s) | Asked::AppProcess(s) => s.launch == launch,
            Asked::Scope(l, _) => *l == launch,
            Asked::BusProcess => false,
        });
        asked
            || self
                .jobs
                .iter()
                .any(|(_, job)| matches!(job, Job::Scope(l, _) if *l == launch))
    }
}

/// Call off `launch`: an app still waiting for its token isn't started; one
/// started has its unit stopped, now or once known. An app spawned from the
/// launcher, outside any unit, or running in a unit not its own, can't be.
pub(crate) fn stop(app: &mut App, launch: LaunchId) {
    let mut starting = app.resource_mut::<Starting>();
    if let Some(at) = starting
        .waiting
        .iter()
        .position(|w| w.start.launch == launch)
    {
        starting.waiting.swap_remove(at);
        return;
    }
    match starting.units.remove(&launch) {
        Some(Unit::Known(unit)) => stop_unit(app, &unit),
        _ if starting.may_learn_unit(launch) => {
            starting.units.insert(launch, Unit::StopWhenKnown);
        }
        _ => {}
    }
}

/// Done with `launch`: its window came, or it ended without one.
pub(crate) fn forget(app: &mut App, launch: LaunchId) {
    app.resource_mut::<Starting>().units.remove(&launch);
}

/// `launch`'s app runs in `unit`, its own: stopped at once if it was called
/// off, else kept to stop while the launch goes on. Once it is over, the
/// window came or the app is gone, and there is nothing to keep.
fn started_in(app: &mut App, launch: LaunchId, unit: String) {
    let going_on =
        app.has_resource::<Launches>() && app.resource::<Launches>().get(launch).is_some();
    let mut starting = app.resource_mut::<Starting>();
    match starting.units.remove(&launch) {
        Some(Unit::StopWhenKnown) => stop_unit(app, &unit),
        _ if going_on => {
            starting.units.insert(launch, Unit::Known(unit));
        }
        _ => {}
    }
}

/// `launch`'s app turned out to have no unit of its own to stop: a stop
/// waiting for one is done, unless another call may still name one.
fn no_unit(app: &mut App, launch: LaunchId) {
    let mut starting = app.resource_mut::<Starting>();
    if !starting.may_learn_unit(launch)
        && matches!(starting.units.get(&launch), Some(Unit::StopWhenKnown))
    {
        starting.units.remove(&launch);
    }
}

/// Ask systemd to stop `unit`.
fn stop_unit(app: &mut App, unit: &str) {
    let call = MethodCall::new(SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER, "StopUnit")
        .body(&(unit, "replace"));
    if send(app, call).is_none() {
        tracing::warn!(%unit, "no session bus to stop the app's unit on");
    }
}

/// Registers the launcher's systems. Safe to call more than once.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Starting>() {
        return;
    }
    app.init_resource::<Starting>()
        .system(on_token)
        .system(on_timeout)
        .system(on_reply)
        .system(on_signal);
}

/// Start `entry` for `launch`, asking for its activation token first when
/// the compositor offers them.
pub(crate) fn start(app: &mut App, entry: &DesktopApp, launch: LaunchId) {
    let can_wait = app.has_resource::<Toplevels>()
        && app.has_resource::<Wayland>()
        && app.has_resource::<Ring>();
    let request = can_wait
        .then(|| {
            let (toplevels, mut wl) = app.query::<(Res<Toplevels>, ResMut<Wayland>)>();
            toplevels.request_activation_token(&mut wl, app_id(&entry.id))
        })
        .flatten();
    let start = AppStart {
        entry: entry.clone(),
        launch,
        token: None,
    };
    let Some(request) = request else {
        return run(app, start);
    };
    let timeout = app.resource_mut::<Ring>().timeout(TOKEN_WAIT);
    let mut starting = app.resource_mut::<Starting>();
    starting.timeouts.push(timeout);
    starting.waiting.push(Waiting {
        request,
        timeout,
        start,
    });
}

/// The token is here: start the app with it.
fn on_token(app: &mut App, e: &ActivationToken) {
    let mut start = {
        let mut starting = app.resource_mut::<Starting>();
        let Some(at) = starting.waiting.iter().position(|w| w.request == e.request) else {
            return;
        };
        starting.waiting.swap_remove(at).start
    };
    if !e.token.is_empty() {
        crate::placeholder::started_with(app, start.launch, &e.token);
        start.token = Some(e.token.clone());
    }
    run(app, start);
}

/// The token is late: start the app without it.
fn on_timeout(app: &mut App, e: &IoEvent) {
    let start = {
        let (mut starting, mut ring) = app.query::<(ResMut<Starting>, ResMut<Ring>)>();
        let Some(at) = starting.timeouts.iter().position(|t| *t == e.token) else {
            return;
        };
        starting.timeouts.swap_remove(at);
        let _ = ring.finish(e.token);
        let Some(at) = starting.waiting.iter().position(|w| w.timeout == e.token) else {
            return;
        };
        starting.waiting.swap_remove(at).start
    };
    let id = &start.entry.id;
    tracing::warn!(%id, "no activation token came, starting it without one");
    run(app, start);
}

/// Start through D-Bus activation when the app takes it, else as a service.
fn run(app: &mut App, start: AppStart) {
    let Some(call) = start
        .entry
        .dbus_activatable
        .then(|| activate_call(&start))
        .flatten()
    else {
        return start_service(app, start);
    };
    if app.resource::<Starting>().bus_unit.is_none() {
        ask_process(app, BUS, Asked::BusProcess);
    }
    match send(app, call) {
        Some(id) => awaiting(app, id, Asked::Activate(start)),
        None => start_service(app, start),
    }
}

/// Start `start` as a service, or spawn it when that can't be asked.
fn start_service(app: &mut App, start: AppStart) {
    let unit = service_name(&start.entry.id);
    let Some(call) = service_call(&start, &unit) else {
        return spawn_or_fail(app, &start);
    };
    subscribe_to_units(app);
    match send(app, call) {
        Some(id) => {
            started_in(app, start.launch, unit);
            awaiting(app, id, Asked::Service(start));
        }
        None => spawn_or_fail(app, &start),
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

/// The answer to call `id` goes to [`on_reply`] with `asked`.
fn awaiting(app: &mut App, id: CallId, asked: Asked) {
    app.resource_mut::<Starting>().calls.push((id, asked));
}

/// Ask the bus for the process that owns `name`.
fn ask_process(app: &mut App, name: &str, asked: Asked) {
    let call = MethodCall::new(BUS, BUS_PATH, BUS, "GetConnectionUnixProcessID").body(&(name,));
    if let Some(id) = send(app, call) {
        awaiting(app, id, asked);
    }
}

/// Ask systemd for its `JobRemoved` and `UnitRemoved`, which it sends only
/// to subscribers.
fn subscribe_to_units(app: &mut App) {
    if app.resource::<Starting>().subscribed.is_some() || !app.has_resource::<DBus>() {
        return;
    }
    let subscribed = app
        .resource_mut::<DBus>()
        .bus(BusId::Session)
        .map(|mut bus| {
            bus.send_no_reply(MethodCall::new(
                SYSTEMD,
                SYSTEMD_PATH,
                SYSTEMD_MANAGER,
                "Subscribe",
            ));
            let rule = |member| {
                MatchRule::signal()
                    .sender(SYSTEMD)
                    .interface(SYSTEMD_MANAGER)
                    .member(member)
            };
            Subscription {
                jobs_removed: bus.add_match(rule("JobRemoved")),
                units_removed: bus.add_match(rule("UnitRemoved")),
            }
        });
    app.resource_mut::<Starting>().subscribed = subscribed;
}

/// An answer. An activation refused falls back to a service; one done asks
/// which process the app is. A service or scope queued is a job to watch; a
/// service refused falls back to spawning.
fn on_reply(app: &mut App, e: &MethodReply) {
    let asked = {
        let mut starting = app.resource_mut::<Starting>();
        let Some(at) = starting.calls.iter().position(|(id, _)| *id == e.call) else {
            return;
        };
        starting.calls.swap_remove(at).1
    };
    match (asked, &e.result) {
        (Asked::Activate(start), Ok(_)) => {
            let name = app_id(&start.entry.id).to_owned();
            ask_process(app, &name, Asked::AppProcess(start));
        }
        (Asked::Activate(start), Err(error)) => {
            let id = &start.entry.id;
            tracing::warn!(%id, "D-Bus activation failed, starting it as a service: {error:?}");
            start_service(app, start);
        }
        (Asked::Service(start), Err(error)) => {
            let id = &start.entry.id;
            tracing::warn!(%id, "systemd did not start it, spawning it instead: {error:?}");
            // In no unit now.
            forget(app, start.launch);
            spawn_or_fail(app, &start);
        }
        (Asked::Service(start), Ok(reply)) => {
            watch_job(app, reply, Job::Service(start.launch));
        }
        (Asked::Scope(launch, scope), Ok(reply)) => {
            watch_job(app, reply, Job::Scope(launch, scope));
        }
        (Asked::Scope(launch, scope), Err(error)) => {
            tracing::warn!(%scope, "systemd made no scope for it: {error:?}");
            no_unit(app, launch);
        }
        (Asked::AppProcess(start), Ok(reply)) => match reply.body::<(u32,)>() {
            Ok((pid,)) => adopt(app, pid, &start),
            Err(_) => no_unit(app, start.launch),
        },
        (Asked::BusProcess, Ok(reply)) => {
            let unit = reply
                .body::<(u32,)>()
                .ok()
                .and_then(|(pid,)| apps::unit_of(pid));
            app.resource_mut::<Starting>().bus_unit = unit;
        }
        (Asked::AppProcess(start), Err(error)) => {
            let id = &start.entry.id;
            tracing::warn!(%id, "the bus did not say which process it is: {error:?}");
            no_unit(app, start.launch);
        }
        (Asked::BusProcess, Err(error)) => {
            tracing::warn!("the bus did not say which process it is: {error:?}");
        }
    }
}

/// A unit start queued: its job, watched until done.
fn watch_job(app: &mut App, reply: &Message, job: Job) {
    match reply.body::<(ObjectPath,)>() {
        Ok((path,)) => app.resource_mut::<Starting>().jobs.push((path, job)),
        Err(error) => {
            tracing::warn!("systemd's answer is unreadable: {error:?}");
            if let Job::Scope(launch, _) = job {
                no_unit(app, launch);
            }
        }
    }
}

/// The app D-Bus activation reached is process `pid`. When the bus runs it
/// in the bus's own unit, it is moved into a scope of its own: there it
/// can be told apart, frozen or ended without the bus, and that scope is
/// the app's once made. In a unit named for the app, one the bus started
/// it in, it stays, and that unit is the app's. In any other, it is left
/// alone: the unit is some other program's, like the launcher's own for an
/// app it spawned earlier, and stopping the launch must not stop it.
fn adopt(app: &mut App, pid: u32, start: &AppStart) {
    let Some(unit) = apps::unit_of(pid) else {
        return no_unit(app, start.launch);
    };
    let entry = &start.entry;
    if app.resource::<Starting>().bus_unit.as_ref() == Some(&unit) {
        subscribe_to_units(app);
        let scope = scope_name(&entry.id);
        match send(app, scope_call(entry, pid, &scope)) {
            Some(id) => awaiting(app, id, Asked::Scope(start.launch, scope)),
            None => no_unit(app, start.launch),
        }
    } else if apps::unit_app_id(&unit).as_deref() == Some(app_id(&entry.id)) {
        subscribe_to_units(app);
        started_in(app, start.launch, unit);
    } else {
        no_unit(app, start.launch);
    }
}

/// systemd's `JobRemoved` or `UnitRemoved`.
fn on_signal(app: &mut App, e: &SignalReceived) {
    let Some(subscribed) = app.resource::<Starting>().subscribed else {
        return;
    };
    if e.matches.contains(&subscribed.jobs_removed) {
        job_removed(app, &e.msg);
    } else if e.matches.contains(&subscribed.units_removed) {
        unit_removed(app, &e.msg);
    }
}

/// A start job is done. A service that failed to start ends its launch; a
/// scope made is the app's unit.
fn job_removed(app: &mut App, msg: &Message) {
    let Ok((_, path, unit, result)) = msg.body::<(u32, ObjectPath, String, String)>() else {
        return;
    };
    let job = {
        let mut starting = app.resource_mut::<Starting>();
        let Some(at) = starting.jobs.iter().position(|(p, _)| *p == path) else {
            return;
        };
        starting.jobs.swap_remove(at).1
    };
    let done = result == "done";
    match job {
        Job::Service(_) if done => {}
        Job::Service(launch) => {
            tracing::warn!(%unit, %result, "the app did not start");
            crate::placeholder::failed(app, launch);
        }
        Job::Scope(launch, scope) if done => started_in(app, launch, scope),
        Job::Scope(launch, _) => {
            tracing::warn!(%unit, %result, "systemd made no scope for the app");
            no_unit(app, launch);
        }
    }
}

/// A unit is gone. A launch whose app ran in it, its window not up yet, is
/// over: the app exited or crashed first.
fn unit_removed(app: &mut App, msg: &Message) {
    let Ok((unit, _)) = msg.body::<(String, ObjectPath)>() else {
        return;
    };
    let launch = app
        .resource::<Starting>()
        .units
        .iter()
        .find(|(_, u)| matches!(u, Unit::Known(known) if *known == unit))
        .map(|(launch, _)| *launch);
    if let Some(launch) = launch {
        tracing::warn!(%unit, "the app ended before its window came");
        crate::placeholder::failed(app, launch);
    }
}

/// The app id: the desktop id without `.desktop`.
fn app_id(desktop_id: &str) -> &str {
    desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id)
}

/// The bus name and object path D-Bus activation reaches the app at: its id,
/// and the id with `.` as `/` and `-` as `_`.
fn application(desktop_id: &str) -> (String, String) {
    let name = app_id(desktop_id);
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    (name.to_owned(), path)
}

/// `org.freedesktop.Application.Activate` for the app. `None` when its id
/// isn't a valid bus name.
fn activate_call(start: &AppStart) -> Option<MethodCall> {
    let (name, path) = application(&start.entry.id);
    let name = BusName::new(&name).ok()?;
    let path = ObjectPath::new(&path).ok()?;
    let entries = start
        .token
        .iter()
        .flat_map(|token| {
            ["activation-token", "desktop-startup-id"].map(|key| {
                (
                    Value::Str(key.to_owned()),
                    Value::Variant(Box::new(Value::Str(token.clone()))),
                )
            })
        })
        .collect();
    let platform_data = Value::Dict {
        key: signature("s"),
        val: signature("v"),
        entries,
    };
    Some(MethodCall::new(name, path, APPLICATION, "Activate").values(&[platform_data]))
}

fn signature(s: &str) -> Signature {
    Signature::new(s).expect("a valid signature")
}

fn text(s: &str) -> Value {
    Value::Str(s.to_owned())
}

fn strings(items: Vec<String>) -> Value {
    Value::Array {
        elem: signature("s"),
        items: items.into_iter().map(Value::Str).collect(),
    }
}

/// The session's target, which an app's unit is part of, so it stops with
/// the session.
fn session_target() -> Value {
    strings(vec!["graphical-session.target".to_owned()])
}

/// `StartTransientUnit` for `entry`'s unit `name`: `properties`, and those
/// every app's unit has.
fn transient_unit(
    name: &str,
    entry: &DesktopApp,
    mut properties: Vec<(&str, Value)>,
) -> MethodCall {
    properties.extend([
        ("Description", text(&entry.name)),
        ("CollectMode", text("inactive-or-failed")),
        ("Slice", text("app.slice")),
        // Stopped with the session.
        ("PartOf", session_target()),
    ]);
    if let Some(path) = entry.path.to_str() {
        properties.push(("SourcePath", text(path)));
    }
    let aux: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
    MethodCall::new(SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER, "StartTransientUnit")
        .body(&(name, "fail", properties, aux))
}

/// `StartTransientUnit` for scope `name` holding `pid`, the app `entry`
/// that the bus runs among its own services.
fn scope_call(entry: &DesktopApp, pid: u32, name: &str) -> MethodCall {
    let pids = Value::Array {
        elem: signature("u"),
        items: vec![Value::U32(pid)],
    };
    transient_unit(name, entry, vec![("PIDs", pids)])
}

/// `StartTransientUnit` for the app's service `name`. `None` when its
/// program is not found on `PATH`.
fn service_call(start: &AppStart, name: &str) -> Option<MethodCall> {
    let entry = &start.entry;
    let program = executable(entry.exec.first()?)?;
    let exec = Value::Array {
        elem: signature("(sasb)"),
        items: vec![Value::Struct(vec![
            Value::Str(program),
            strings(exec_args(&entry.exec)),
            Value::Bool(false),
        ])],
    };
    let properties = vec![
        ("ExecStart", exec),
        // A program that can't be run fails the start job, rather than the
        // service after it.
        ("Type", text("exec")),
        // Running while any of its processes is: an app whose first process
        // hands over to another and exits isn't killed with it.
        ("ExitType", text("cgroup")),
        // Stopped before the rest of the session.
        ("After", session_target()),
        ("WorkingDirectory", text("~")),
        ("Environment", strings(environment(start.token.as_deref()))),
    ];
    Some(transient_unit(name, entry, properties))
}

/// `argv` for `ExecStart`: systemd expands `$` in arguments, and `$$` keeps
/// one.
fn exec_args(argv: &[String]) -> Vec<String> {
    argv.iter().map(|arg| arg.replace('$', "$$")).collect()
}

/// The variables that hand an activation token to the app.
const TOKEN_VARIABLES: [&str; 2] = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"];

/// [`TOKEN_VARIABLES`], set to `token`.
fn token_variables(token: &str) -> [(&'static str, String); 2] {
    TOKEN_VARIABLES.map(|name| (name, token.to_owned()))
}

/// The launcher's environment, as the app would have inherited it, less
/// what systemd sets for each unit and what it would refuse: names other
/// than ASCII letters, digits and `_`, or starting with a digit, and
/// anything that isn't UTF-8. With `token`, the app's activation token.
fn environment(token: Option<&str>) -> Vec<String> {
    let token = token.into_iter().flat_map(token_variables);
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(key, _)| {
            valid_name(key)
                && !UNIT_VARIABLES.contains(&key.as_str())
                && !TOKEN_VARIABLES.contains(&key.as_str())
        })
        .chain(token.map(|(key, value)| (key.to_owned(), value)))
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
    unit_name(desktop_id, '@', "service")
}

/// The scope's name, as the service's but with a dash before the random
/// part, as scopes are named.
fn scope_name(desktop_id: &str) -> String {
    unit_name(desktop_id, '-', "scope")
}

fn unit_name(desktop_id: &str, before_random: char, kind: &str) -> String {
    let id = apps::escape_unit(app_id(desktop_id));
    let random = RandomState::new().hash_one(desktop_id);
    format!("app-mechanix-{id}{before_random}{random:016x}.{kind}")
}

/// Spawn the app from the launcher. One that can't be ends its launch.
fn spawn_or_fail(app: &mut App, start: &AppStart) {
    if let Err(e) = spawn(&start.entry.exec, start.token.as_deref()) {
        tracing::warn!(id = %start.entry.id, "launch failed: {e}");
        crate::placeholder::failed(app, start.launch);
    }
}

/// Runs `argv` in its own process group with the launcher's environment,
/// and `token` handed over, and no stdio. A thread reaps the child so it
/// never lingers as a zombie.
fn spawn(argv: &[String], token: Option<&str>) -> io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .envs(token.into_iter().flat_map(token_variables))
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
        assert!(spawn(&["true".to_string()], Some("tok")).is_ok());
        assert!(spawn(&[], None).is_err());
        assert!(spawn(&["/nonexistent/app-drawer-test".to_string()], None).is_err());
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
    fn unit_names_give_back_the_app_id() {
        for name in [
            service_name("org.example.foo-bar.desktop"),
            scope_name("org.example.foo-bar.desktop"),
        ] {
            assert_eq!(
                apps::unit_app_id(&name).as_deref(),
                Some("org.example.foo-bar"),
                "{name}"
            );
        }
    }

    #[test]
    fn activation_reaches_the_app_at_its_id() {
        assert_eq!(
            application("org.gnome.Nautilus.desktop"),
            ("org.gnome.Nautilus".into(), "/org/gnome/Nautilus".into())
        );
        assert_eq!(
            application("org.example.foo-bar.desktop"),
            ("org.example.foo-bar".into(), "/org/example/foo_bar".into())
        );
    }

    const NOTES: &str = "org.example.Notes.desktop";

    fn entry(id: &str) -> DesktopApp {
        DesktopApp {
            id: id.into(),
            name: "Notes".into(),
            exec: vec!["sh".into()],
            icon: None,
            startup_wm_class: None,
            path: "/usr/share/applications/notes.desktop".into(),
            dbus_activatable: true,
        }
    }

    /// An app with the start's bookkeeping, and the start of app `id`, its
    /// launch going on.
    fn starting_app(id: &str) -> (App, AppStart) {
        let mut app = App::new();
        app.init_resource::<Starting>();
        let mut launches = Launches::default();
        let launch = launches.add(entry(id), apps::RowIcon::Glyph(SpriteId(0)));
        app.insert_resource(launches);
        let start = AppStart {
            entry: entry(id),
            launch,
            token: None,
        };
        (app, start)
    }

    #[test]
    fn calls_build_with_and_without_a_token() {
        // Building a call encodes its body, which checks every value's type.
        for token in [None, Some("tok")] {
            let start = AppStart {
                token: token.map(String::from),
                ..starting_app(NOTES).1
            };
            assert!(activate_call(&start).is_some());
            assert!(service_call(&start, &service_name(&start.entry.id)).is_some());
            let _ = scope_call(&start.entry, 42, &scope_name(&start.entry.id));
        }
        // Not a bus name: no activation, the service instead.
        assert!(activate_call(&starting_app("kde-notes.desktop").1).is_none());
    }

    fn units(app: &App) -> usize {
        app.resource::<Starting>().units.len()
    }

    #[test]
    fn a_unit_is_kept_only_while_its_launch_goes_on() {
        let (mut app, start) = starting_app(NOTES);
        let launch = start.launch;
        started_in(&mut app, launch, "app-a.service".into());
        assert_eq!(units(&app), 1);
        forget(&mut app, launch);
        assert_eq!(units(&app), 0);

        // Learned after the launch ended: its window came first.
        app.resource_mut::<Launches>().remove(launch);
        started_in(&mut app, launch, "app-a.service".into());
        assert_eq!(units(&app), 0);
    }

    /// A process in a unit that isn't the app's (here the test's own, with
    /// the bus's own unit unknown) is left alone: no unit to stop.
    #[test]
    fn a_unit_not_named_for_the_app_is_never_its_own() {
        let (mut app, start) = starting_app(NOTES);
        adopt(&mut app, std::process::id(), &start);
        assert_eq!(units(&app), 0);
    }

    #[test]
    fn a_stop_waits_for_a_unit_only_while_one_may_come() {
        let (mut app, start) = starting_app(NOTES);
        let launch = start.launch;
        // Nothing in flight: nothing to wait for.
        stop(&mut app, launch);
        assert_eq!(units(&app), 0);

        // A scope on its way: the stop waits for it, then stops it.
        let job = ObjectPath::new("/org/freedesktop/systemd1/job/1").unwrap();
        let scope = "app-mechanix-a-1.scope".to_owned();
        app.resource_mut::<Starting>()
            .jobs
            .push((job, Job::Scope(launch, scope.clone())));
        stop(&mut app, launch);
        assert!(matches!(
            app.resource::<Starting>().units.get(&launch),
            Some(Unit::StopWhenKnown)
        ));
        no_unit(&mut app, launch);
        assert_eq!(units(&app), 1, "the scope may still come");
        app.resource_mut::<Starting>().jobs.clear();
        started_in(&mut app, launch, scope);
        assert_eq!(units(&app), 0, "stopped once known");

        // A stop waiting for a scope that failed instead: done with.
        app.resource_mut::<Starting>()
            .units
            .insert(launch, Unit::StopWhenKnown);
        no_unit(&mut app, launch);
        assert_eq!(units(&app), 0);
    }

    #[test]
    fn programs_are_found_on_path() {
        let sh = executable("sh").expect("sh is on PATH");
        assert!(sh.starts_with('/') && sh.ends_with("/sh"), "{sh}");
        assert_eq!(executable("/bin/sh").as_deref(), Some("/bin/sh"));
        assert_eq!(executable("no-such-program-for-app-drawer"), None);
    }

    #[test]
    fn the_units_own_variables_stay_behind_and_the_token_goes() {
        let env = environment(None);
        assert!(env.iter().any(|v| v.starts_with("PATH=")));
        for name in UNIT_VARIABLES {
            assert!(!env.iter().any(|v| v.starts_with(&format!("{name}="))));
        }
        assert!(!env.iter().any(|v| v.starts_with("XDG_ACTIVATION_TOKEN=")));
        let env = environment(Some("tok"));
        assert!(env.contains(&"XDG_ACTIVATION_TOKEN=tok".to_owned()));
        assert!(env.contains(&"DESKTOP_STARTUP_ID=tok".to_owned()));
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
