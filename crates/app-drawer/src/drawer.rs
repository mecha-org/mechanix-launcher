//! The drawer: a search bar over a scrolling list of every installed app.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use mecha_wayland::prelude::*;
use screen::{AppLaunched, Hidden, Shown};

use crate::rank::{Hit, Ranker, fit};
use crate::row::{GAP, NAME_PX, RowBuilder, SIDE, Tapped};
use crate::search_bar::SearchBar;
use crate::{Activator, Launcher};
use apps::{DesktopApp, IconCache, IconDirs, Registry, RegistryChanged, RowIcon, scan};

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

/// Sent by the drawer for its window; applied when there is a compositor.
struct SetKeyboard {
    window: NodeId,
    ki: KeyboardInteractivity,
}
impl Signal for SetKeyboard {}

pub(crate) struct DrawerBuilder {
    pub data_dirs: Vec<PathBuf>,
    pub icon_dirs: Rc<IconDirs>,
    pub launcher: Launcher,
    pub at_top: Rc<Cell<bool>>,
}

impl Build for DrawerBuilder {
    type Widget = Drawer;
}

pub(crate) struct Drawer {
    data_dirs: Vec<PathBuf>,
    icon_dirs: Rc<IconDirs>,
    launcher: Launcher,
    at_top: Rc<Cell<bool>>,
    apps: Vec<DesktopApp>,
    icons: Vec<RowIcon>,
    ranker: Ranker,
    query: String,
    hits: Vec<Hit>,
    /// `None` until the first layout gives the drawer a width.
    max_chars: Option<usize>,
    focused: bool,
    keyboard: KeyboardInteractivity,
    bar: SearchBar,
    list: Handle<Scrollable>,
    rows: Vec<NodeId>,
}

impl Drawer {
    /// The keyboard interactivity last asked of the drawer's window.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn keyboard(&self) -> KeyboardInteractivity {
        self.keyboard
    }
}

impl Widget for Drawer {
    type Builder = DrawerBuilder;

    fn build(b: DrawerBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().column().fill();
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        let bar = SearchBar::spawn(s, me);
        let list = s.spawn(
            me,
            scrollable().style(
                LayoutStyle::default()
                    .block()
                    .fill()
                    .min_height(px(0.0))
                    .padding(Insets::new(px(0.0), px(0.0), px(LIST_BOTTOM), px(0.0))),
            ),
        );

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
            launcher: b.launcher,
            at_top: b.at_top,
            apps,
            icons,
            ranker,
            query: String::new(),
            hits,
            max_chars: None,
            focused: false,
            keyboard: KeyboardInteractivity::None,
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
}
impl Resource for Running {}

/// Sent by a tap on a running app; the activator runs with the app.
struct Activate(ToplevelId);
impl Signal for Activate {}

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
pub(crate) fn install(app: &mut App, activator: Activator) {
    app.insert_resource(Running {
        fronts: Vec::new(),
        activator,
    });
    sync_running(app);
    app.system(|app: &mut App, _: &RegistryChanged| sync_running(app))
        .system(|app: &mut App, _: &Emitted<Shown>| sync_running(app))
        .system(|app: &mut App, e: &Activate| {
            let activator = app.resource::<Running>().activator.clone();
            activator(app, e.0);
        });
    if !app.has_resource::<IconCache>() {
        let cache = IconCache::new(&mut app.resource_mut::<Atlas>());
        app.insert_resource(cache);
    }
    app.system(track_top)
        .system(refit_on_layout)
        .system(apply_keyboard);
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
    let ki = if focused {
        KeyboardInteractivity::Exclusive
    } else {
        KeyboardInteractivity::None
    };
    set_keyboard(ctx, ki);
    restyle_bar(ctx);
}

/// Remembers `ki` and asks the drawer's window for it.
fn set_keyboard(ctx: &mut Context<'_, Drawer>, ki: KeyboardInteractivity) {
    ctx.me().keyboard = ki;
    if let Some(window) = ctx.component::<InWindow>().and_then(|w| w.0) {
        ctx.signal(SetKeyboard { window, ki });
    }
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
    let (id, exec, launcher) = (app.id.clone(), app.exec.clone(), d.launcher.clone());
    let front = ctx
        .resource::<Running>()
        .fronts
        .iter()
        .find(|(running, _)| *running == id)
        .map(|&(_, front)| front);
    if let Some(front) = front {
        ctx.signal(Activate(front));
        clear_query(ctx);
        ctx.signal(AppLaunched);
        return;
    }
    match launcher(&exec) {
        Ok(()) => {
            clear_query(ctx);
            ctx.signal(AppLaunched);
        }
        Err(e) => tracing::warn!(%id, "launch failed: {e}"),
    }
}

/// Hands the keyboard back and clears the search; safe to repeat.
fn on_hidden(ctx: &mut Context<'_, Drawer>) {
    if ctx.me().focused {
        let input = ctx.me().bar.input;
        ctx.emit(InputFocus { focused: false }, input);
    }
    set_keyboard(ctx, KeyboardInteractivity::None);
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

/// Each frame: the nav may close the drawer by swipe only while its list
/// is at the top (a rubber-band pull past it still counts).
fn track_top(app: &mut App, _: &Frame) {
    let lists: Vec<(NodeId, Rc<Cell<bool>>)> = app
        .widgets::<Drawer>()
        .map(|(_, d)| (d.list.id(), d.at_top.clone()))
        .collect();
    for (list, gate) in lists {
        if let Some(v) = app.component::<Viewport>(list) {
            gate.set(v.scroll.y <= 0.5);
        }
    }
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

fn apply_keyboard(app: &mut App, s: &SetKeyboard) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_keyboard_interactivity(s.window, s.ki, &mut wl);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use mecha_wayland::mechanix_widgets::{self as mw, ColorSource, prelude::FontBook};
    use screen::Screen;

    use super::*;
    use crate::AppDrawer;
    use crate::row::Row;
    use apps::{AppKey, IconDirs, RunningApp};

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

    /// text-input's press rule, which needs a compositor to install: a
    /// press in the field's window whose targets miss the focused field
    /// blurs it.
    fn blur_on_outside_press(app: &mut App, e: &Emitted<Press>) {
        let fields: Vec<NodeId> = app
            .widgets::<Drawer>()
            .filter(|(_, d)| d.focused)
            .map(|(_, d)| d.bar.input.id())
            .collect();
        for field in fields {
            if !e.targets.iter().any(|&t| t == field) {
                app.emit(InputFocus { focused: false }, field);
            }
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
            .system(blur_on_outside_press);
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
        assert_eq!(ui.me().keyboard(), KeyboardInteractivity::Exclusive);

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
        assert_eq!(ui.me().keyboard(), KeyboardInteractivity::Exclusive);
    }

    #[test]
    fn tap_launches_clears_and_signals() {
        let seen: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
        let rec = seen.clone();
        let screen = drawer(&three("tap")).launcher(move |argv| {
            rec.borrow_mut().push(argv.to_vec());
            Ok(())
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
        }];
        let apps = running
            .iter()
            .map(|&(app_id, n)| RunningApp {
                key: AppKey::AppId(app_id.into()),
                entry: Some(0),
                windows: vec![ToplevelId::from_raw(n)],
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
            .launcher(move |argv| {
                l.borrow_mut().push(argv.to_vec());
                Ok(())
            })
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
            .launcher(move |argv| {
                l.borrow_mut().push(argv.to_vec());
                Ok(())
            })
            .activator(move |_, id| a.borrow_mut().push(id));
        let mut ui = ui_with(screen, Some(registry_with(&[])));
        ui.type_in("chr");
        let row = ui.rows()[0];
        ui.click(row);

        assert_eq!(launched.borrow().len(), 1);
        assert!(activated.borrow().is_empty());
        assert_eq!(ui.app.resource::<Launched>().0, 1);
    }

    #[test]
    fn focus_takes_the_keyboard_and_hidden_gives_it_back() {
        let mut ui = ui(drawer(&three("focus")));
        assert_eq!(ui.me().keyboard(), KeyboardInteractivity::None);
        ui.type_in("chr");
        assert_eq!(ui.me().keyboard(), KeyboardInteractivity::Exclusive);

        let d = ui.drawer;
        ui.app.emit(Hidden, d);
        ui.app.tick();
        assert_eq!(ui.me().keyboard(), KeyboardInteractivity::None);
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
        let screen = drawer(&data);
        let at_top = screen.at_top();
        let mut ui = ui(screen);
        assert_eq!(ui.rows().len(), 60);
        assert!(at_top.get());

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
        ui.app.signal(Frame(ui.win));
        ui.app.flush();
        assert!(!at_top.get());

        ui.type_in("zebra");
        assert_eq!(ui.rows().len(), 2);
        assert_eq!(ui.app.component::<Viewport>(list).unwrap().scroll.y, 0.0);
        ui.app.signal(Frame(ui.win));
        ui.app.flush();
        assert!(at_top.get());
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
