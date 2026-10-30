//! `NavModule`: spawns the nav windows in order, builds every screen into
//! its container node, and applies the stack's first state.

use std::collections::HashMap;
use std::rc::Rc;

use mecha_wayland::prelude::*;

use super::apply::{Action, ContainerNodes, Nav, NavWindow, apply, on_resized};
use super::input::on_contact;
use super::tick::on_frame;
use crate::core::{AreaId, ContainerSpec, NavConfig, Stack, Trigger};

/// One nav window: its touch area, layer role, layout and containers.
pub struct WindowSpec {
    area: AreaId,
    role: Role,
    layout: LayoutStyle,
    containers: Vec<ContainerSpec>,
    no_input: bool,
    clear: Color,
}

pub fn nav_window(area: AreaId, role: Role, layout: LayoutStyle) -> WindowSpec {
    WindowSpec {
        area,
        role,
        layout,
        containers: Vec::new(),
        no_input: false,
        clear: Color::BLACK,
    }
}

impl WindowSpec {
    /// Adds a root container. Later containers draw over earlier ones.
    pub fn container(mut self, c: ContainerSpec) -> Self {
        self.containers.push(c);
        self
    }

    /// Takes no input at all: touches fall through to what is beneath.
    /// Applied on the window's first `Resized`.
    pub fn no_input(mut self) -> Self {
        self.no_input = true;
        self
    }

    /// Clears to transparent instead of black: where nothing is drawn,
    /// what is beneath shows. Input is unchanged.
    pub fn transparent(mut self) -> Self {
        self.clear = Color::TRANSPARENT;
        self
    }
}

/// The gesture navigation. Windows are spawned in the order they are
/// added, which is their stacking order within a layer.
///
/// Expects the host to have installed layout, paint and window (and
/// whatever the screens need, such as a `FontBook`), and for presentation,
/// Wayland with `ZwlrLayerShellV1` bound.
pub struct NavModule {
    cfg: NavConfig,
    windows: Vec<WindowSpec>,
    actions: Vec<(Box<dyn Trigger>, Action)>,
}

impl NavModule {
    pub fn new(cfg: NavConfig) -> Self {
        Self {
            cfg,
            windows: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// Runs `f` when a gesture commits `trigger`, moving no container: for
    /// things that own their window, such as the on-screen keyboard. It
    /// runs after the stack's state is applied.
    pub fn action(mut self, trigger: impl Trigger, f: impl Fn(&mut App) + 'static) -> Self {
        self.actions.push((Box::new(trigger), Rc::new(f)));
        self
    }

    pub fn window(mut self, w: WindowSpec) -> Self {
        self.windows.push(w);
        self
    }
}

/// Absolute, inset 0, the parent's full size.
fn full_size() -> LayoutStyle {
    LayoutStyle::default()
        .absolute()
        .inset(Insets::all(px(0.0)))
        .fill()
}

impl Module for NavModule {
    fn install(self, app: &mut App) {
        let mut roots = Vec::new();
        // Each root container's window index.
        let mut root_window = HashMap::new();
        let mut windows = Vec::new();
        let root = app.root();
        for (i, w) in self.windows.into_iter().enumerate() {
            for c in w.containers {
                root_window.insert(c.id, i);
                roots.push(c);
            }
            let layer = match &w.role {
                Role::Layer(l) => Some(l.layer),
                Role::Toplevel => None,
            };
            let node = app
                .spawn_with(
                    root,
                    window().title(w.area.0).layout(w.layout).clear(w.clear),
                    (w.role,),
                )
                .id();
            windows.push(NavWindow {
                node,
                layer,
                area: w.area,
                containers: Vec::new(),
                no_input: w.no_input,
            });
        }

        let (mut stack, rows) = Stack::new(self.cfg, roots);
        let actions = self
            .actions
            .into_iter()
            .map(|(trigger, f)| {
                stack.action(trigger);
                f
            })
            .collect();
        // In z order, so a parent's node exists before its children's.
        let mut built = Vec::new();
        for (id, parent, screen) in rows {
            let (window, parent_node) = match parent {
                None => {
                    let i = root_window[&id];
                    (i, windows[i].node)
                }
                Some(p) => {
                    let &(_, i, node) = built
                        .iter()
                        .find(|(c, _, _)| *c == p)
                        .expect("a parent is built before its children");
                    (i, node)
                }
            };
            let node = app.spawn(parent_node, div().style(full_size())).id();
            screen.build(app, node);
            windows[window].containers.push(id);
            built.push((id, window, node));
        }
        // After every child container, so each cover is its node's last child.
        let containers = built
            .into_iter()
            .map(|(id, window, node)| ContainerNodes {
                id,
                window,
                node,
                cover: app.spawn(node, div().style(full_size())).id(),
            })
            .collect();

        app.insert_resource(Nav {
            stack,
            windows,
            containers,
            sizes: HashMap::new(),
            pressed: Vec::new(),
            last_tick: None,
            actions,
        });
        app.system(on_resized);
        app.system(on_contact);
        app.system(on_frame);
        apply(app);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::{Duration, Instant};

    use mecha_wayland::mechanix_widgets::prelude::{FontBook, Text};
    use screen::{Screen, placeholder_page};
    use status_bar::GEIST_MONO_BYTES;

    use super::*;
    use crate::core::{
        AreaId, BOTTOM_EDGE, ContainerId, Direction, FADE, INSTANT, Input, NavConfig, Timing,
        container, swipe,
    };
    use crate::shell::apply::window_layers;
    use crate::shell::apply::{Nav, apply};

    pub(crate) const HOME_AREA: AreaId = AreaId("home");
    pub(crate) const PANEL_AREA: AreaId = AreaId("panel");

    pub(crate) const HOME: ContainerId = ContainerId("home");
    pub(crate) const DRAWER: ContainerId = ContainerId("drawer");
    pub(crate) const NOTIFICATIONS: ContainerId = ContainerId("notifications");

    struct Label(&'static str);
    impl Screen for Label {
        fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
            placeholder_page(app, parent, self.0)
        }
    }

    fn role(layer: Layer, namespace: &str) -> Role {
        Role::Layer(LayerRole {
            layer,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            exclusive_zone: 0,
            namespace: namespace.into(),
            keyboard_interactivity: KeyboardInteractivity::None,
        })
    }

    /// Home with its drawer: opens with a quick bottom swipe and a fade.
    pub(crate) fn home_spec(home_open: bool) -> ContainerSpec {
        let home = container(HOME, Label("Home"))
            .open(swipe(BOTTOM_EDGE, Direction::Up, Timing::Quick), FADE)
            .child(
                container(DRAWER, Label("Drawer"))
                    .open_close(swipe(HOME_AREA, Direction::Up, Timing::Any), INSTANT),
            );
        if home_open {
            home.initially_open()
        } else {
            home
        }
    }

    pub(crate) fn module(home_open: bool) -> NavModule {
        module_with(home_spec(home_open))
    }

    /// The test windows, with `home` as the home window's container.
    pub(crate) fn module_with(home: ContainerSpec) -> NavModule {
        NavModule::new(NavConfig::default())
            .window(
                nav_window(
                    HOME_AREA,
                    role(Layer::Bottom, "home"),
                    LayoutStyle::default(),
                )
                .container(home),
            )
            .window(nav_window(
                BOTTOM_EDGE,
                role(Layer::Top, "bottom-edge"),
                LayoutStyle::default().height(px(20.0)),
            ))
            .window(
                nav_window(
                    PANEL_AREA,
                    role(Layer::Overlay, "panel"),
                    LayoutStyle::default(),
                )
                .container(container(NOTIFICATIONS, Label("Notifications"))),
            )
    }

    pub(crate) fn app_with(m: NavModule) -> App {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(WindowModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());
        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));
        // PresentationModule registers Role; it needs a live compositor.
        app.register_component::<Role>();
        app.add_module(m);
        app.tick();
        app
    }

    pub(crate) fn node_of(app: &App, id: ContainerId) -> NodeId {
        app.resource::<Nav>().container(id).node
    }

    fn cover_of(app: &App, id: ContainerId) -> NodeId {
        app.resource::<Nav>().container(id).cover
    }

    pub(crate) fn display(app: &App, node: NodeId) -> Display {
        app.component::<LayoutStyle>(node).unwrap().display
    }

    /// Texts under `node` that are not inside another container's node.
    fn own_texts(app: &App, node: NodeId) -> usize {
        let others: Vec<NodeId> = [HOME, DRAWER, NOTIFICATIONS]
            .into_iter()
            .map(|id| node_of(app, id))
            .filter(|n| *n != node)
            .collect();
        app.descendants(node)
            .filter(|n| app.widget::<Text>(*n).is_some())
            .filter(|n| {
                let mut p = app.parent(*n);
                while let Some(a) = p {
                    if a == node {
                        return true;
                    }
                    if others.contains(&a) {
                        return false;
                    }
                    p = app.parent(a);
                }
                false
            })
            .count()
    }

    #[test]
    fn a_transparent_window_clears_to_transparent_and_the_rest_to_black() {
        let m = NavModule::new(NavConfig::default())
            .window(nav_window(
                HOME_AREA,
                role(Layer::Bottom, "home"),
                LayoutStyle::default(),
            ))
            .window(
                nav_window(
                    BOTTOM_EDGE,
                    role(Layer::Top, "bottom-edge"),
                    LayoutStyle::default().height(px(20.0)),
                )
                .transparent(),
            );
        let app = app_with(m);
        let clears: Vec<Color> = app
            .resource::<Windows>()
            .iter()
            .map(|w| app.widget::<Window>(w).unwrap().clear())
            .collect();
        assert_eq!(clears, vec![Color::BLACK, Color::TRANSPARENT]);
    }

    #[test]
    fn windows_spawn_in_declaration_order() {
        let app = app_with(module(true));
        let ids: Vec<_> = app.resource::<Windows>().iter().collect();
        assert_eq!(ids.len(), 3);
        let declared: Vec<_> = app
            .resource::<Nav>()
            .windows
            .iter()
            .map(|w| w.node)
            .collect();
        assert_eq!(ids, declared);
        let layers: Vec<_> = ids
            .iter()
            .map(
                |id| match app.component::<Role>(*id).expect("window has a Role") {
                    Role::Layer(l) => (l.layer, l.namespace.clone()),
                    _ => panic!("expected a layer role"),
                },
            )
            .collect();
        assert_eq!(
            layers,
            vec![
                (Layer::Bottom, "home".to_string()),
                (Layer::Top, "bottom-edge".to_string()),
                (Layer::Overlay, "panel".to_string()),
            ]
        );
    }

    #[test]
    fn hidden_windows_go_to_background_visible_to_their_own_layer() {
        let app = app_with(module(true));
        let nav = app.resource::<Nav>();
        let w = |i: usize| nav.windows[i].node;
        // Home is open; the panel is closed; the strip is never asked.
        assert_eq!(
            window_layers(nav),
            vec![(w(0), Layer::Bottom), (w(2), Layer::Background)]
        );
        let app = app_with(module(false));
        let nav = app.resource::<Nav>();
        assert_eq!(
            window_layers(nav),
            vec![
                (nav.windows[0].node, Layer::Background),
                (nav.windows[2].node, Layer::Background)
            ]
        );
    }

    #[test]
    fn screens_build_inside_their_container_nodes() {
        let app = app_with(module(true));
        for id in [HOME, DRAWER, NOTIFICATIONS] {
            assert_eq!(own_texts(&app, node_of(&app, id)), 1, "{id:?}");
        }
        let home = node_of(&app, HOME);
        let drawer = node_of(&app, DRAWER);
        assert!(app.descendants(home).any(|n| n == drawer));
        let windows = &app.resource::<Nav>().windows;
        assert_eq!(app.parent(home), Some(windows[0].node));
        assert_eq!(
            app.parent(node_of(&app, NOTIFICATIONS)),
            Some(windows[2].node)
        );
        // The cover is drawn over everything in its container, children too.
        let children = app.children(home).unwrap();
        assert_eq!(children.last(), Some(&cover_of(&app, HOME)));
    }

    #[test]
    fn closed_containers_are_hidden_nodes() {
        let app = app_with(module(true));
        assert_eq!(display(&app, node_of(&app, DRAWER)), Display::Hidden);
        assert_eq!(display(&app, node_of(&app, NOTIFICATIONS)), Display::Hidden);
        assert_eq!(display(&app, node_of(&app, HOME)), Display::Flex);
    }

    #[test]
    fn cover_follows_visual() {
        let mut app = app_with(module(false));
        assert_eq!(display(&app, node_of(&app, HOME)), Display::Hidden);
        {
            let mut nav = app.resource_mut::<Nav>();
            let t0 = Instant::now();
            let size = Size::new(600.0, 20.0);
            nav.stack.handle(Input::Down {
                contact: 1,
                area: BOTTOM_EDGE,
                area_size: size,
                pos: Point { x: 300.0, y: 10.0 },
                t: t0,
            });
            for i in 1..=12u32 {
                nav.stack.handle(Input::Move {
                    contact: 1,
                    pos: Point {
                        x: 300.0,
                        y: 10.0 - 10.0 * i as f32,
                    },
                    t: t0 + Duration::from_millis(8 * i as u64),
                });
            }
            nav.stack.handle(Input::Up {
                contact: 1,
                t: t0 + Duration::from_millis(96),
            });
            nav.stack.handle(Input::Tick {
                dt: Duration::from_millis(80),
            });
        }
        apply(&mut app);
        let cover = app.resource::<Nav>().stack.visual(HOME).cover;
        assert_eq!(cover, 0.5);
        assert_eq!(display(&app, node_of(&app, HOME)), Display::Flex);
        match app.component::<Paint>(cover_of(&app, HOME)).unwrap() {
            Paint::Quad(q) => {
                assert_eq!(q.color.a, cover);
                assert!(!q.is_opaque);
            }
            _ => panic!("cover paints a quad"),
        }
    }
}
