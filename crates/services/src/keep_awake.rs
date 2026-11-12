//! Keeping the screen awake, which [`KeepAwake`] holds, and whose changes
//! are [`OnChanged<KeepAwake>`]:
//!
//! - The user asks with [`SetKeepAwake`], from quick settings.
//! - Apps that are not sandboxed (emulators, video players, browsers) ask
//!   over `org.freedesktop.ScreenSaver` on the session bus. `Inhibit`
//!   returns a cookie; `UnInhibit` with it lets go, and so does the app
//!   leaving the bus. Served at both paths apps call:
//!   `/org/freedesktop/ScreenSaver` and `/ScreenSaver`. Only the inhibit
//!   calls, `GetActive` and `ActiveChanged` are served: the screen saver
//!   is active while the session is locked ([`SessionLockChanged`]).
//!
//! FIXME: `org.freedesktop.PowerManagement.Inhibit` isn't served. An app
//! that only asks it to keep the system awake (no suspend, the screen may
//! blank) isn't seen, so the idle timer still blanks and suspends.

use mecha_wayland::dbus::{ExportId, MatchId, MethodCallReceived};
use mecha_wayland::prelude::*;

use crate::SessionLockChanged;

const NAME: &str = "org.freedesktop.ScreenSaver";
const INTERFACE: &str = "org.freedesktop.ScreenSaver";
const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

/// Who keeps the screen awake.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeepAwake {
    /// The user, from quick settings.
    pub user: bool,
    /// The apps holding inhibits, in the order they first asked.
    pub apps: Vec<AppHold>,
}
impl Resource for KeepAwake {}

impl KeepAwake {
    /// Anyone does.
    pub fn any(&self) -> bool {
        self.user || !self.apps.is_empty()
    }
}

/// One app keeping the screen awake: all its inhibits under one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppHold {
    /// The app's unique bus name: the same while it stays on the bus.
    pub owner: String,
    /// The name it gave.
    pub app: String,
    /// The reasons it gave, one per inhibit, in order.
    pub reasons: Vec<String>,
}

/// Keep the screen awake for the user (`true`), or stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetKeepAwake(pub bool);
impl Signal for SetKeepAwake {}

struct Inhibit {
    cookie: u32,
    /// The caller's unique name: its inhibits go when it leaves the bus.
    owner: String,
    app: String,
    reason: String,
}

struct ScreenSaver {
    exports: Vec<ExportId>,
    /// `NameOwnerChanged` for names that lost their owner.
    vanished: MatchId,
    next_cookie: u32,
    inhibits: Vec<Inhibit>,
    /// The session is locked: what `GetActive` says.
    active: bool,
}
impl Resource for ScreenSaver {}

/// Install after `DBusModule::new().session()`.
pub struct KeepAwakeModule;

impl Module for KeepAwakeModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<ScreenSaver>() {
            return;
        }
        let spec = || {
            InterfaceSpec::new(INTERFACE)
                .method("Inhibit", "ss", "u")
                .method("UnInhibit", "u", "")
                .method("GetActive", "", "b")
                .signal("ActiveChanged", "b")
        };
        let (exports, vanished) = {
            let mut dbus = app.resource_mut::<DBus>();
            let mut bus = dbus.session();
            bus.request_name(NAME, NameFlags::DO_NOT_QUEUE);
            let exports = PATHS
                .into_iter()
                .filter_map(|path| {
                    bus.export(path, spec())
                        .inspect_err(|e| eprintln!("[keep-awake] exporting {path}: {e:?}"))
                        .ok()
                })
                .collect();
            let vanished = bus.add_match(
                MatchRule::signal()
                    .sender("org.freedesktop.DBus")
                    .interface("org.freedesktop.DBus")
                    .member("NameOwnerChanged")
                    .arg(2, ""),
            );
            (exports, vanished)
        };
        app.init_resource::<KeepAwake>();
        app.insert_resource(ScreenSaver {
            exports,
            vanished,
            next_cookie: 1,
            inhibits: Vec::new(),
            active: false,
        });
        app.system(on_call)
            .system(on_vanished)
            .system(on_lock_changed)
            .system(|app, e: &SetKeepAwake| {
                if app.resource::<KeepAwake>().user != e.0 {
                    app.resource_mut::<KeepAwake>().user = e.0;
                }
            });
    }
}

/// Locked or unlocked: the screen saver is active or not, and says so.
fn on_lock_changed(app: &mut App, e: &SessionLockChanged) {
    if std::mem::replace(&mut app.resource_mut::<ScreenSaver>().active, e.0) == e.0 {
        return;
    }
    let mut dbus = app.resource_mut::<DBus>();
    let mut bus = dbus.session();
    for path in PATHS {
        bus.emit_signal(path, INTERFACE, "ActiveChanged", &(e.0,));
    }
}

fn on_call(app: &mut App, e: &MethodCallReceived) {
    if !app.resource::<ScreenSaver>().exports.contains(&e.export) {
        return;
    }
    match e.call.member().as_str() {
        "Inhibit" => {
            let Ok((name, reason)) = e.call.args::<(String, String)>() else {
                return;
            };
            let owner = e.call.sender().map(|s| s.as_str().to_owned());
            let cookie = {
                let mut ss = app.resource_mut::<ScreenSaver>();
                let cookie = ss.next_cookie;
                ss.next_cookie = cookie.checked_add(1).unwrap_or(1);
                ss.inhibits.push(Inhibit {
                    cookie,
                    owner: owner.unwrap_or_default(),
                    app: name,
                    reason,
                });
                cookie
            };
            app.resource_mut::<DBus>().reply(&e.call, &(cookie,));
            update_apps(app);
        }
        "UnInhibit" => {
            let Ok((cookie,)) = e.call.args::<(u32,)>() else {
                return;
            };
            app.resource_mut::<ScreenSaver>()
                .inhibits
                .retain(|i| i.cookie != cookie);
            app.resource_mut::<DBus>().reply(&e.call, &());
            update_apps(app);
        }
        "GetActive" => {
            let active = app.resource::<ScreenSaver>().active;
            app.resource_mut::<DBus>().reply(&e.call, &(active,));
        }
        _ => {}
    }
}

/// An app left the bus: its inhibits go with it.
fn on_vanished(app: &mut App, e: &SignalReceived) {
    if e.bus != BusId::Session || !e.matches.contains(&app.resource::<ScreenSaver>().vanished) {
        return;
    }
    let Ok((name, _, _)) = e.msg.body::<(String, String, String)>() else {
        return;
    };
    app.resource_mut::<ScreenSaver>()
        .inhibits
        .retain(|i| i.owner != name);
    update_apps(app);
}

/// Group the inhibits by app into [`KeepAwake::apps`], if that changed.
fn update_apps(app: &mut App) {
    let mut apps: Vec<AppHold> = Vec::new();
    for i in &app.resource::<ScreenSaver>().inhibits {
        match apps
            .iter_mut()
            .find(|h| h.owner == i.owner && h.app == i.app)
        {
            Some(h) => h.reasons.push(i.reason.clone()),
            None => apps.push(AppHold {
                owner: i.owner.clone(),
                app: i.app.clone(),
                reasons: vec![i.reason.clone()],
            }),
        }
    }
    if app.resource::<KeepAwake>().apps != apps {
        app.resource_mut::<KeepAwake>().apps = apps;
    }
}

#[cfg(test)]
mod tests {
    use dbus::fake::FakeBus;

    use super::*;

    fn call(sender: &'static str, path: &'static str, member: &'static str) -> Message {
        Message::method_call(path, member)
            .with_interface(INTERFACE)
            .with_destination(NAME)
            .with_sender(sender)
    }

    fn cookie(bus: &mut FakeBus) -> u32 {
        bus.turn();
        let (cookie,) = bus
            .expect(MessageKind::MethodReturn)
            .body::<(u32,)>()
            .unwrap();
        cookie
    }

    /// The apps keeping the screen awake, as (app, reasons) pairs.
    fn apps(bus: &FakeBus) -> Vec<(&str, Vec<&str>)> {
        let ka = bus.app.resource::<KeepAwake>();
        ka.apps
            .iter()
            .map(|h| {
                (
                    h.app.as_str(),
                    h.reasons.iter().map(String::as_str).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn inhibits_hold_until_let_go_or_their_app_leaves() {
        let mut bus = FakeBus::new();
        bus.app.add_module(KeepAwakeModule);
        bus.turn();
        bus.received();

        bus.send(call(":1.7", PATHS[0], "Inhibit").with_body(&("ARMSX2", "Game")));
        let first = cookie(&mut bus);
        assert_eq!(apps(&bus), [("ARMSX2", vec!["Game"])]);

        // The other path, another app, asking twice: one holder each.
        bus.send(call(":1.8", PATHS[1], "Inhibit").with_body(&("mpv", "Video")));
        let second = cookie(&mut bus);
        assert_ne!(first, second);
        bus.send(call(":1.8", PATHS[1], "Inhibit").with_body(&("mpv", "Audio")));
        cookie(&mut bus);
        assert_eq!(
            apps(&bus),
            [("ARMSX2", vec!["Game"]), ("mpv", vec!["Video", "Audio"])]
        );

        bus.send(call(":1.7", PATHS[0], "UnInhibit").with_body(&(first,)));
        bus.turn();
        bus.expect(MessageKind::MethodReturn);
        assert_eq!(apps(&bus), [("mpv", vec!["Video", "Audio"])]);

        // mpv leaves the bus without letting go.
        bus.send(
            Message::signal(
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameOwnerChanged",
            )
            .with_sender("org.freedesktop.DBus")
            .with_body(&(":1.8", ":1.8", "")),
        );
        bus.turn();
        assert_eq!(apps(&bus), []);
        assert!(!bus.app.resource::<KeepAwake>().any());

        bus.send(call(":1.9", PATHS[0], "GetActive"));
        bus.turn();
        let (active,) = bus
            .expect(MessageKind::MethodReturn)
            .body::<(bool,)>()
            .unwrap();
        assert!(!active);
    }

    #[test]
    fn the_user_keeps_it_awake_too() {
        let mut bus = FakeBus::new();
        bus.app.add_module(KeepAwakeModule);
        bus.app.signal(SetKeepAwake(true));
        bus.turn();
        let ka = bus.app.resource::<KeepAwake>();
        assert!(ka.user && ka.any());
    }

    /// Every `ActiveChanged` since the last call, as (path, active).
    fn active_changes(bus: &mut FakeBus) -> Vec<(String, bool)> {
        bus.received()
            .into_iter()
            .filter(|m| {
                m.kind() == MessageKind::Signal
                    && m.member().is_some_and(|n| n.as_str() == "ActiveChanged")
            })
            .map(|m| {
                let path = m.path().unwrap().as_str().to_owned();
                (path, m.body::<(bool,)>().unwrap().0)
            })
            .collect()
    }

    fn get_active(bus: &mut FakeBus) -> bool {
        bus.send(call(":1.9", PATHS[0], "GetActive"));
        bus.turn();
        bus.expect(MessageKind::MethodReturn)
            .body::<(bool,)>()
            .unwrap()
            .0
    }

    #[test]
    fn it_is_active_while_the_session_is_locked() {
        let mut bus = FakeBus::new();
        bus.app.add_module(KeepAwakeModule);
        bus.turn();
        bus.received();

        bus.app.signal(SessionLockChanged(true));
        bus.turn();
        let both = |active| PATHS.map(|p| (p.to_owned(), active)).to_vec();
        assert_eq!(active_changes(&mut bus), both(true));
        assert!(get_active(&mut bus));

        // Already locked: nothing to say.
        bus.app.signal(SessionLockChanged(true));
        bus.turn();
        assert!(active_changes(&mut bus).is_empty());

        bus.app.signal(SessionLockChanged(false));
        bus.turn();
        assert_eq!(active_changes(&mut bus), both(false));
        assert!(!get_active(&mut bus));
    }
}
