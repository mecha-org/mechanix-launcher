use app_drawer::{AppDrawer, GEIST_MONO, GEIST_MONO_LIGHT, LaunchPlaceholder};
use apps::RegistryModule;
use control_center::app_screens::AppScreen;
use control_center::{ControlCenterScreen, select_screen};
use home::Home;
use keyboard::{KeyboardModule, hide_keyboard, show_keyboard, suspend_keyboard_zone};
use lockscreen::{IdleModule, IdleSettings, LockChanged, LockSettings, LockscreenModule};
use mecha_pulseaudio::{PulseAudioModule, VolumeControlModule};
use mecha_wayland::mechanix_widgets::prelude::{Clock, ClockTick, FontBook};
use mecha_wayland::prelude::*;
use nav::core::{
    AreaId, BOTTOM_EDGE, ContainerId, Direction, ExternalEvent, FADE, INSTANT, NavConfig,
    OnExternal, Span, TOP_EDGE, Timing, container, swipe,
};
use nav::shell::{ContainerShown, NavExternal, NavModule, nav_window};
use notifications::NotificationsModule;
use power_menu::PowerMenuModule;
use quick_settings::QuickSettingsModule;
use running_apps::{RunningApps, RunningAppsModule};
use screen::{OrientationModule, Screen};
use services::{KeepAwakeModule, PrepareForSleep, SessionModule};
use status_bar::{
    DEFAULT_BAR_HEIGHT, DEFAULT_ICON_SIZE, StatusBarModule, StatusBarScreen, StatusIcons,
};

const HOME: AreaId = AreaId("home");
const LAUNCH: AreaId = AreaId("launch-placeholder");
const RUNNING_APPS: AreaId = AreaId("running-apps");
const PANEL: AreaId = AreaId("panel");
const STATUS_BAR: AreaId = AreaId("status-bar");

const HOME_ID: ContainerId = ContainerId("home");
const DRAWER: ContainerId = ContainerId("drawer");
const PLACEHOLDER: ContainerId = ContainerId("placeholder");
const RUNNING: ContainerId = ContainerId("running");
const CONTROL_CENTER: ContainerId = ContainerId("control_center");
const STATUS: ContainerId = ContainerId("status");

/// The launcher screens that cover apps. While any is open the keyboard
/// reserves no space, so they and the apps under them keep their full
/// size. Opening one hides the keyboard, and the apps lose the keyboard
/// focus while it shows (the app switcher and the launch placeholder take
/// it; for home, the compositor keeps it off the hidden apps), so an app's
/// text input beneath can't bring it back until it closes. The control
/// center is not one: it is a panel, over the keyboard and whatever is
/// shown, and leaves them as they are.
const COVERS: [ContainerId; 3] = [HOME_ID, PLACEHOLDER, RUNNING];

/// GPU atlas pages: the default, plus a colour page for the keyboard's
/// emoji, which the app icons leave no room for at 2x.
const BUDGET: Budget = Budget {
    mono_pages: 4,
    color_pages: 3,
};

/// The [`COVERS`] open now.
struct OpenCovers(Vec<ContainerId>);
impl Resource for OpenCovers {}

/// The session locked: nav hears it, and closes what isn't to greet the
/// unlock.
fn on_lock(app: &mut App, e: &LockChanged) {
    if e.0 {
        app.signal(NavExternal(ExternalEvent::Locked));
    }
}

/// Woken from sleep: every clock shows the time now. Their timers are
/// monotonic, which doesn't count the time asleep.
fn on_wake(app: &mut App, e: &PrepareForSleep) {
    if !e.0 {
        let clocks: Vec<NodeId> = app.widgets::<Clock>().map(|(id, _)| id).collect();
        app.emit(ClockTick, clocks);
    }
}

fn on_container_shown(app: &mut App, s: &ContainerShown) {
    if !COVERS.contains(&s.id) {
        return;
    }
    let any_open = {
        let mut open = app.resource_mut::<OpenCovers>();
        open.0.retain(|&id| id != s.id);
        if s.shown {
            open.0.push(s.id);
        }
        !open.0.is_empty()
    };
    // Losing the focus hides a keyboard an app's text input brought up, but
    // not one shown by hand. Hiding also clears the zone first, so the app
    // under resizes as the screen comes up.
    if s.shown {
        hide_keyboard(app);
    }
    suspend_keyboard_zone(app, any_open);
}

/// A screen laid out below the status bar, for a window that reserves no
/// zones and ignores everyone else's (exclusive zone `-1`). Such a window
/// stays the size of the output whether or not the keyboard reserves its
/// height, so opening it over an app with the keyboard up draws no frame
/// at the size the keyboard left it.
struct BelowBar<S>(S);

impl<S: Screen> Screen for BelowBar<S> {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        let area = LayoutStyle::default().absolute().inset(Insets::new(
            px(DEFAULT_BAR_HEIGHT),
            px(0.0),
            px(0.0),
            px(0.0),
        ));
        let area = app.spawn(parent, div().style(area)).id();
        self.0.build(app, area)
    }
}

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

/// A layer role. `shell_role` tells the compositor the surface's role,
/// which decides its stacking and edge-gesture routing: `Edge` gesture zones
/// and `Bar` bars above `View` views, all above a fullscreen app.
fn role(
    layer: Layer,
    anchor: Anchor,
    exclusive_zone: i32,
    shell_role: ShellRole,
    namespace: &str,
) -> Role {
    Role::Layer(LayerRole {
        layer,
        anchor,
        exclusive_zone,
        namespace: namespace.into(),
        keyboard_interactivity: KeyboardInteractivity::None,
        shell_role: Some(shell_role),
    })
}

/// Home's role: on the bottom layer, below the apps, as layer shell keeps
/// what a shell shows when nothing covers it; over them only while it
/// shows home. Its zone ignores everyone's, and the drawer's search takes
/// the keyboard when tapped.
fn home_role() -> Role {
    Role::Layer(LayerRole {
        layer: Layer::Bottom,
        anchor: ALL_SIDES,
        exclusive_zone: -1,
        namespace: "home".into(),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        shell_role: None,
    })
}

/// A quick swipe up from the middle of the bottom edge; the corners are the
/// keyboard's.
fn bottom_quick() -> nav::core::Swipe {
    swipe(BOTTOM_EDGE, Direction::Up, Timing::Quick).span(Span::Middle)
}

/// Any swipe up from one bottom corner, quick or held: shows the keyboard.
fn bottom_corner(span: Span) -> nav::core::Swipe {
    swipe(BOTTOM_EDGE, Direction::Up, Timing::Any).span(span)
}

/// A panel that pulls down from its span of the top edge and closes on an up swipe.
fn panel(id: ContainerId, screen: impl screen::Screen, span: Span) -> nav::core::ContainerSpec {
    container(id, screen)
        .open(
            swipe(TOP_EDGE, Direction::Down, Timing::Any).span(span),
            INSTANT,
        )
        .close(swipe(PANEL, Direction::Up, Timing::Any), INSTANT)
        // Not where unlocking leaves you.
        .close(OnExternal(ExternalEvent::Locked), INSTANT)
}

fn top_half(span: Span) -> nav::core::Swipe {
    swipe(TOP_EDGE, Direction::Down, Timing::Any).span(span)
}

/// Opaque space under the keyboard's keys; the bottom swipe strip sits inside it.
const KEYBOARD_BOTTOM_GAP: f32 = 20.0;

fn navigation(cfg: NavConfig, drawer: AppDrawer) -> NavModule {
    let strip = LayoutStyle::default().height(px(cfg.bottom_strip_height));
    let bar = LayoutStyle::default().height(px(DEFAULT_BAR_HEIGHT));
    let sides = Anchor::LEFT | Anchor::RIGHT;

    // Home stays open while an app it started starts: what closes it is the
    // app coming to the front, which the compositor tells (`HomeLeft`).
    let home = container(HOME_ID, BelowBar(Home))
        .open(bottom_quick(), FADE)
        .close(OnExternal(ExternalEvent::HomeLeft), INSTANT)
        .close(OnExternal(ExternalEvent::WindowOpened), INSTANT)
        .open(OnExternal(ExternalEvent::LastWindowClosed), FADE)
        // FIXME: consider giving the drawer a window of its own, stacked
        // above home's.
        .child(
            container(DRAWER, BelowBar(drawer))
                .open(swipe(HOME, Direction::Up, Timing::Any), INSTANT)
                .close(swipe(HOME, Direction::Down, Timing::Any), INSTANT)
                .close(bottom_quick(), INSTANT),
        )
        .initially_open();
    // An app starting, over home until its window comes; left like any
    // screen, while the app goes on starting. Recent apps opens over it.
    let placeholder = container(PLACEHOLDER, LaunchPlaceholder)
        .open(OnExternal(ExternalEvent::LaunchPlaceholderUp), INSTANT)
        .close(OnExternal(ExternalEvent::LaunchPlaceholderDown), INSTANT)
        .close(bottom_quick(), INSTANT)
        // Not where unlocking leaves you.
        .close(OnExternal(ExternalEvent::Locked), INSTANT);
    let running = container(RUNNING, BelowBar(RunningApps::new()))
        .open(
            swipe(BOTTOM_EDGE, Direction::Up, Timing::Held).span(Span::Middle),
            INSTANT,
        )
        .close(bottom_quick(), INSTANT)
        .close(OnExternal(ExternalEvent::AppLaunched), INSTANT)
        // Closing the last app from here leaves nothing to show but Home.
        .close(OnExternal(ExternalEvent::LastWindowClosed), INSTANT)
        // Not where unlocking leaves you.
        .close(OnExternal(ExternalEvent::Locked), INSTANT);

    NavModule::new(cfg)
        .action(bottom_corner(Span::LeftCorner), show_keyboard)
        .action(bottom_corner(Span::RightCorner), show_keyboard)
        .action(top_half(Span::LeftHalf), |app| {
            select_screen(app, AppScreen::NotificationsScreen)
        })
        .action(top_half(Span::RightHalf), |app| {
            select_screen(app, AppScreen::QuickSettingsScreen)
        })
        .window(
            nav_window(HOME, home_role(), LayoutStyle::default())
                .container(home)
                .home(),
        )
        .window(
            nav_window(
                LAUNCH,
                role(
                    Layer::Overlay,
                    ALL_SIDES,
                    -1,
                    ShellRole::View,
                    "launch-placeholder",
                ),
                LayoutStyle::default(),
            )
            .container(placeholder)
            .takes_keyboard(),
        )
        // After the placeholder's: recent apps stacks over it.
        .window(
            nav_window(
                RUNNING_APPS,
                role(
                    Layer::Overlay,
                    ALL_SIDES,
                    -1,
                    ShellRole::View,
                    "running-apps",
                ),
                LayoutStyle::default(),
            )
            .container(running)
            .takes_keyboard(),
        )
        // FIXME: the compositor never draws the edge strips; give them a
        // single-pixel buffer scaled by wp_viewporter instead of rendering
        // them.
        .window(
            nav_window(
                BOTTOM_EDGE,
                // Above the keyboard, in the gap it leaves under its keys;
                // zone -1 keeps it on the edge rather than above the
                // keyboard's zone.
                role(
                    Layer::Overlay,
                    Anchor::BOTTOM | sides,
                    -1,
                    ShellRole::Edge,
                    "bottom-edge",
                ),
                strip,
            )
            .transparent(),
        )
        .window(
            nav_window(
                PANEL,
                role(Layer::Overlay, ALL_SIDES, -1, ShellRole::Panel, "panel"),
                LayoutStyle::default(),
            )
            .container(panel(
                CONTROL_CENTER,
                BelowBar(ControlCenterScreen),
                Span::Whole,
            )),
        )
        .window(
            nav_window(
                TOP_EDGE,
                role(
                    Layer::Overlay,
                    Anchor::TOP | sides,
                    -1,
                    ShellRole::Edge,
                    "top-edge",
                ),
                bar.clone(),
            )
            .transparent(),
        )
        .window(
            nav_window(
                STATUS_BAR,
                role(
                    Layer::Overlay,
                    Anchor::TOP | sides,
                    DEFAULT_BAR_HEIGHT as i32,
                    ShellRole::Bar,
                    "status-bar",
                ),
                bar,
            )
            .container(
                container(STATUS, StatusBarScreen)
                    .initially_open()
                    // Out of the way of a fullscreen app: with nothing
                    // shown, the window is hidden.
                    .close(OnExternal(ExternalEvent::FullscreenEntered), INSTANT)
                    .open(OnExternal(ExternalEvent::FullscreenLeft), INSTANT),
            )
            .no_input(),
        )
}

fn main() {
    // An auth worker runs PAM and exits; it never gets past this.
    lockscreen::run_worker_if_asked();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(OrientationModule)
        .add_module(InteractivityModule)
        .add_module(AnimationModule)
        .add_module(ScrollModule)
        .add_module(RenderModule::default())
        .insert_resource(Atlas::new());

    app.add_module(MechanixTheme::dark());

    let (regular, light) = {
        let mut atlas = app.resource_mut::<Atlas>();
        (
            atlas.add_font(GEIST_MONO).expect("GeistMono font loads"),
            atlas
                .add_font(GEIST_MONO_LIGHT)
                .expect("GeistMono Light font loads"),
        )
    };
    app.insert_resource(FontBook::new(regular).light(light));

    let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
        .expect("Status bar icons load");
    app.insert_resource(icons);

    app.add_module(RingModule::default())
        .add_module(TimerModule)
        .add_module(DBusModule::new().system().session())
        .add_module(PulseAudioModule)
        .add_module(VolumeControlModule)
        .add_module(
            WaylandModule::new()
                .bind::<WlCompositor>()
                .bind::<ZwpLinuxDmabufV1>()
                .bind::<XdgWmBase>()
                .bind::<WlSeat>()
                .bind::<ZwlrLayerShellV1>(),
        )
        .add_module(TextInputModule)
        .add_module(ToplevelsModule)
        .add_module(RegistryModule {
            data_dirs: apps::host_sources().0,
        })
        // Places the previews before presentation commits a frame.
        .add_module(RunningAppsModule)
        .add_module(PresentationModule {
            app_id: "mechanix.launcher".into(),
            budget: BUDGET,
        });

    let cfg = NavConfig::default();
    let bottom_gap = KEYBOARD_BOTTOM_GAP;
    app.add_module(navigation(cfg, AppDrawer::new()))
        .add_module(StatusBarModule)
        .add_module(NotificationsModule)
        .add_module(KeyboardModule { bottom_gap })
        .add_module(QuickSettingsModule)
        .add_module(PowerMenuModule)
        .add_module(SessionModule)
        .add_module(KeepAwakeModule)
        // Last, so its lock windows stack above everything else's.
        .add_module(LockscreenModule {
            settings: LockSettings {
                // Until the configuration has it: `MECHANIX_SUSPEND=1` or `=0`
                // overrides the default.
                suspend: std::env::var("MECHANIX_SUSPEND")
                    .map(|v| v == "1")
                    .unwrap_or(LockSettings::default().suspend),
                ..LockSettings::default()
            },
        })
        .add_module(IdleModule {
            settings: IdleSettings::default(),
        });

    // Home starts open.
    app.insert_resource(OpenCovers(vec![HOME_ID]));
    suspend_keyboard_zone(&mut app, true);
    app.system(on_container_shown)
        .system(on_wake)
        .system(on_lock);

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty apps, each with its own SVG icon (about what the 4a has), so
    /// the budget doesn't depend on the host's installed apps.
    fn fixture_drawer() -> AppDrawer {
        let data = std::env::temp_dir().join(format!("launcher-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let apps = data.join("applications");
        let icons = data.join("icons/hicolor/scalable/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::create_dir_all(&icons).unwrap();
        for i in 0..20u32 {
            let id = format!("app{i:02}");
            std::fs::write(
                apps.join(format!("{id}.desktop")),
                format!(
                    "[Desktop Entry]\nType=Application\nName=App {}\nExec=true\nIcon={id}\n",
                    char::from(b'A' + i as u8)
                ),
            )
            .unwrap();
            // A distinct colour and shape per app, so no two icons share a slot.
            let (r, g, b) = ((i * 37) % 256, (i * 91) % 256, (i * 151) % 256);
            std::fs::write(
                icons.join(format!("{id}.svg")),
                format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" rx="{}" fill="#{r:02x}{g:02x}{b:02x}"/><circle cx="{}" cy="24" r="8" fill="#fff"/></svg>"##,
                    i % 12,
                    8 + i
                ),
            )
            .unwrap();
        }
        AppDrawer::from_data_dirs(vec![data])
    }

    /// The launcher's UI as `main` builds it, without the compositor and
    /// D-Bus: the theme, the font, the status icons, every nav window and
    /// screen, and the keyboard.
    fn launcher() -> App {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(RingModule::default())
            .add_module(TimerModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(OrientationModule)
            .add_module(InteractivityModule)
            .add_module(AnimationModule)
            .add_module(ScrollModule)
            .add_module(RenderModule::default())
            .add_module(RunningAppsModule)
            .insert_resource(Atlas::new());
        app.add_module(MechanixTheme::dark());
        let (regular, light) = {
            let mut atlas = app.resource_mut::<Atlas>();
            (
                atlas.add_font(GEIST_MONO).expect("GeistMono font loads"),
                atlas
                    .add_font(GEIST_MONO_LIGHT)
                    .expect("GeistMono Light font loads"),
            )
        };
        app.insert_resource(FontBook::new(regular).light(light));
        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("Status bar icons load");
        app.insert_resource(icons);
        // PresentationModule registers Role; it needs a live compositor.
        app.register_component::<Role>();
        app.add_module(navigation(NavConfig::default(), fixture_drawer()));
        keyboard::spawn_ui(&mut app, KEYBOARD_BOTTOM_GAP);
        app.tick();
        app
    }

    /// `(mono, colour)` pages: glyph and icon pages, then image pages.
    fn pages(app: &App) -> (u32, u32) {
        let atlas = app.resource::<Atlas>();
        let (mut mono, mut color) = (0, 0);
        for page in atlas.pages() {
            match atlas.class(page.id()) {
                Class::Glyph | Class::Icon => mono += 1,
                Class::Image => color += 1,
                Class::External => {}
            }
        }
        (mono, color)
    }

    /// Rebuild at `scale` and rescale every window's tree, as presentation
    /// does for a window's preferred scale.
    fn rescale(app: &mut App, scale: f32) {
        assert!(app.resource_mut::<Atlas>().set_scale(scale));
        let windows: Vec<NodeId> = app.resource::<Windows>().iter().collect();
        let mut targets = Vec::new();
        for w in windows {
            targets.push(w);
            targets.extend(app.descendants(w));
        }
        app.emit(Rescaled, targets);
        app.tick();
    }

    /// The whole launcher goes idle: rounds of tick and `BeforeWait`, as
    /// the runner's turns run them, leave no work pending, so the runner
    /// blocks rather than going round forever.
    #[test]
    fn the_launcher_goes_idle() {
        let mut app = launcher();
        let rounds = (0..10)
            .take_while(|_| {
                app.tick();
                app.signal(BeforeWait);
                app.flush();
                app.has_pending_work()
            })
            .count();
        assert!(rounds < 10, "work is still pending after ten rounds");
    }

    #[test]
    fn the_launcher_fits_its_budget_at_1x_and_2x() {
        let budget = BUDGET;
        let mut app = launcher();
        let at_1x = pages(&app);
        rescale(&mut app, 2.0);
        let at_2x = pages(&app);
        eprintln!("pages (mono, colour): 1x {at_1x:?}, 2x {at_2x:?}");
        for (scale, (mono, color)) in [(1, at_1x), (2, at_2x)] {
            assert!(
                mono <= budget.mono_pages && color <= budget.color_pages,
                "{scale}x: {mono} mono and {color} colour pages over {budget:?}"
            );
        }
    }
}
