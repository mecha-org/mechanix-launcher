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
    /// The windows' app id, lowercased.
    AppId(String),
    /// A window with no app id: an app of its own.
    Window(ToplevelId),
}

/// One running app: the windows that share an app id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    pub key: AppKey,
    /// Index into the entries slice given to [`group`], when one matches.
    pub entry: Option<usize>,
    pub windows: Vec<ToplevelId>,
    /// The window to activate: the most recently activated one.
    pub front: ToplevelId,
    /// The highest `last_activated` of the windows.
    pub last_used: u64,
}

/// Running apps from the open windows, most recently used first. Windows
/// with a parent (dialogs) belong to their parent's app and are skipped.
pub fn group(toplevels: &[ToplevelInfo], entries: &[DesktopApp]) -> Vec<RunningApp> {
    let mut apps: Vec<RunningApp> = Vec::new();
    let mut index: HashMap<AppKey, usize> = HashMap::new();
    for t in toplevels.iter().filter(|t| t.parent.is_none()) {
        let key = if t.app_id.is_empty() {
            AppKey::Window(t.id)
        } else {
            AppKey::AppId(t.app_id.to_lowercase())
        };
        match index.get(&key) {
            Some(&i) => {
                let app = &mut apps[i];
                app.windows.push(t.id);
                if t.last_activated > app.last_used {
                    app.last_used = t.last_activated;
                    app.front = t.id;
                }
            }
            None => {
                let entry = match &key {
                    AppKey::AppId(id) => match_entry(id, entries),
                    AppKey::Window(_) => None,
                };
                index.insert(key.clone(), apps.len());
                apps.push(RunningApp {
                    key,
                    entry,
                    windows: vec![t.id],
                    front: t.id,
                    last_used: t.last_activated,
                });
            }
        }
    }
    // Stable, so equally recent apps keep the order they were first seen in.
    apps.sort_by_key(|a| Reverse(a.last_used));
    apps
}

/// The entry an app id belongs to: by desktop id, then `StartupWMClass`,
/// then the basename of the command, all case-insensitively.
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
    let rules: [&dyn Fn(&DesktopApp) -> bool; 3] = [&by_id, &by_class, &by_exec];
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

    /// Rescans the entries; whether they changed.
    fn rescan(&mut self) -> bool {
        let entries = scan(&self.data_dirs);
        let changed = entries != self.entries;
        self.entries = entries;
        changed
    }

    fn regroup(&mut self, toplevels: &Toplevels) {
        let infos: Vec<ToplevelInfo> = toplevels.iter().cloned().collect();
        self.apps = group(&infos, &self.entries);
    }
}

/// Signalled when [`Registry`] changed: the running apps differ (a window
/// came, went or was activated; a title-only change does not count), or a
/// rescan found different installed apps.
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
            parent: None,
            last_activated,
        }
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

    #[test]
    fn windows_group_by_app_id_case_insensitively() {
        let apps = group(&[win(1, "Org.Mecha.Notes", 1), win(2, "org.mecha.notes", 2)], &[]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].key, AppKey::AppId("org.mecha.notes".into()));
        assert_eq!(apps[0].windows, [id(1), id(2)]);
    }

    #[test]
    fn child_windows_never_make_cards() {
        let mut dialog = win(2, "notes", 5);
        dialog.parent = Some(id(1));
        assert!(group(&[dialog.clone()], &[]).is_empty());
        let apps = group(&[win(1, "notes", 1), dialog], &[]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].windows, [id(1)]);
    }

    #[test]
    fn empty_app_id_is_its_own_app() {
        let apps = group(&[win(1, "", 1), win(2, "", 2)], &[]);
        assert_eq!(apps.len(), 2);
        assert!(apps.iter().any(|a| a.key == AppKey::Window(id(1))));
        assert!(apps.iter().any(|a| a.key == AppKey::Window(id(2))));
        assert!(apps.iter().all(|a| a.entry.is_none()));
    }

    #[test]
    fn front_is_most_recently_activated() {
        let apps = group(&[win(1, "a", 3), win(2, "a", 7), win(3, "a", 7)], &[]);
        assert_eq!(apps[0].front, id(2));
        assert_eq!(apps[0].last_used, 7);
    }

    #[test]
    fn apps_order_most_recent_first() {
        let apps = group(
            &[win(1, "a", 1), win(2, "b", 9), win(3, "c", 1), win(4, "a", 4)],
            &[],
        );
        let keys: Vec<_> = apps.iter().map(|a| a.key.clone()).collect();
        assert_eq!(
            keys,
            [
                AppKey::AppId("b".into()),
                AppKey::AppId("a".into()),
                AppKey::AppId("c".into())
            ]
        );
    }

    #[test]
    fn grouped_app_carries_its_entry() {
        let entries = [entry("foot.desktop", "foot", None)];
        assert_eq!(group(&[win(1, "foot", 1)], &entries)[0].entry, Some(0));
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
