use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use apps::{AppKey, DesktopApp, IconDirs, Registry, RegistryChanged, RowIcon, RunningApp};
use mecha_wayland::mechanix_widgets::prelude::FontBook;
use mecha_wayland::prelude::*;
use screen::{AppLaunched, Hidden, Screen, Shown};

use crate::card::Card;
use crate::thumbs::{Place, ThumbKey, ThumbSink};
use crate::view::View;
use crate::{RunningApps, RunningAppsModule};

const W: f32 = 390.0;
const H: f32 = 844.0;

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#0a0"/></svg>"##;

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Create(ToplevelId),
    Set(ToplevelId, Place),
    Destroy(ToplevelId),
}

/// Records every call, keyed by the toplevel each thumbnail shows.
#[derive(Default)]
struct Sink {
    calls: RefCell<Vec<Call>>,
    live: RefCell<HashMap<ThumbKey, ToplevelId>>,
    next: Cell<ThumbKey>,
}

impl ThumbSink for Rc<Sink> {
    fn create(&self, _: &mut App, _: NodeId, toplevel: ToplevelId) -> Option<ThumbKey> {
        let key = self.next.get();
        self.next.set(key + 1);
        self.live.borrow_mut().insert(key, toplevel);
        self.calls.borrow_mut().push(Call::Create(toplevel));
        Some(key)
    }

    fn set(&self, _: &mut App, thumb: ThumbKey, place: Place) {
        let id = self.live.borrow()[&thumb];
        self.calls.borrow_mut().push(Call::Set(id, place));
    }

    fn destroy(&self, _: &mut App, thumb: ThumbKey) {
        let id = self.live.borrow_mut().remove(&thumb).expect("destroyed once");
        self.calls.borrow_mut().push(Call::Destroy(id));
    }
}

impl Sink {
    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    fn live(&self) -> Vec<ToplevelId> {
        let mut v: Vec<_> = self.live.borrow().values().copied().collect();
        v.sort_by_key(|t| format!("{t:?}"));
        v
    }
}

fn id(n: u32) -> ToplevelId {
    ToplevelId::from_raw(n)
}

fn data_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("running-apps-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    let icons = d.join("icons/hicolor/scalable/apps");
    fs::create_dir_all(&icons).unwrap();
    fs::write(icons.join("camera.svg"), SVG).unwrap();
    d
}

fn entries() -> Vec<DesktopApp> {
    let entry = |id: &str, name: &str| DesktopApp {
        id: format!("{id}.desktop"),
        name: name.into(),
        exec: vec![id.into()],
        icon: Some(id.into()),
        startup_wm_class: None,
    };
    vec![entry("camera", "Camera"), entry("clock", "Clock"), entry("files", "Files")]
}

/// `(app id, entry, windows)`, most recently used first; the first window
/// is the front.
fn registry(apps: &[(&str, Option<usize>, &[u32])]) -> Registry {
    let apps = apps
        .iter()
        .enumerate()
        .map(|(i, &(app_id, entry, windows))| RunningApp {
            key: AppKey::AppId(app_id.into()),
            entry,
            windows: windows.iter().map(|&n| id(n)).collect(),
            front: id(windows[0]),
            last_used: 100 - i as u64,
        })
        .collect();
    Registry::fixture(entries(), apps)
}

fn three() -> Registry {
    registry(&[
        ("clock", Some(1), &[1]),
        ("camera", Some(0), &[2]),
        ("files", Some(2), &[3]),
    ])
}

#[derive(Default)]
struct Launched(u32);
impl Resource for Launched {}

fn count_launch(app: &mut App, _: &AppLaunched) {
    app.resource_mut::<Launched>().0 += 1;
}

struct Ui {
    app: App,
    win: NodeId,
    view: NodeId,
    sink: Rc<Sink>,
    activated: Rc<RefCell<Vec<ToplevelId>>>,
    closed: Rc<RefCell<Vec<ToplevelId>>>,
}

fn ui(name: &str, registry: Registry) -> Ui {
    let mut app = App::new();
    app.insert_resource(registry);
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(InteractivityModule)
        .add_module(AnimationModule)
        .add_module(ScrollModule)
        .add_module(RunningAppsModule)
        .add_module(MechanixTheme::dark())
        .init_resource::<Launched>()
        .system(count_launch);
    app.insert_resource(Atlas::new());
    let regular = app
        .resource_mut::<Atlas>()
        .add_font(assets::fonts::GEIST_MONO)
        .unwrap();
    app.insert_resource(FontBook::new(regular));
    let root = app.root();
    let win = app
        .spawn(
            root,
            window().layout(LayoutStyle::default().column().size(px(W), px(H))),
        )
        .id();

    let sink = Rc::new(Sink::default());
    let activated: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
    let closed: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
    let (a, c) = (activated.clone(), closed.clone());
    let dirs = vec![data_dir(name)];
    let screen = RunningApps::with_icon_dirs(IconDirs::new(&dirs, None))
        .activator(move |_, t| a.borrow_mut().push(t))
        .closer(move |_, t| c.borrow_mut().push(t))
        .thumbs(sink.clone());
    let view = screen.build(&mut app, win);
    app.tick();
    app.tick();
    Ui {
        app,
        win,
        view,
        sink,
        activated,
        closed,
    }
}

impl Ui {
    fn cards(&mut self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self
            .app
            .descendants(self.view)
            .filter(|&n| self.app.widget::<Card>(n).is_some())
            .collect();
        out.dedup();
        out
    }

    fn card(&self, n: NodeId) -> &Card {
        self.app.widget::<Card>(n).unwrap()
    }

    fn names(&mut self) -> Vec<String> {
        self.cards()
            .into_iter()
            .map(|c| self.card(c).name().to_string())
            .collect()
    }

    fn empty_shown(&self) -> bool {
        let empty = self.app.widget::<View>(self.view).unwrap().empty();
        self.app.component::<LayoutStyle>(empty).unwrap().display != Display::Hidden
    }

    fn emit(&mut self, e: impl Event) {
        let v = self.view;
        self.app.emit(e, v);
        self.app.tick();
    }

    fn frame(&mut self) {
        self.app.signal(Frame(self.win));
        self.app.flush();
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

    fn set_registry(&mut self, r: Registry) {
        self.app.insert_resource(r);
        self.app.signal(RegistryChanged);
        self.app.tick();
        self.app.tick();
    }

    fn contact(&mut self, phase: ContactPhase, at: Point) {
        let window = self.win;
        self.app.signal(ContactInput {
            window,
            contact: ContactId::Touch(0),
            phase,
            position: at,
            time: 0,
        });
        self.app.flush();
    }

    /// A finger drag from the middle of `card`'s header by `dx`.
    fn drag(&mut self, card: NodeId, dx: f32) {
        let header = self.card(card).header();
        let r = self.app.component::<ComputedLayout>(header).unwrap().rect;
        let at = Point::new(r.x() + r.width() / 2.0 - 60.0, r.y() + r.height() / 2.0);
        self.contact(ContactPhase::Moved, at);
        self.contact(ContactPhase::Pressed, at);
        for i in 1..=4 {
            let step = Point::new(at.x + dx * i as f32 / 4.0, at.y);
            self.contact(ContactPhase::Moved, step);
        }
        let end = Point::new(at.x + dx, at.y);
        self.contact(ContactPhase::Released, end);
        self.app.tick();
    }
}

fn sets(calls: &[Call]) -> Vec<(ToplevelId, Place)> {
    calls
        .iter()
        .filter_map(|c| match c {
            Call::Set(t, p) => Some((*t, *p)),
            _ => None,
        })
        .collect()
}

#[test]
fn one_card_per_app_in_registry_order() {
    let mut ui = ui("order", three());
    assert_eq!(ui.names(), ["Clock", "Camera", "Files"]);
    assert!(!ui.empty_shown());
}

#[test]
fn unmatched_app_shows_app_id_and_generic_icon() {
    let mut ui = ui(
        "unmatched",
        registry(&[("org.example.thing", None, &[4]), ("camera", Some(0), &[5])]),
    );
    assert_eq!(ui.names(), ["org.example.thing", "Camera"]);
    let cards = ui.cards();
    assert!(matches!(ui.card(cards[0]).icon(), RowIcon::Glyph(_)));
    assert!(matches!(ui.card(cards[1]).icon(), RowIcon::Image(_)));
}

#[test]
fn empty_shows_no_apps_open() {
    let mut ui = ui("empty", registry(&[]));
    assert!(ui.cards().is_empty());
    assert!(ui.empty_shown());
}

#[test]
fn last_card_closed_leaves_empty_text() {
    let mut ui = ui("last", registry(&[("clock", Some(1), &[1])]));
    ui.emit(Shown);
    let card = ui.cards()[0];
    let close = ui.card(card).close_button();
    ui.click(close);
    assert_eq!(*ui.closed.borrow(), [id(1)]);

    ui.set_registry(registry(&[]));
    assert!(ui.cards().is_empty());
    assert!(ui.empty_shown());
    assert_eq!(ui.app.resource::<Launched>().0, 0, "the screen stays open");
}

#[test]
fn tap_activates_front_and_signals() {
    let mut ui = ui("tap", three());
    let camera = ui.cards()[1];
    ui.click(camera);
    assert_eq!(*ui.activated.borrow(), [id(2)]);
    assert_eq!(ui.app.resource::<Launched>().0, 1);
}

#[test]
fn x_closes_every_window_and_dims() {
    let mut ui = ui("x", registry(&[("files", Some(2), &[7, 8]), ("clock", Some(1), &[9])]));
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    assert_eq!(*ui.closed.borrow(), [id(7), id(8)]);
    assert!(ui.activated.borrow().is_empty());
    assert_eq!(ui.app.resource::<Launched>().0, 0);

    // Still there, dimmed, deaf to taps.
    assert_eq!(ui.cards().len(), 2);
    assert!(ui.card(files).closing());
    assert!(ui.card(files).opacity() < 1.0);
    ui.click(files);
    assert!(ui.activated.borrow().is_empty());
    ui.click(close);
    assert_eq!(ui.closed.borrow().len(), 2, "a second ✕ asks nothing more");

    // The preview fades with it.
    ui.sink.take();
    ui.frame();
    let alpha = sets(&ui.sink.take())
        .into_iter()
        .find(|(t, _)| *t == id(7))
        .map(|(_, p)| p.alpha);
    assert!(alpha.is_some_and(|a| a < 1.0), "{alpha:?}");

    // One of two windows goes: still closing.
    ui.set_registry(registry(&[("files", Some(2), &[8]), ("clock", Some(1), &[9])]));
    let files = ui.cards()[0];
    assert!(ui.card(files).closing());
    // Gone.
    ui.set_registry(registry(&[("clock", Some(1), &[9])]));
    assert_eq!(ui.names(), ["Clock"]);
}

#[test]
fn swipe_away_closes() {
    let mut ui = ui("swipe", registry(&[("files", Some(2), &[7, 8]), ("clock", Some(1), &[9])]));
    ui.emit(Shown);
    let files = ui.cards()[0];

    ui.drag(files, 30.0);
    assert!(ui.closed.borrow().is_empty(), "a short drag closes nothing");
    assert!(ui.activated.borrow().is_empty(), "nor taps");
    assert!(!ui.card(files).closing());
    assert_eq!(ui.card(files).offset(), 0.0, "it springs back");

    ui.drag(files, 200.0);
    assert_eq!(*ui.closed.borrow(), [id(7), id(8)]);
    assert!(ui.activated.borrow().is_empty());
    assert!(ui.card(files).closing());
}

#[test]
fn swiping_card_fades_its_preview() {
    let mut ui = ui("fade", registry(&[("clock", Some(1), &[9])]));
    ui.emit(Shown);
    ui.frame();
    ui.sink.take();
    let card = ui.cards()[0];
    let header = ui.card(card).header();
    let r = ui.app.component::<ComputedLayout>(header).unwrap().rect;
    let at = Point::new(r.x() + 40.0, r.y() + r.height() / 2.0);
    ui.contact(ContactPhase::Moved, at);
    ui.contact(ContactPhase::Pressed, at);
    ui.contact(ContactPhase::Moved, Point::new(at.x + 80.0, at.y));
    ui.app.tick();
    ui.frame();
    let s = sets(&ui.sink.take());
    assert_eq!(s.len(), 1);
    let (_, p) = s[0];
    assert!(p.alpha < 1.0 && p.alpha > 0.0, "{p:?}");
    assert!(p.x > 24, "the preview moves with the card: {p:?}");
}

#[test]
fn shown_creates_thumbnails_and_hidden_destroys_them() {
    let mut ui = ui("thumbs", three());
    assert!(ui.sink.take().is_empty(), "nothing before Shown");
    ui.emit(Shown);
    assert_eq!(
        ui.sink.take(),
        [Call::Create(id(1)), Call::Create(id(2)), Call::Create(id(3))]
    );

    // The first frame places each, at the box's window-local position.
    ui.frame();
    let s = sets(&ui.sink.take());
    assert_eq!(s.len(), 3);
    let cards = ui.cards();
    for (card, (t, p)) in cards.iter().zip(&s) {
        let hole = ui.card(*card).hole();
        let r = ui.app.component::<Layout>(hole).unwrap().rect;
        assert_eq!(*t, ui.card(*card).front());
        assert_eq!((p.x, p.y), (r.x().round() as i32, r.y().round() as i32));
        assert!((p.scale - r.width() / W).abs() < 1e-4, "{p:?} {r:?}");
        assert!((r.height() / r.width() - H / W).abs() < 0.01, "box has the output's aspect");
        assert_eq!(p.alpha, 1.0);
    }

    ui.emit(Hidden);
    let calls = ui.sink.take();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| matches!(c, Call::Destroy(_))));
    assert!(ui.sink.live().is_empty());
    ui.frame();
    assert!(ui.sink.take().is_empty(), "nothing placed while hidden");

    // Reopening creates fresh ones.
    ui.emit(Shown);
    assert_eq!(ui.sink.live().len(), 3);
}

#[test]
fn scroll_moves_thumbnails() {
    let mut ui = ui("scroll", three());
    ui.emit(Shown);
    ui.frame();
    let before = sets(&ui.sink.take());
    ui.frame();
    assert!(ui.sink.take().is_empty(), "an idle frame sends nothing");

    let list = ui.app.widget::<View>(ui.view).unwrap().list();
    ui.app.component_mut::<Viewport>(list).unwrap().scroll.y = 50.0;
    ui.frame();
    let after = sets(&ui.sink.take());
    assert_eq!(after.len(), 3);
    for ((t0, p0), (t1, p1)) in before.iter().zip(&after) {
        assert_eq!(t0, t1);
        assert_eq!(p1.y, p0.y - 50);
        assert_eq!(p1.x, p0.x);
    }
}

#[test]
fn closed_app_drops_its_card() {
    let mut ui = ui("drop", three());
    ui.emit(Shown);
    ui.frame();
    ui.sink.take();
    ui.set_registry(registry(&[("clock", Some(1), &[1]), ("files", Some(2), &[3])]));
    assert_eq!(ui.names(), ["Clock", "Files"]);
    let calls = ui.sink.take();
    assert!(calls.contains(&Call::Destroy(id(2))), "{calls:?}");
    assert_eq!(ui.sink.live(), [id(1), id(3)]);
}

#[test]
fn front_change_replaces_the_thumbnail() {
    let mut ui = ui("front", registry(&[("files", Some(2), &[7, 8])]));
    ui.emit(Shown);
    ui.sink.take();
    ui.set_registry(registry(&[("files", Some(2), &[8, 7])]));
    let calls = ui.sink.take();
    assert!(calls.contains(&Call::Destroy(id(7))), "{calls:?}");
    assert!(calls.contains(&Call::Create(id(8))), "{calls:?}");
    assert_eq!(ui.sink.live(), [id(8)]);
}

#[test]
fn new_app_while_shown_gets_a_thumbnail() {
    let mut ui = ui("new", registry(&[("clock", Some(1), &[1])]));
    ui.emit(Shown);
    ui.set_registry(registry(&[("camera", Some(0), &[2]), ("clock", Some(1), &[1])]));
    assert_eq!(ui.names(), ["Camera", "Clock"]);
    assert_eq!(ui.sink.live(), [id(1), id(2)]);
}
