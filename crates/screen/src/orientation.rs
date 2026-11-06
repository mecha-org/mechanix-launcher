//! Portrait or landscape: one value for the whole launcher.
//!
//! [`OrientationModule`] keeps a single [`Orientation`] resource, decided
//! from the screen's shape, and sends [`OrientationChanged`] to every node
//! when it flips, and to each new node spawned while it is landscape. So
//! a widget builds its portrait layout and needs only one handler:
//!
//! ```ignore
//! s.on::<OrientationChanged>(me, |ctx, e| { /* restyle for e.0 */ });
//! ```
//!
//! A system reads `app.resource::<Orientation>()` and can register for
//! `OnChanged<Orientation>`.
//!
//! The screen's shape is the largest window's: the launcher's full-screen
//! views cover the output, while bars, edge strips and the keyboard are
//! thin slices of it whose own shape says nothing about the screen.

use mecha_wayland::prelude::*;

/// The shape of the screen. Portrait (taller than wide, or square) until
/// the first layout says otherwise.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl Resource for Orientation {}

impl Orientation {
    /// Landscape when strictly wider than tall.
    pub fn of(rect: Rect) -> Self {
        if rect.width() > rect.height() {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }

    pub fn is_landscape(self) -> bool {
        self == Orientation::Landscape
    }

    pub fn is_portrait(self) -> bool {
        self == Orientation::Portrait
    }
}

/// Emitted at every node when the [`Orientation`] flips, carrying the new
/// one, and at each node spawned while it is landscape. Never emitted for
/// the initial portrait, which is what widgets build in.
#[derive(Clone, Copy, Debug)]
pub struct OrientationChanged(pub Orientation);
impl Event for OrientationChanged {}

/// Inserts the [`Orientation`] resource and keeps it current. Installs
/// after `WindowModule` and `LayoutModule`.
pub struct OrientationModule;

impl Module for OrientationModule {
    fn install(self, app: &mut App) {
        app.init_resource::<Orientation>()
            .system(on_spawned)
            .system(on_layout);
    }
}

/// The largest laid-out window: the one that stands for the screen.
fn screen_rect(app: &App) -> Option<Rect> {
    app.resource::<Windows>()
        .iter()
        .filter_map(|id| app.component::<ComputedLayout>(id).map(|l| l.rect))
        .filter(|r| r.width() > 0.0 && r.height() > 0.0)
        .max_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
}

/// A node spawned while landscape hears so, since it built portrait.
fn on_spawned(app: &mut App, s: &Spawned) {
    let orientation = *app.resource::<Orientation>();
    if orientation.is_landscape() {
        app.emit(OrientationChanged(orientation), s.id);
    }
}

/// After layout, updates the orientation if the screen's shape flipped and
/// tells every node.
fn on_layout(app: &mut App, _: &LayoutDone) {
    let Some(rect) = screen_rect(app) else {
        return;
    };
    let orientation = Orientation::of(rect);
    if *app.resource::<Orientation>() == orientation {
        return;
    }
    *app.resource_mut::<Orientation>() = orientation;
    let root = app.root();
    let nodes: Vec<NodeId> = std::iter::once(root).chain(app.descendants(root)).collect();
    app.emit(OrientationChanged(orientation), nodes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn app() -> App {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(OrientationModule);
        app
    }

    fn size(w: f32, h: f32) -> LayoutStyle {
        LayoutStyle::default().column().size(px(w), px(h))
    }

    fn settle(app: &mut App) {
        for _ in 0..3 {
            app.tick();
        }
    }

    #[test]
    fn the_largest_window_decides_not_a_thin_bar() {
        let mut app = app();
        let root = app.root();
        let screen = app.spawn(root, window().layout(size(480.0, 800.0)));
        // A status bar: wider than tall, but a slice of the screen.
        app.spawn(root, window().layout(size(480.0, 40.0)));
        settle(&mut app);
        assert_eq!(*app.resource::<Orientation>(), Orientation::Portrait);

        *app.component_mut::<LayoutStyle>(screen).unwrap() = size(800.0, 480.0);
        settle(&mut app);
        assert_eq!(*app.resource::<Orientation>(), Orientation::Landscape);
    }

    #[test]
    fn every_node_hears_each_flip_once() {
        let mut app = app();
        let root = app.root();
        let screen = app.spawn(root, window().layout(size(480.0, 800.0)));
        let heard = Rc::new(RefCell::new(Vec::new()));
        let log = heard.clone();
        let leaf = app.spawn(screen, Listener(log));
        settle(&mut app);
        assert!(
            heard.borrow().is_empty(),
            "no event for the initial portrait"
        );

        *app.component_mut::<LayoutStyle>(screen).unwrap() = size(800.0, 480.0);
        settle(&mut app);
        *app.component_mut::<LayoutStyle>(screen).unwrap() = size(480.0, 800.0);
        settle(&mut app);
        assert_eq!(
            *heard.borrow(),
            [Orientation::Landscape, Orientation::Portrait]
        );
        assert!(app.widget::<Listener>(leaf).is_some());
    }

    #[test]
    fn a_node_spawned_in_landscape_hears_it() {
        let mut app = app();
        let root = app.root();
        let screen = app.spawn(root, window().layout(size(800.0, 480.0)));
        settle(&mut app);
        let heard = Rc::new(RefCell::new(Vec::new()));
        app.spawn(screen, Listener(heard.clone()));
        settle(&mut app);
        assert_eq!(*heard.borrow(), [Orientation::Landscape]);
    }

    struct Listener(Rc<RefCell<Vec<Orientation>>>);

    impl Build for Listener {
        type Widget = Listener;
    }

    impl Widget for Listener {
        type Builder = Listener;

        fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
            s.on::<OrientationChanged>(me, |ctx, e| ctx.me().0.borrow_mut().push(e.0));
            b
        }
    }
}
