use app_drawer::{AppDrawer, GEIST_MONO, GEIST_MONO_LIGHT};
use apps::RegistryModule;
use home::Home;
use keyboard::{KeyboardModule, hide_keyboard, show_keyboard, suspend_keyboard_zone};
use mecha_wayland::mechanix_widgets::prelude::FontBook;
use mecha_wayland::prelude::*;
use nav::core::{
    AreaId, BOTTOM_EDGE, ContainerId, Direction, ExternalEvent, FADE, INSTANT, NavConfig,
    OnExternal, Span, TOP_EDGE, Timing, container, gated, swipe,
};
use nav::shell::{ContainerShown, NavModule, nav_window};
use notifications::{Notifications, NotificationsModule};
use quick_settings::{QuickSettingsModule, QuickSettingsScreen};
use running_apps::{RunningApps, RunningAppsModule};
use status_bar::{
    DEFAULT_BAR_HEIGHT, DEFAULT_ICON_SIZE, StatusBarModule, StatusBarScreen, StatusIcons,
};

const HOME: AreaId = AreaId("home");
const RUNNING_APPS: AreaId = AreaId("running-apps");
const PANEL: AreaId = AreaId("panel");
const STATUS_BAR: AreaId = AreaId("status-bar");

const HOME_ID: ContainerId = ContainerId("home");
const DRAWER: ContainerId = ContainerId("drawer");
const RUNNING: ContainerId = ContainerId("running");
const NOTIFICATIONS: ContainerId = ContainerId("notifications");
const QUICK_SETTINGS: ContainerId = ContainerId("quick_settings");
const STATUS: ContainerId = ContainerId("status");

/// The launcher screens that cover apps. While any is open the keyboard
/// reserves no space, so they and the apps under them keep their full
/// size; opening one hides the keyboard.
const COVERS: [ContainerId; 4] = [HOME_ID, RUNNING, NOTIFICATIONS, QUICK_SETTINGS];

/// The [`COVERS`] open now.
struct OpenCovers(Vec<ContainerId>);
impl Resource for OpenCovers {}

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
    // Hiding clears the zone first, so the app under resizes as the
    // screen comes up.
    if s.shown {
        hide_keyboard(app);
    }
    suspend_keyboard_zone(app, any_open);
}

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

fn role(layer: Layer, anchor: Anchor, exclusive_zone: i32, namespace: &str) -> Role {
    Role::Layer(LayerRole {
        layer,
        anchor,
        exclusive_zone,
        namespace: namespace.into(),
        keyboard_interactivity: KeyboardInteractivity::None,
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

/// A panel that pulls down from its half of the top edge and closes on an up swipe.
fn panel(id: ContainerId, screen: impl screen::Screen, span: Span) -> nav::core::ContainerSpec {
    container(id, screen)
        .open(
            swipe(TOP_EDGE, Direction::Down, Timing::Any).span(span),
            INSTANT,
        )
        .close(swipe(PANEL, Direction::Up, Timing::Any), INSTANT)
}

fn navigation(cfg: NavConfig, drawer: AppDrawer) -> NavModule {
    let strip = LayoutStyle::default().height(px(cfg.bottom_strip_height));
    let bar = LayoutStyle::default().height(px(DEFAULT_BAR_HEIGHT));
    let sides = Anchor::LEFT | Anchor::RIGHT;

    // The drawer closes on a down drag only while its list rests at the top;
    // further down, the drag scrolls.
    let at_top = drawer.at_top();
    let home = container(HOME_ID, Home)
        .open(bottom_quick(), FADE)
        .close(OnExternal(ExternalEvent::AppLaunched), INSTANT)
        .child(
            container(DRAWER, drawer)
                .open(swipe(HOME, Direction::Up, Timing::Any), INSTANT)
                .close(
                    gated(swipe(HOME, Direction::Down, Timing::Any), at_top),
                    INSTANT,
                )
                .close(bottom_quick(), INSTANT),
        )
        .initially_open();
    let running = container(RUNNING, RunningApps::new())
        .open(
            swipe(BOTTOM_EDGE, Direction::Up, Timing::Held).span(Span::Middle),
            INSTANT,
        )
        .close(bottom_quick(), INSTANT)
        .close(OnExternal(ExternalEvent::AppLaunched), INSTANT);

    NavModule::new(cfg)
        .action(bottom_corner(Span::LeftCorner), show_keyboard)
        .action(bottom_corner(Span::RightCorner), show_keyboard)
        .window(
            nav_window(
                HOME,
                role(Layer::Top, ALL_SIDES, 0, "home"),
                LayoutStyle::default(),
            )
            .container(home),
        )
        .window(
            nav_window(
                RUNNING_APPS,
                role(Layer::Top, ALL_SIDES, 0, "running-apps"),
                LayoutStyle::default(),
            )
            .container(running),
        )
        .window(
            nav_window(
                BOTTOM_EDGE,
                // Above the keyboard, in the gap it leaves under its keys;
                // zone -1 keeps it on the edge rather than above the
                // keyboard's zone.
                role(Layer::Overlay, Anchor::BOTTOM | sides, -1, "bottom-edge"),
                strip,
            )
            .transparent(),
        )
        .window(
            nav_window(
                PANEL,
                role(Layer::Top, ALL_SIDES, 0, "panel"),
                LayoutStyle::default(),
            )
            .container(panel(NOTIFICATIONS, Notifications, Span::LeftHalf))
            .container(panel(QUICK_SETTINGS, QuickSettingsScreen, Span::RightHalf)),
        )
        .window(
            nav_window(
                TOP_EDGE,
                role(Layer::Overlay, Anchor::TOP | sides, -1, "top-edge"),
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
                    "status-bar",
                ),
                bar,
            )
            .container(container(STATUS, StatusBarScreen).initially_open())
            .no_input(),
        )
}

fn main() {
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
            budget: Budget::default(),
        });

    let cfg = NavConfig::default();
    let bottom_gap = cfg.bottom_strip_height;
    app.add_module(navigation(cfg, AppDrawer::new()))
        .add_module(StatusBarModule)
        .add_module(NotificationsModule)
        .add_module(KeyboardModule { bottom_gap })
        .add_module(QuickSettingsModule);

    // Home starts open.
    app.insert_resource(OpenCovers(vec![HOME_ID]));
    suspend_keyboard_zone(&mut app, true);
    app.system(on_container_shown);

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
            .add_module(PaintModule)
            .add_module(WindowModule)
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
        keyboard::spawn_ui(&mut app, NavConfig::default().bottom_strip_height);
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

    #[test]
    fn the_launcher_fits_the_default_budget_at_1x_and_2x() {
        let budget = Budget::default();
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
