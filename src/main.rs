use app_drawer::AppDrawer;
use home::Home;
use keyboard::{KeyboardModule, show_keyboard};
use mecha_wayland::mechanix_widgets::prelude::FontBook;
use mecha_wayland::prelude::*;
use nav::core::{
    AreaId, BOTTOM_EDGE, ContainerId, Direction, FADE, INSTANT, NavConfig, Span, TOP_EDGE, Timing,
    container, swipe,
};
use nav::shell::{NavModule, nav_window};
use notifications::{Notifications, NotificationsModule};
use quick_settings::{QuickSettingsModule, QuickSettingsScreen};
use running_apps::RunningApps;
use search::Search;
use status_bar::{
    DEFAULT_BAR_HEIGHT, DEFAULT_ICON_SIZE, GEIST_MONO_BYTES, StatusBarModule, StatusBarScreen,
    StatusIcons,
};

const HOME: AreaId = AreaId("home");
const RUNNING_APPS: AreaId = AreaId("running-apps");
const PANEL: AreaId = AreaId("panel");
const STATUS_BAR: AreaId = AreaId("status-bar");

const HOME_ID: ContainerId = ContainerId("home");
const DRAWER: ContainerId = ContainerId("drawer");
const SEARCH: ContainerId = ContainerId("search");
const RUNNING: ContainerId = ContainerId("running");
const NOTIFICATIONS: ContainerId = ContainerId("notifications");
const QUICK_SETTINGS: ContainerId = ContainerId("quick_settings");
const STATUS: ContainerId = ContainerId("status");

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

fn navigation(cfg: NavConfig) -> NavModule {
    let strip = LayoutStyle::default().height(px(cfg.bottom_strip_height));
    let bar = LayoutStyle::default().height(px(DEFAULT_BAR_HEIGHT));
    let sides = Anchor::LEFT | Anchor::RIGHT;

    let home = container(HOME_ID, Home)
        .open(bottom_quick(), FADE)
        .child(
            container(DRAWER, AppDrawer)
                .open_close(swipe(HOME, Direction::Up, Timing::Any), INSTANT)
                .close(bottom_quick(), INSTANT),
        )
        .child(
            container(SEARCH, Search)
                .open_close(swipe(HOME, Direction::Right, Timing::Any), INSTANT)
                .close(bottom_quick(), INSTANT),
        )
        .initially_open();
    let running = container(RUNNING, RunningApps)
        .open(
            swipe(BOTTOM_EDGE, Direction::Up, Timing::Held).span(Span::Middle),
            INSTANT,
        )
        .close(bottom_quick(), INSTANT);

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
        .window(nav_window(
            BOTTOM_EDGE,
            role(Layer::Top, Anchor::BOTTOM | sides, 0, "bottom-edge"),
            strip,
        ))
        .window(
            nav_window(
                PANEL,
                role(Layer::Top, ALL_SIDES, 0, "panel"),
                LayoutStyle::default(),
            )
            .container(panel(NOTIFICATIONS, Notifications, Span::LeftHalf))
            .container(panel(QUICK_SETTINGS, QuickSettingsScreen, Span::RightHalf)),
        )
        .window(nav_window(
            TOP_EDGE,
            role(Layer::Overlay, Anchor::TOP | sides, -1, "top-edge"),
            bar.clone(),
        ))
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
        .add_module(RenderModule::default())
        .insert_resource(Atlas::new());

    app.add_module(MechanixTheme::dark());

    let font = app
        .resource_mut::<Atlas>()
        .add_font(GEIST_MONO_BYTES)
        .expect("GeistMono font loads");
    app.insert_resource(FontBook::new(font));

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
        .add_module(PresentationModule {
            app_id: "mechanix.launcher".into(),
            budget: Budget::default(),
        });

    app.add_module(navigation(NavConfig::default()))
        .add_module(StatusBarModule)
        .add_module(NotificationsModule)
        .add_module(KeyboardModule)
        .add_module(QuickSettingsModule);

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .add_module(RenderModule::default())
            .insert_resource(Atlas::new());
        app.add_module(MechanixTheme::dark());
        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono font loads");
        app.insert_resource(FontBook::new(font));
        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("Status bar icons load");
        app.insert_resource(icons);
        // PresentationModule registers Role; it needs a live compositor.
        app.register_component::<Role>();
        app.add_module(navigation(NavConfig::default()));
        keyboard::spawn_ui(&mut app);
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
