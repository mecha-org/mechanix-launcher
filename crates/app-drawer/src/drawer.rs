//! The drawer: a search bar over a scrolling list of every installed app.

use std::path::PathBuf;
use std::rc::Rc;

use mecha_wayland::prelude::*;
use screen::{AppLaunched, Hidden, Shown};

use crate::rank::{Hit, Ranker, fit};
use crate::row::{GAP, NAME_PX, RowBuilder, SIDE, Tapped};
use crate::search_bar::SearchBar;
use crate::{Activator, Launcher};
use apps::{
    DesktopApp, IconCache, IconDirs, Launches, Registry, RegistryChanged, RowIcon, ShowLaunch, scan,
};

/// Geist Mono advances 600 of its 1000 units per em for every glyph.
const ADVANCE_EM: f32 = 0.6;
const ICON: f32 = 48.0;
/// Space under the last row.
const LIST_BOTTOM: f32 = 24.0;

/// The most name chars a row of `width` fits beside its icon.
fn max_chars(width: f32) -> usize {
    let room = width - SIDE * 2.0 - ICON - GAP;
    (room / (NAME_PX * ADVANCE_EM)).floor().max(0.0) as usize
}

/// Emitted at the drawer when its width gives rows a new `max_chars`.
struct Refit(usize);
impl Event for Refit {}

pub(crate) struct DrawerBuilder {
    pub data_dirs: Vec<PathBuf>,
    pub icon_dirs: Rc<IconDirs>,
}

impl Build for DrawerBuilder {
    type Widget = Drawer;
}

pub(crate) struct Drawer {
    data_dirs: Vec<PathBuf>,
    icon_dirs: Rc<IconDirs>,
    apps: Vec<DesktopApp>,
    icons: Vec<RowIcon>,
    ranker: Ranker,
    query: String,
    hits: Vec<Hit>,
    /// `None` until the first layout gives the drawer a width.
    max_chars: Option<usize>,
    focused: bool,
    bar: SearchBar,
    list: Handle<Scrollable>,
    rows: Vec<NodeId>,
}

impl Widget for Drawer {
    type Builder = DrawerBuilder;

    fn build(b: DrawerBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().column().fill();
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));
        let bar = SearchBar::spawn(s, me);
        let list = s.spawn(
            me,
            scrollable().overscroll(Overscroll::YieldStart).style(
                LayoutStyle::default()
                    .block()
                    .fill()
                    .min_height(px(0.0))
                    .padding(Insets::new(px(0.0), px(0.0), px(LIST_BOTTOM), px(0.0))),
            ),
        );
        // A tap on the list takes the focus from the search, and with it
        // the keyboard. The search bar, its clear button included, leaves
        // it there.
        *s.component_mut::<Focusable>(list).unwrap() = Focusable(true);

        // After the input's own handlers, so its text is already updated.
        let input = bar.input;
        s.on::<InputEdit>(input, |ctx, _| sync_query(ctx))
            .on::<KeyPress>(input, |ctx, _| sync_query(ctx))
            .on::<KeyRepeat>(input, |ctx, _| sync_query(ctx))
            .on::<InputFocus>(input, |ctx, e| on_focus(ctx, e.focused))
            .on::<Clicked>(bar.clear, |ctx, _| clear(ctx))
            .on::<Tapped>(me, |ctx, e| launch(ctx, e.0))
            .on::<Refit>(me, |ctx, e| {
                ctx.me().max_chars = Some(e.0);
                rebuild_rows(ctx);
            })
            .on::<Hidden>(me, |ctx, _| on_hidden(ctx))
            .on::<Shown>(me, |ctx, _| rescan(ctx));

        let apps = scan(&b.data_dirs);
        let icons = {
            let (mut atlas, mut cache) = s.fetch::<(ResMut<Atlas>, ResMut<IconCache>)>(me);
            resolve_icons(&mut atlas, &mut cache, &b.icon_dirs, &apps)
        };
        let mut ranker = Ranker::new();
        let hits = ranker.rank(&names(&apps), "");
        let mut drawer = Drawer {
            data_dirs: b.data_dirs,
            icon_dirs: b.icon_dirs,
            apps,
            icons,
            ranker,
            query: String::new(),
            hits,
            max_chars: None,
            focused: false,
            bar,
            list,
            rows: Vec::new(),
        };
        drawer.rows = row_builders(&drawer, me.id())
            .into_iter()
            .map(|row| s.spawn(list, row).id())
            .collect();
        drawer
    }
}

/// The running apps' front windows by desktop id, and how to activate one.
/// Kept current from the [`Registry`], which may be absent (no compositor
/// support, or tests): then nothing is running and taps launch.
struct Running {
    fronts: Vec<(String, ToplevelId)>,
    activator: Activator,
    launcher: Launcher,
}
impl Resource for Running {}

/// Sent by a tap on a running app; the activator runs with the app.
struct Activate(ToplevelId);
impl Signal for Activate {}

/// Sent by a tap on an app that isn't running or starting, with its icon;
/// the launcher runs with the app, under its launch placeholder.
struct StartApp {
    app: DesktopApp,
    icon: RowIcon,
}
impl Signal for StartApp {}

/// Production activator: raise the window for the seat, if the compositor
/// offers toplevel management.
pub(crate) fn activate_toplevel(app: &mut App, id: ToplevelId) {
    if !(app.has_resource::<Toplevels>()
        && app.has_resource::<Wayland>()
        && app.has_resource::<WlSeat>())
    {
        return;
    }
    let (toplevels, mut wl, seat) = app.query::<(Res<Toplevels>, ResMut<Wayland>, Res<WlSeat>)>();
    toplevels.activate(&mut wl, *seat, id);
}

fn sync_running(app: &mut App) {
    if !app.has_resource::<Registry>() {
        return;
    }
    let fronts = {
        let registry = app.resource::<Registry>();
        registry
            .apps()
            .iter()
            .filter_map(|a| {
                let entry = registry.entries().get(a.entry?)?;
                Some((entry.id.clone(), a.front))
            })
            .collect()
    };
    app.resource_mut::<Running>().fronts = fronts;
}

/// Registers the drawer's systems. Call once.
pub(crate) fn install(app: &mut App, activator: Activator, launcher: Launcher) {
    app.insert_resource(Running {
        fronts: Vec::new(),
        activator,
        launcher,
    });
    sync_running(app);
    crate::launch::install(app);
    crate::placeholder::install(app);
    app.system(|app: &mut App, _: &RegistryChanged| sync_running(app))
        .system(|app: &mut App, _: &Emitted<Shown>| sync_running(app))
        .system(|app: &mut App, e: &Activate| {
            let activator = app.resource::<Running>().activator.clone();
            activator(app, e.0);
        })
        .system(|app: &mut App, e: &StartApp| {
            let launch = crate::placeholder::start(app, &e.app, e.icon);
            let launcher = app.resource::<Running>().launcher.clone();
            launcher(app, &e.app, launch);
        });
    if !app.has_resource::<IconCache>() {
        let cache = IconCache::new(&mut app.resource_mut::<Atlas>());
        app.insert_resource(cache);
    }
    app.system(refit_on_layout);
}

fn names(apps: &[DesktopApp]) -> Vec<&str> {
    apps.iter().map(|a| a.name.as_str()).collect()
}

fn resolve_icons(
    atlas: &mut Atlas,
    cache: &mut IconCache,
    dirs: &IconDirs,
    apps: &[DesktopApp],
) -> Vec<RowIcon> {
    apps.iter()
        .map(|a| {
            let file = a.icon.as_deref().and_then(|i| dirs.find(i));
            cache.get(atlas, file.as_ref())
        })
        .collect()
}

fn row_builders(d: &Drawer, drawer: NodeId) -> Vec<RowBuilder> {
    d.hits
        .iter()
        .map(|hit| {
            let name = &d.apps[hit.index].name;
            let (name, positions) = fit(name, &hit.positions, d.max_chars.unwrap_or(usize::MAX));
            RowBuilder {
                name,
                positions,
                icon: d.icons[hit.index],
                index: hit.index,
                drawer,
            }
        })
        .collect()
}

fn rebuild_rows(ctx: &mut Context<'_, Drawer>) {
    for row in std::mem::take(&mut ctx.me().rows) {
        ctx.remove(row);
    }
    let me = ctx.handle().id();
    let list = ctx.me().list;
    let rows = row_builders(ctx.me(), me)
        .into_iter()
        .map(|row| ctx.spawn(list, row).id())
        .collect();
    ctx.me().rows = rows;
}

fn rerank(ctx: &mut Context<'_, Drawer>) {
    let d = ctx.me();
    d.hits = d.ranker.rank(&names(&d.apps), &d.query);
    rebuild_rows(ctx);
}

/// Reads the field; on a change, re-ranks. Restyles the bar either way.
fn sync_query(ctx: &mut Context<'_, Drawer>) {
    let input = ctx.me().bar.input;
    let text = ctx
        .widget::<Input>(input)
        .map(|i| i.text().to_string())
        .unwrap_or_default();
    if text != ctx.me().query {
        ctx.me().query = text;
        rerank(ctx);
    }
    restyle_bar(ctx);
}

fn restyle_bar(ctx: &mut Context<'_, Drawer>) {
    let d = ctx.me();
    let (bar, has_text) = (d.bar, !d.query.is_empty());
    let active = d.focused || has_text;
    bar.show(ctx, active, has_text);
}

fn on_focus(ctx: &mut Context<'_, Drawer>, focused: bool) {
    ctx.me().focused = focused;
    restyle_bar(ctx);
}

/// Empties the field and shows every app again.
fn clear_query(ctx: &mut Context<'_, Drawer>) {
    let input = ctx.me().bar.input;
    if let Some(mut c) = ctx.at(input) {
        c.set_text("");
    }
    sync_query(ctx);
}

fn clear(ctx: &mut Context<'_, Drawer>) {
    clear_query(ctx);
    if ctx.me().focused {
        // Tells the input method the text is now empty.
        let input = ctx.me().bar.input;
        ctx.emit(InputFocus { focused: true }, input);
    }
}

fn launch(ctx: &mut Context<'_, Drawer>, index: usize) {
    let d = ctx.me();
    let Some(app) = d.apps.get(index) else {
        return;
    };
    let entry = app.clone();
    let icon = d.icons[index];
    let front = ctx
        .resource::<Running>()
        .fronts
        .iter()
        .find(|(running, _)| *running == entry.id)
        .map(|&(_, front)| front);
    // Already starting: its placeholder again, not a second start.
    let starting = ctx
        .resource::<Launches>()
        .iter()
        .find(|l| l.app.id == entry.id)
        .map(|l| l.id);
    match (front, starting) {
        (Some(front), _) => ctx.signal(Activate(front)),
        (None, Some(launch)) => ctx.signal(ShowLaunch(launch)),
        // Starting is asynchronous: a failure ends its launch later.
        (None, None) => ctx.signal(StartApp { app: entry, icon }),
    }
    clear_query(ctx);
    ctx.signal(AppLaunched);
}

/// Blurs and clears the search; safe to repeat.
fn on_hidden(ctx: &mut Context<'_, Drawer>) {
    if ctx.me().focused {
        let input = ctx.me().bar.input;
        ctx.emit(InputFocus { focused: false }, input);
    }
    clear_query(ctx);
}

/// Picks up installs and removals since the last scan.
fn rescan(ctx: &mut Context<'_, Drawer>) {
    let apps = scan(&ctx.me().data_dirs);
    if apps == ctx.me().apps {
        return;
    }
    let dirs = ctx.me().icon_dirs.clone();
    let icons = {
        let (mut atlas, mut cache) = ctx.fetch::<(ResMut<Atlas>, ResMut<IconCache>)>();
        resolve_icons(&mut atlas, &mut cache, &dirs, &apps)
    };
    let d = ctx.me();
    d.apps = apps;
    d.icons = icons;
    rerank(ctx);
}

fn refit_on_layout(app: &mut App, _: &LayoutDone) {
    let drawers: Vec<(NodeId, Option<usize>)> = app
        .widgets::<Drawer>()
        .map(|(id, d)| (id, d.max_chars))
        .collect();
    for (id, current) in drawers {
        let width = app
            .component::<ComputedLayout>(id)
            .map_or(0.0, |l| l.rect.width());
        if width <= 0.0 {
            continue;
        }
        let fits = max_chars(width);
        if current != Some(fits) {
            app.emit(Refit(fits), id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use mecha_wayland::mechanix_widgets::{self as mw, ColorSource, prelude::FontBook};
    use screen::{LaunchPlaceholderDown, LaunchPlaceholderUp, Screen};

    use super::*;
    use crate::row::Row;
    use crate::{AppDrawer, LaunchPlaceholder};
    use apps::{AppKey, IconDirs, LaunchId, Launches, RunningApp, ShowLaunch, StopLaunch};

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#0a0"/></svg>"##;

    fn data_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("app-drawer-ui-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn install(data: &Path, id: &str, name: &str, exec: &str) {
        let apps = data.join("applications");
        fs::create_dir_all(&apps).unwrap();
        fs::write(
            apps.join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={id}\n"),
        )
        .unwrap();
        let icons = data.join("icons/hicolor/scalable/apps");
        fs::create_dir_all(&icons).unwrap();
        fs::write(icons.join(format!("{id}.svg")), SVG).unwrap();
    }

    fn three(name: &str) -> PathBuf {
        let d = data_dir(name);
        install(&d, "camera", "Camera", "camera");
        install(
            &d,
            "chromium",
            "Chromium",
            "chromium-browser --incognito %U",
        );
        install(&d, "clock", "Clock", "clock");
        d
    }

    fn drawer(data: &Path) -> AppDrawer {
        let dirs = vec![data.to_path_buf()];
        let icons = IconDirs::new(&dirs, None);
        AppDrawer::with_sources(dirs, icons)
    }

    struct Ui {
        app: App,
        win: NodeId,
        drawer: NodeId,
    }

    #[derive(Default)]
    struct Launched(usize);
    impl Resource for Launched {}

    fn count_launch(app: &mut App, _: &AppLaunched) {
        app.resource_mut::<Launched>().0 += 1;
    }

    /// text-input's focus rule, which needs a compositor to install: a
    /// focused field the keyboard focus moved off blurs.
    fn blur_on_focus_moved(app: &mut App, _: &OnChanged<KeyboardFocus>) {
        let focus = app.resource::<KeyboardFocus>().focused();
        let fields: Vec<NodeId> = app
            .widgets::<Drawer>()
            .filter(|(_, d)| d.focused)
            .map(|(_, d)| d.bar.input.id())
            .filter(|&field| focus != Some(field))
            .collect();
        for field in fields {
            app.emit(InputFocus { focused: false }, field);
        }
    }

    fn ui(screen: AppDrawer) -> Ui {
        ui_with(screen, None)
    }

    fn ui_with(screen: AppDrawer, registry: Option<Registry>) -> Ui {
        let mut app = App::new();
        if let Some(registry) = registry {
            app.insert_resource(registry);
        }
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(InteractivityModule)
            .add_module(AnimationModule)
            .add_module(ScrollModule)
            .add_module(MechanixTheme::dark())
            .init_resource::<Launched>()
            .system(count_launch)
            .system(blur_on_focus_moved);
        // PresentationModule registers Role, for the launch placeholder's
        // window; it needs a live compositor.
        app.register_component::<Role>();
        app.insert_resource(Atlas::new());
        let (regular, light) = {
            let mut atlas = app.resource_mut::<Atlas>();
            (
                atlas.add_font(assets::fonts::GEIST_MONO).unwrap(),
                atlas.add_font(assets::fonts::GEIST_MONO_LIGHT).unwrap(),
            )
        };
        app.insert_resource(FontBook::new(regular).light(light));
        let root = app.root();
        let win = app
            .spawn(
                root,
                window().layout(LayoutStyle::default().column().size(px(390.0), px(844.0))),
            )
            .id();
        let drawer = screen.build(&mut app, win);
        app.tick();
        app.tick();
        Ui { app, win, drawer }
    }

    impl Ui {
        fn me(&self) -> &Drawer {
            self.app.widget::<Drawer>(self.drawer).unwrap()
        }

        fn input(&self) -> NodeId {
            self.me().bar.input.id()
        }

        fn list(&self) -> NodeId {
            self.me().list.id()
        }

        fn rows(&self) -> Vec<NodeId> {
            let list = self.list();
            self.app
                .children(list)
                .unwrap()
                .iter()
                .copied()
                .filter(|&c| self.app.widget::<Row>(c).is_some())
                .collect()
        }

        fn names(&self) -> Vec<String> {
            self.rows()
                .into_iter()
                .map(|r| self.app.widget::<Row>(r).unwrap().name().to_string())
                .collect()
        }

        fn text(&self) -> String {
            self.app
                .widget::<Input>(self.input())
                .unwrap()
                .text()
                .to_string()
        }

        fn type_in(&mut self, s: &str) {
            let input = self.input();
            self.app.emit(InputFocus { focused: true }, input);
            self.app.emit(
                InputEdit {
                    commit: Some(s.into()),
                    ..Default::default()
                },
                input,
            );
            self.app.tick();
        }

        fn click(&mut self, node: NodeId) {
            self.app.emit(
                Clicked {
                    contact: ContactId::Touch(0),
                    position: Point::ZERO,
                },
                node,
            );
            self.app.tick();
        }
    }

    #[test]
    fn lists_every_app_a_to_z() {
        let ui = ui(drawer(&three("all")));
        assert_eq!(ui.names(), ["Camera", "Chromium", "Clock"]);
    }

    #[test]
    fn typing_filters_and_highlights() {
        let mut ui = ui(drawer(&three("filter")));
        ui.type_in("chr");
        assert_eq!(ui.names(), ["Chromium"]);

        let row = ui.rows()[0];
        let r = ui.app.widget::<Row>(row).unwrap();
        assert_eq!(r.runs()[0], ("Chr".to_string(), true));
        assert_eq!(r.runs()[1], ("omium".to_string(), false));
        let first = ui
            .app
            .descendants(row)
            .find_map(|n| ui.app.widget::<mw::Text>(n))
            .unwrap();
        assert_eq!(first.color, Some(ColorSource::Role(ColorRole::Primary)));
        assert_eq!(first.weight, Some(FontWeight::Regular));
    }

    #[test]
    fn no_match_shows_no_rows() {
        let mut ui = ui(drawer(&three("none")));
        ui.type_in("zzz");
        assert!(ui.rows().is_empty());
        let bar = ui.app.children(ui.drawer).unwrap()[0];
        assert!(ui.app.descendants(bar).any(|n| n == ui.input()));
    }

    #[test]
    fn clear_button_lies_inside_the_field() {
        let mut ui = ui(drawer(&three("inside")));
        ui.type_in("c");
        let clear = ui.me().bar.clear.id();
        let rect = |n| ui.app.component::<ComputedLayout>(n).unwrap().rect;
        let (field, button) = (rect(ui.input()), rect(clear));
        assert!(button.width() >= 44.0 && button.height() > 0.0);
        assert!(field.contains(button.origin));
        assert!(button.right() <= field.right() && button.bottom() <= field.bottom());
    }

    #[test]
    fn clear_restores_the_list() {
        let mut ui = ui(drawer(&three("clear")));
        ui.type_in("chr");
        assert_eq!(ui.rows().len(), 1);
        assert!(ui.me().focused);

        // A real tap at the button's centre, through hit testing.
        let clear = ui.me().bar.clear.id();
        let r = ui.app.component::<ComputedLayout>(clear).unwrap().rect;
        let at = Point::new(r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
        for phase in [
            ContactPhase::Moved,
            ContactPhase::Pressed,
            ContactPhase::Released,
        ] {
            let window = ui.win;
            ui.app.signal(ContactInput {
                window,
                contact: ContactId::Touch(0),
                phase,
                position: at,
                time: 0,
            });
            ui.app.flush();
        }
        ui.app.tick();

        assert_eq!(ui.names(), ["Camera", "Chromium", "Clock"]);
        assert_eq!(ui.text(), "");
        assert!(ui.me().focused);
    }

    #[test]
    fn tap_launches_clears_and_signals() {
        let seen: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
        let rec = seen.clone();
        let screen = drawer(&three("tap")).launcher(move |_, entry, _| {
            assert_eq!(entry.id, "chromium.desktop");
            rec.borrow_mut().push(entry.exec.clone());
        });
        let mut ui = ui(screen);
        ui.type_in("chr");
        let row = ui.rows()[0];
        ui.click(row);

        assert_eq!(*seen.borrow(), [["chromium-browser", "--incognito"]]);
        assert_eq!(ui.text(), "");
        assert_eq!(ui.rows().len(), 3);
        assert_eq!(ui.app.resource::<Launched>().0, 1);
    }

    fn registry_with(running: &[(&str, u32)]) -> Registry {
        let entries = vec![DesktopApp {
            id: "chromium.desktop".into(),
            name: "Chromium".into(),
            exec: vec!["chromium-browser".into(), "--incognito".into()],
            icon: None,
            startup_wm_class: None,
            path: PathBuf::from("/usr/share/applications/chromium.desktop"),
            dbus_activatable: false,
        }];
        let apps = running
            .iter()
            .map(|&(app_id, n)| RunningApp {
                key: AppKey::Entry(app_id.into()),
                entry: Some(0),
                windows: vec![ToplevelId::from_raw(n)],
                dialogs: Vec::new(),
                modal: false,
                pids: Vec::new(),
                front: ToplevelId::from_raw(n),
                last_used: 1,
                title: String::new(),
            })
            .collect();
        Registry::fixture(entries, apps)
    }

    #[test]
    fn tap_on_running_app_activates_instead() {
        let launched: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
        let activated: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
        let (l, a) = (launched.clone(), activated.clone());
        let screen = drawer(&three("run"))
            .launcher(move |_, entry, _| l.borrow_mut().push(entry.exec.clone()))
            .activator(move |_, id| a.borrow_mut().push(id));
        let mut ui = ui_with(screen, Some(registry_with(&[("chromium", 7)])));
        ui.type_in("chr");
        let row = ui.rows()[0];
        ui.click(row);

        assert_eq!(*activated.borrow(), [ToplevelId::from_raw(7)]);
        assert!(launched.borrow().is_empty());
        assert_eq!(ui.text(), "");
        assert_eq!(ui.app.resource::<Launched>().0, 1);
    }

    #[test]
    fn tap_on_stopped_app_still_launches() {
        let launched: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
        let activated: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
        let (l, a) = (launched.clone(), activated.clone());
        let screen = drawer(&three("stopped"))
            .launcher(move |_, entry, _| l.borrow_mut().push(entry.exec.clone()))
            .activator(move |_, id| a.borrow_mut().push(id));
        let mut ui = ui_with(screen, Some(registry_with(&[])));
        ui.type_in("chr");
        let row = ui.rows()[0];
        ui.click(row);

        assert_eq!(launched.borrow().len(), 1);
        assert!(activated.borrow().is_empty());
        assert_eq!(ui.app.resource::<Launched>().0, 1);
    }

    /// Whether a launch's placeholder is up.
    fn placeholders(ui: &mut Ui) -> usize {
        usize::from(crate::placeholder::shown(&ui.app).is_some())
    }

    /// Search for `query` and tap the first row.
    fn tap_first(ui: &mut Ui, query: &str) {
        ui.type_in(query);
        let row = ui.rows()[0];
        ui.click(row);
        ui.app.tick();
    }

    /// A drawer named `name`, no app running, and Chromium tapped: starting,
    /// under its placeholder.
    fn chromium_starting(name: &str) -> Ui {
        let screen = drawer(&three(name)).launcher(|_, _, _| {});
        let mut ui = ui_with(screen, Some(registry_with(&[])));
        tap_first(&mut ui, "chr");
        ui
    }

    #[test]
    fn a_launch_shows_the_placeholder_until_the_app_runs() {
        let mut ui = chromium_starting("placeholder");
        assert_eq!(placeholders(&mut ui), 1);

        // Another app's window doesn't end it.
        ui.app
            .insert_resource(Registry::fixture(Vec::new(), Vec::new()));
        ui.app.signal(RegistryChanged);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 1);

        ui.app.insert_resource(registry_with(&[("chromium", 7)]));
        ui.app.signal(RegistryChanged);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);
    }

    #[test]
    fn activating_a_running_app_shows_no_placeholder() {
        let screen = drawer(&three("no-placeholder")).activator(|_, _| {});
        let mut ui = ui_with(screen, Some(registry_with(&[("chromium", 7)])));
        tap_first(&mut ui, "chr");
        assert_eq!(placeholders(&mut ui), 0);
    }

    fn launches(ui: &Ui) -> Vec<LaunchId> {
        ui.app.resource::<Launches>().iter().map(|l| l.id).collect()
    }

    #[test]
    fn the_placeholder_can_be_taken_down_early_and_shown_again() {
        let mut ui = chromium_starting("placeholder-early");
        assert_eq!(placeholders(&mut ui), 1);
        let launch = launches(&ui);
        assert_eq!(launch.len(), 1);

        crate::placeholder::take_down(&mut ui.app);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);
        assert_eq!(launches(&ui), launch, "the launch goes on");

        ui.app.signal(ShowLaunch(launch[0]));
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 1);
    }

    #[test]
    fn a_launch_stopped_goes_with_its_placeholder() {
        let mut ui = chromium_starting("placeholder-stop");
        let launch = launches(&ui)[0];

        ui.app.signal(StopLaunch(launch));
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);
        assert!(launches(&ui).is_empty());
    }

    /// The container asks the placeholder signals made: `(up, down)`.
    #[derive(Default)]
    struct Asked(u32, u32);
    impl Resource for Asked {}

    /// The placeholder follows its container: a tap asks for it, nav hiding
    /// it (the user went elsewhere) leaves the launch going, and the launch
    /// ending gives up only a container still asked for.
    #[test]
    fn the_placeholder_follows_its_container() {
        let screen = drawer(&three("placeholder-container")).launcher(|_, _, _| {});
        let mut ui = ui_with(screen, Some(registry_with(&[])));
        let root = ui.app.root();
        let placeholder = LaunchPlaceholder.build(&mut ui.app, root);
        ui.app
            .init_resource::<Asked>()
            .system(|app: &mut App, _: &LaunchPlaceholderUp| app.resource_mut::<Asked>().0 += 1)
            .system(|app: &mut App, _: &LaunchPlaceholderDown| app.resource_mut::<Asked>().1 += 1);
        let asked = |ui: &Ui| {
            let a = ui.app.resource::<Asked>();
            (a.0, a.1)
        };

        tap_first(&mut ui, "chr");
        assert_eq!(asked(&ui), (1, 0));
        assert_eq!(placeholders(&mut ui), 1);
        let launch = launches(&ui)[0];

        ui.app.emit(Hidden, vec![placeholder]);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);
        assert_eq!(launches(&ui), [launch], "the launch goes on");

        ui.app.signal(ShowLaunch(launch));
        ui.app.tick();
        assert_eq!(asked(&ui), (2, 0));
        ui.app.signal(StopLaunch(launch));
        ui.app.tick();
        assert_eq!(asked(&ui), (2, 1));
        assert_eq!(placeholders(&mut ui), 0);

        // Over with its container already closed: nothing to give up.
        tap_first(&mut ui, "chr");
        let launch = launches(&ui)[0];
        ui.app.emit(Hidden, vec![placeholder]);
        ui.app.tick();
        ui.app.signal(StopLaunch(launch));
        ui.app.tick();
        assert_eq!(asked(&ui), (3, 1));
    }

    /// A second tap while the app starts brings its placeholder back; it
    /// doesn't start the app again.
    #[test]
    fn tapping_an_app_still_starting_shows_its_placeholder_again() {
        let started = Rc::new(RefCell::new(0));
        let s = started.clone();
        let screen =
            drawer(&three("placeholder-again")).launcher(move |_, _, _| *s.borrow_mut() += 1);
        let mut ui = ui_with(screen, Some(registry_with(&[])));
        tap_first(&mut ui, "chr");
        crate::placeholder::take_down(&mut ui.app);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);

        tap_first(&mut ui, "chr");
        assert_eq!(*started.borrow(), 1, "started once");
        assert_eq!(launches(&ui).len(), 1);
        assert_eq!(placeholders(&mut ui), 1);
    }

    /// A start that failed ends its launch at once, placeholder and all.
    #[test]
    fn a_failed_start_ends_its_launch() {
        let mut ui = chromium_starting("placeholder-failed");
        let launch = launches(&ui)[0];

        crate::placeholder::failed(&mut ui.app, launch);
        ui.app.tick();
        assert_eq!(placeholders(&mut ui), 0);
        assert!(launches(&ui).is_empty());
        // Twice is harmless.
        crate::placeholder::failed(&mut ui.app, launch);
    }

    #[test]
    fn a_press_on_the_list_blurs_the_search() {
        let mut ui = ui(drawer(&three("press-list")));
        ui.type_in("c");
        assert!(ui.me().focused);

        // A real press on a row, through hit testing: the start of a
        // scroll or a tap alike.
        let row = ui.rows()[0];
        let r = ui.app.component::<ComputedLayout>(row).unwrap().rect;
        let at = Point::new(r.x() + r.width() / 2.0, r.y() + r.height() / 2.0);
        for phase in [ContactPhase::Moved, ContactPhase::Pressed] {
            let window = ui.win;
            ui.app.signal(ContactInput {
                window,
                contact: ContactId::Touch(0),
                phase,
                position: at,
                time: 0,
            });
            ui.app.flush();
        }
        ui.app.tick();
        assert!(!ui.me().focused);
    }

    #[test]
    fn hidden_blurs_and_clears_the_search() {
        let mut ui = ui(drawer(&three("focus")));
        assert!(!ui.me().focused);
        ui.type_in("chr");
        assert!(ui.me().focused);

        let d = ui.drawer;
        ui.app.emit(Hidden, d);
        ui.app.tick();
        assert!(!ui.me().focused);
        assert_eq!(ui.text(), "");
        assert_eq!(ui.rows().len(), 3);
    }

    #[test]
    fn shrinking_list_clamps_scroll() {
        let data = data_dir("clamp");
        for i in 0..58 {
            install(
                &data,
                &format!("filler{i:02}"),
                &format!("Filler {i:02}"),
                "f",
            );
        }
        install(&data, "zebra1", "Zebra One", "z");
        install(&data, "zebra2", "Zebra Two", "z");
        let mut ui = ui(drawer(&data));
        assert_eq!(ui.rows().len(), 60);

        let list = ui.list();
        let l = *ui.app.component::<ComputedLayout>(list).unwrap();
        let last = *ui.rows().last().unwrap();
        let bottom = ui
            .app
            .component::<ComputedLayout>(last)
            .unwrap()
            .rect
            .bottom();
        let max = bottom + l.padding.bottom - l.rect.y() - l.rect.height();
        assert!(max > 0.0, "60 rows overflow the list");
        ui.app.component_mut::<Viewport>(list).unwrap().scroll.y = max;

        ui.type_in("zebra");
        assert_eq!(ui.rows().len(), 2);
        assert_eq!(ui.app.component::<Viewport>(list).unwrap().scroll.y, 0.0);
    }

    #[test]
    fn shown_picks_up_a_new_install() {
        let data = three("shown");
        let mut ui = ui(drawer(&data));
        install(&data, "files", "Files", "nautilus");
        let d = ui.drawer;
        ui.app.emit(Shown, d);
        ui.app.tick();
        assert_eq!(ui.names(), ["Camera", "Chromium", "Clock", "Files"]);
    }
}
