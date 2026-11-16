//! Groups the compositor's open windows into running apps.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mecha_wayland::prelude::*;
use screen::Shown;

use crate::{DesktopApp, scan};

/// What identifies a running app.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppKey {
    /// An installed app: its desktop entry's id.
    Entry(String),
    /// A process whose windows match no entry: its windows go together.
    Process(u32),
    /// A window with nothing else to go by: an app of its own.
    Window(ToplevelId),
}

/// One running app: its windows, and the dialogs over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    pub key: AppKey,
    /// Index into the entries slice given to [`group`], when one matches.
    pub entry: Option<usize>,
    pub windows: Vec<ToplevelId>,
    /// The dialogs open over its windows, a dialog's own dialogs included.
    pub dialogs: Vec<ToplevelId>,
    /// Whether one of its dialogs is modal.
    pub modal: bool,
    /// The processes its windows and dialogs belong to, where known.
    pub pids: Vec<u32>,
    /// The window to activate: the most recently activated one.
    pub front: ToplevelId,
    /// The highest `last_activated` of the windows.
    pub last_used: u64,
    /// For an app without an entry, what there is to name it by: its first
    /// window's app id, or else its title. Empty for an entry, so a title
    /// change is not a change to the running apps.
    pub title: String,
}

/// Running apps from the open windows, most recently used first. A window
/// belongs to, in order:
///
/// 1. its parent's app, if it has a parent: it is one of its dialogs;
/// 2. the entry its app id matches ([`match_entry`]);
/// 3. the entry the app id of the systemd unit it runs in matches
///    (`scope_app`, from its pid; see [`unit_app_id`]);
/// 4. the app of another window of its process;
/// 5. its process, or itself when its pid is unknown.
///
/// An app id that matches no entry groups nothing: unrelated apps often
/// share one (`python3`, `electron`).
pub fn group(
    toplevels: &[ToplevelInfo],
    entries: &[DesktopApp],
    scope_app: impl Fn(u32) -> Option<String>,
) -> Vec<RunningApp> {
    // Each window without a parent, with the entry it matches itself.
    let tops: Vec<(&ToplevelInfo, Option<usize>)> = toplevels
        .iter()
        .filter(|t| t.parent.is_none())
        .map(|t| {
            let entry = Some(t.app_id.as_str())
                .filter(|id| !id.is_empty())
                .and_then(|id| match_entry(id, entries))
                .or_else(|| match_entry(&scope_app(t.pid?)?, entries));
            (t, entry)
        })
        .collect();
    let by_pid: HashMap<u32, usize> = tops
        .iter()
        .filter_map(|&(t, entry)| Some((t.pid?, entry?)))
        .collect();
    let mut apps: Vec<RunningApp> = Vec::new();
    let mut index: HashMap<AppKey, usize> = HashMap::new();
    for (t, entry) in tops {
        let entry = entry.or_else(|| by_pid.get(&t.pid?).copied());
        let key = match (entry, t.pid) {
            (Some(i), _) => AppKey::Entry(entries[i].id.clone()),
            (None, Some(pid)) => AppKey::Process(pid),
            (None, None) => AppKey::Window(t.id),
        };
        match index.get(&key) {
            Some(&i) => {
                let app = &mut apps[i];
                app.windows.push(t.id);
                app.pids.extend(t.pid.filter(|p| !app.pids.contains(p)));
                if t.last_activated > app.last_used {
                    app.last_used = t.last_activated;
                    app.front = t.id;
                }
            }
            None => {
                let title = match (entry, t.app_id.is_empty()) {
                    (Some(_), _) => String::new(),
                    (None, false) => t.app_id.clone(),
                    (None, true) => t.title.clone(),
                };
                index.insert(key.clone(), apps.len());
                apps.push(RunningApp {
                    key,
                    entry,
                    windows: vec![t.id],
                    dialogs: Vec::new(),
                    modal: false,
                    pids: t.pid.into_iter().collect(),
                    front: t.id,
                    last_used: t.last_activated,
                    title,
                });
            }
        }
    }
    let parent_of: HashMap<ToplevelId, ToplevelId> = toplevels
        .iter()
        .filter_map(|t| Some((t.id, t.parent?)))
        .collect();
    for t in toplevels.iter().filter(|t| t.parent.is_some()) {
        // Up to the window without a parent; bounded, in case of a cycle.
        let mut top = t.id;
        for _ in 0..toplevels.len() {
            match parent_of.get(&top) {
                Some(&p) => top = p,
                None => break,
            }
        }
        if let Some(app) = apps.iter_mut().find(|a| a.windows.contains(&top)) {
            app.dialogs.push(t.id);
            app.modal |= t.modal;
            app.pids.extend(t.pid.filter(|p| !app.pids.contains(p)));
        }
    }
    // Stable, so equally recent apps keep the order they were first seen in.
    apps.sort_by_key(|a| Reverse(a.last_used));
    apps
}

/// The app id in the name of the systemd unit `pid` runs in, when that is
/// an app's unit; see [`unit_app_id`].
pub fn scope_app_id(pid: u32) -> Option<String> {
    unit_app_id(&unit_of(pid)?)
}

/// The systemd unit `pid` runs in, when that is an app's own unit (see
/// [`unit_app_id`]): ending that unit ends the app, and nothing else.
pub fn app_unit(pid: u32) -> Option<String> {
    unit_of(pid).filter(|unit| unit_app_id(unit).is_some())
}

/// The last part of `pid`'s cgroup path: its unit, if systemd made it.
fn unit_of(pid: u32) -> Option<String> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(path.rsplit('/').next()?.to_owned())
}

/// The app id in an app's unit name, as freedesktop names them and the
/// portals read them: `app[-<launcher>]-<id>-<random>.scope`, or
/// `app[-<launcher>]-<id>[@<random>|-autostart].service`. The id's own
/// dashes are escaped (`\x2d`), so a dash ends it.
///
/// Also the units dbus-broker starts an activated app in,
/// `dbus-<its own connection>-<bus name>@<n>.service`: an app that takes
/// D-Bus activation has its desktop id as its bus name.
pub fn unit_app_id(unit: &str) -> Option<String> {
    if let Some(rest) = unit.strip_prefix("dbus-:") {
        let (_connection, named) = rest.strip_suffix(".service")?.split_once('-')?;
        let (name, _instance) = named.split_once('@')?;
        return unescape_unit(name).filter(|id| !id.is_empty());
    }
    let rest = unit.strip_prefix("app-")?;
    let named = if let Some(scope) = rest.strip_suffix(".scope") {
        scope.rsplit_once('-')?.0
    } else {
        let service = rest.strip_suffix(".service")?;
        let service = service.split_once('@').map_or(service, |(id, _)| id);
        service.strip_suffix("-autostart").unwrap_or(service)
    };
    // The launcher, when there is one, is a plain word.
    let id = match named.split_once('-') {
        Some((launcher, id)) if launcher.chars().all(|c| c.is_ascii_alphanumeric()) => id,
        _ => named,
    };
    unescape_unit(id).filter(|id| !id.is_empty())
}

/// Undo systemd's `\xNN` escaping of a unit name.
fn unescape_unit(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("\\x") {
        out.extend_from_slice(&rest.as_bytes()[..at]);
        let hex = rest.get(at + 2..at + 4)?;
        out.push(u8::from_str_radix(hex, 16).ok()?);
        rest = &rest[at + 4..];
    }
    out.extend_from_slice(rest.as_bytes());
    String::from_utf8(out).ok()
}

/// The entry an app id belongs to: by desktop id, then `StartupWMClass`,
/// then the basename of the command, then (for a reverse-DNS id) by its
/// last part as the desktop id, all case-insensitively.
pub fn match_entry(app_id: &str, entries: &[DesktopApp]) -> Option<usize> {
    let want = app_id.to_lowercase();
    let by_id = |e: &DesktopApp| {
        let id = e.id.to_lowercase();
        id.strip_suffix(".desktop").unwrap_or(&id) == want
    };
    let by_class = |e: &DesktopApp| {
        e.startup_wm_class
            .as_deref()
            .is_some_and(|c| c.to_lowercase() == want)
    };
    let by_exec = |e: &DesktopApp| {
        e.exec
            .first()
            .and_then(|c| Path::new(c).file_name())
            .is_some_and(|n| n.to_string_lossy().to_lowercase() == want)
    };
    let last = want.rsplit_once('.').map(|(_, last)| last);
    let by_last = |e: &DesktopApp| {
        let id = e.id.to_lowercase();
        last.is_some_and(|last| id.strip_suffix(".desktop").unwrap_or(&id) == last)
    };
    let rules: [&dyn Fn(&DesktopApp) -> bool; 4] = [&by_id, &by_class, &by_exec, &by_last];
    rules
        .iter()
        .find_map(|rule| entries.iter().position(|e| rule(e)))
}

/// The installed apps and the running ones. Rebuilt by [`RegistryModule`].
pub struct Registry {
    data_dirs: Vec<PathBuf>,
    entries: Vec<DesktopApp>,
    apps: Vec<RunningApp>,
}

impl Resource for Registry {}

impl Registry {
    /// Running apps, most recently used first.
    pub fn apps(&self) -> &[RunningApp] {
        &self.apps
    }

    /// Installed apps; [`RunningApp::entry`] indexes this.
    pub fn entries(&self) -> &[DesktopApp] {
        &self.entries
    }

    /// The running app of `entries()[entry]`, if it is running.
    pub fn running_entry(&self, entry: usize) -> Option<&RunningApp> {
        self.apps.iter().find(|a| a.entry == Some(entry))
    }

    /// A registry holding exactly `entries` and `apps`, for tests of
    /// consumers that need no compositor or scan.
    #[doc(hidden)]
    pub fn fixture(entries: Vec<DesktopApp>, apps: Vec<RunningApp>) -> Registry {
        Registry {
            data_dirs: Vec::new(),
            entries,
            apps,
        }
    }

    /// Rescans the entries; whether they changed.
    fn rescan(&mut self) -> bool {
        let entries = scan(&self.data_dirs);
        let changed = entries != self.entries;
        self.entries = entries;
        changed
    }

    fn regroup(&mut self, toplevels: &Toplevels) {
        let infos: Vec<ToplevelInfo> = toplevels.iter().cloned().collect();
        self.apps = group(&infos, &self.entries, scope_app_id);
    }
}

/// Signalled when [`Registry`] changed: the running apps differ (a window
/// came, went or was activated, or a window with no app id was renamed;
/// other title changes do not count), or a rescan found different
/// installed apps.
pub struct RegistryChanged;
impl Signal for RegistryChanged {}

/// Keeps a [`Registry`] current. Install after `ToplevelsModule`.
///
/// Entries are rescanned whenever any screen is `Shown` (the app-wide
/// `Emitted<Shown>`), so an app installed meanwhile is found by the time a
/// screen that lists apps appears.
pub struct RegistryModule {
    pub data_dirs: Vec<PathBuf>,
}

impl Module for RegistryModule {
    fn install(self, app: &mut App) {
        let mut registry = Registry {
            entries: scan(&self.data_dirs),
            data_dirs: self.data_dirs,
            apps: Vec::new(),
        };
        if app.has_resource::<Toplevels>() {
            registry.regroup(&app.resource::<Toplevels>());
        }
        app.insert_resource(registry);
        app.system(on_toplevels_changed).system(on_shown);
    }
}

fn on_toplevels_changed(app: &mut App, _: &ToplevelsChanged) {
    if !app.has_resource::<Toplevels>() {
        return;
    }
    let changed = {
        let (toplevels, mut registry) = app.query::<(Res<Toplevels>, ResMut<Registry>)>();
        let before = std::mem::take(&mut registry.apps);
        registry.regroup(&toplevels);
        registry.apps != before
    };
    if changed {
        app.signal(RegistryChanged);
    }
}

fn on_shown(app: &mut App, _: &Emitted<Shown>) {
    let changed = if app.has_resource::<Toplevels>() {
        let (toplevels, mut registry) = app.query::<(Res<Toplevels>, ResMut<Registry>)>();
        let changed = registry.rescan();
        if changed {
            registry.regroup(&toplevels);
        }
        changed
    } else {
        app.resource_mut::<Registry>().rescan()
    };
    if changed {
        app.signal(RegistryChanged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> ToplevelId {
        ToplevelId::from_raw(n)
    }

    fn win(n: u32, app_id: &str, last_activated: u64) -> ToplevelInfo {
        ToplevelInfo {
            id: id(n),
            title: String::new(),
            app_id: app_id.into(),
            activated: false,
            fullscreen: false,
            parent: None,
            pid: None,
            modal: false,
            last_activated,
        }
    }

    fn in_process(mut w: ToplevelInfo, pid: u32) -> ToplevelInfo {
        w.pid = Some(pid);
        w
    }

    fn entry(id: &str, exec: &str, wm_class: Option<&str>) -> DesktopApp {
        DesktopApp {
            id: id.into(),
            name: id.into(),
            exec: exec.split(' ').map(String::from).collect(),
            icon: None,
            startup_wm_class: wm_class.map(String::from),
        }
    }

    /// Entries for the app ids the tests use.
    fn installed() -> Vec<DesktopApp> {
        ["a", "b", "c", "notes", "clock", "org.mecha.notes"]
            .into_iter()
            .map(|id| entry(&format!("{id}.desktop"), id, None))
            .collect()
    }

    fn key(id: &str) -> AppKey {
        AppKey::Entry(format!("{id}.desktop"))
    }

    /// Group with no unit to look up.
    fn group_plain(toplevels: &[ToplevelInfo], entries: &[DesktopApp]) -> Vec<RunningApp> {
        group(toplevels, entries, |_| None)
    }

    #[test]
    fn windows_of_one_entry_group_whatever_the_case() {
        let apps = group_plain(
            &[win(1, "Org.Mecha.Notes", 1), win(2, "org.mecha.notes", 2)],
            &installed(),
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].key, key("org.mecha.notes"));
        assert_eq!(apps[0].windows, [id(1), id(2)]);
    }

    #[test]
    fn an_app_id_without_an_entry_groups_nothing() {
        // Two unrelated apps both calling themselves `python3`.
        let apps = group_plain(
            &[
                in_process(win(1, "python3", 1), 10),
                in_process(win(2, "python3", 2), 20),
                win(3, "python3", 3),
            ],
            &installed(),
        );
        let keys: Vec<_> = apps.iter().map(|a| a.key.clone()).collect();
        assert_eq!(
            keys,
            [
                AppKey::Window(id(3)),
                AppKey::Process(20),
                AppKey::Process(10)
            ]
        );
        assert!(apps.iter().all(|a| a.title == "python3"));
    }

    #[test]
    fn one_process_without_an_entry_is_one_app() {
        let apps = group_plain(
            &[
                in_process(win(1, "tool", 1), 10),
                in_process(win(2, "", 2), 10),
            ],
            &installed(),
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].key, AppKey::Process(10));
        assert_eq!(apps[0].title, "tool", "named by its first window");
    }

    #[test]
    fn the_unit_names_the_app_when_the_app_id_does_not() {
        let scope = |pid| (pid == 10).then(|| "notes".to_string());
        let apps = group(
            &[
                in_process(win(1, "python3", 1), 10),
                in_process(win(2, "python3", 2), 20),
            ],
            &installed(),
            scope,
        );
        let notes = apps.iter().find(|a| a.key == key("notes")).unwrap();
        assert_eq!(notes.windows, [id(1)]);
        assert!(apps.iter().any(|a| a.key == AppKey::Process(20)));
    }

    #[test]
    fn the_app_id_outranks_the_unit() {
        // An app started from a terminal runs in the terminal's unit.
        let terminal = |_| Some("a".to_string());
        let apps = group(
            &[in_process(win(1, "clock", 1), 10)],
            &installed(),
            terminal,
        );
        assert_eq!(apps[0].key, key("clock"));
    }

    #[test]
    fn a_process_goes_with_its_known_window() {
        let apps = group_plain(
            &[
                in_process(win(1, "notes", 1), 10),
                in_process(win(2, "helper", 2), 10),
            ],
            &installed(),
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].key, key("notes"));
        assert_eq!(apps[0].windows, [id(1), id(2)]);
    }

    #[test]
    fn child_windows_never_make_cards() {
        let mut dialog = win(2, "notes", 5);
        dialog.parent = Some(id(1));
        assert!(group_plain(&[dialog.clone()], &installed()).is_empty());
        let apps = group_plain(&[win(1, "notes", 1), dialog], &installed());
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].windows, [id(1)]);
    }

    #[test]
    fn dialogs_go_with_their_windows_app_however_deep() {
        let mut dialog = win(2, "", 5);
        dialog.parent = Some(id(1));
        let mut inner = win(3, "other", 6);
        inner.parent = Some(id(2));
        inner.modal = true;
        let apps = group_plain(
            &[win(1, "notes", 1), dialog, inner, win(4, "clock", 2)],
            &installed(),
        );
        assert_eq!(apps.len(), 2);
        let notes = apps.iter().find(|a| a.windows == [id(1)]).unwrap();
        assert_eq!(notes.dialogs, [id(2), id(3)]);
        assert!(notes.modal);
        let clock = apps.iter().find(|a| a.windows == [id(4)]).unwrap();
        assert!(clock.dialogs.is_empty());
        assert!(!clock.modal);
    }

    #[test]
    fn empty_app_id_is_its_own_app() {
        let apps = group_plain(&[win(1, "", 1), win(2, "", 2)], &installed());
        assert_eq!(apps.len(), 2);
        assert!(apps.iter().any(|a| a.key == AppKey::Window(id(1))));
        assert!(apps.iter().any(|a| a.key == AppKey::Window(id(2))));
        assert!(apps.iter().all(|a| a.entry.is_none()));
    }

    #[test]
    fn window_keyed_app_carries_its_title_and_a_rename_is_a_change() {
        let mut lone = win(1, "", 1);
        lone.title = "Scratch".into();
        let mut notes = win(2, "notes", 2);
        notes.title = "Shopping list".into();
        let apps = group_plain(&[lone.clone(), notes.clone()], &installed());
        let by_window = apps
            .iter()
            .find(|a| a.key == AppKey::Window(id(1)))
            .unwrap();
        assert_eq!(by_window.title, "Scratch");
        let by_entry = apps.iter().find(|a| a.key == key("notes")).unwrap();
        assert_eq!(by_entry.title, "", "an entry names its own card");

        // Renaming the window-keyed app's window changes the running apps;
        // renaming an entry's window does not.
        let mut renamed = lone.clone();
        renamed.title = "Renamed".into();
        assert_ne!(group_plain(&[renamed, notes.clone()], &installed()), apps);
        let mut retitled = notes;
        retitled.title = "Todo".into();
        assert_eq!(group_plain(&[lone, retitled], &installed()), apps);
    }

    #[test]
    fn front_is_most_recently_activated() {
        let apps = group_plain(
            &[win(1, "a", 3), win(2, "a", 7), win(3, "a", 7)],
            &installed(),
        );
        assert_eq!(apps[0].front, id(2));
        assert_eq!(apps[0].last_used, 7);
    }

    #[test]
    fn apps_order_most_recent_first() {
        let apps = group_plain(
            &[
                win(1, "a", 1),
                win(2, "b", 9),
                win(3, "c", 1),
                win(4, "a", 4),
            ],
            &installed(),
        );
        let keys: Vec<_> = apps.iter().map(|a| a.key.clone()).collect();
        assert_eq!(keys, [key("b"), key("a"), key("c")]);
    }

    #[test]
    fn grouped_app_carries_its_entry() {
        let entries = [entry("foot.desktop", "foot", None)];
        assert_eq!(
            group_plain(&[win(1, "foot", 1)], &entries)[0].entry,
            Some(0)
        );
    }

    #[test]
    fn unit_names_give_the_app_id() {
        let cases = [
            (
                "app-mechanix-org.gnome.Nautilus-42.scope",
                Some("org.gnome.Nautilus"),
            ),
            (
                "app-gnome-org.gnome.Evolution\\x2dalarm\\x2dnotify-2437.scope",
                Some("org.gnome.Evolution-alarm-notify"),
            ),
            ("app-firefox-1234.scope", Some("firefox")),
            ("app-org.kde.dolphin@a1b2.service", Some("org.kde.dolphin")),
            ("app-gnome-foo-autostart.service", Some("foo")),
            (
                "dbus-:1.3-org.gnome.Loupe@0.service",
                Some("org.gnome.Loupe"),
            ),
            (
                "dbus-:1.3-org.example.foo\\x2dbar@12.service",
                Some("org.example.foo-bar"),
            ),
            ("dbus-broker.service", None),
            ("mechanix-shell.service", None),
            ("session-2.scope", None),
        ];
        for (unit, want) in cases {
            assert_eq!(unit_app_id(unit).as_deref(), want, "{unit}");
        }
    }

    #[test]
    fn a_reverse_dns_id_matches_by_its_last_part() {
        let entries = [entry("calculator.desktop", "calc", None)];
        assert_eq!(match_entry("org.example.Calculator", &entries), Some(0));
    }

    #[test]
    fn match_entry_rules_in_order() {
        let entries = [
            entry("org.gnome.Nautilus.desktop", "nautilus", None),
            entry("org.mozilla.firefox.desktop", "sh -c x", Some("firefox")),
            entry("foot-server.desktop", "/usr/bin/foot --server", None),
        ];
        assert_eq!(match_entry("org.gnome.nautilus", &entries), Some(0));
        assert_eq!(match_entry("firefox", &entries), Some(1));
        assert_eq!(match_entry("foot", &entries), Some(2));
        assert_eq!(match_entry("nothing", &entries), None);
        // The desktop id outranks a command basename that also matches.
        let clash = [
            entry("a.desktop", "/bin/b", None),
            entry("b.desktop", "other", None),
        ];
        assert_eq!(match_entry("b", &clash), Some(1));
    }

    #[test]
    fn rescan_reports_only_real_changes() {
        let root = std::env::temp_dir().join(format!("apps-rescan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let mut registry = Registry {
            data_dirs: vec![root.clone()],
            entries: Vec::new(),
            apps: Vec::new(),
        };
        assert!(!registry.rescan());
        std::fs::write(
            apps.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=A\nExec=a\n",
        )
        .unwrap();
        assert!(registry.rescan());
        assert!(!registry.rescan());
        assert_eq!(registry.entries().len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
