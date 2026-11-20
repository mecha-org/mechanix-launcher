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
    /// Windows whose commit was requested, in order.
    commits: RefCell<Vec<NodeId>>,
    /// Stands for a compositor without thumbnail views.
    unavailable: Cell<bool>,
}

impl ThumbSink for Rc<Sink> {
    fn available(&self, _: &App) -> bool {
        !self.unavailable.get()
    }

    fn create(&self, _: &mut App, _: NodeId, toplevel: ToplevelId) -> Option<ThumbKey> {
        if self.unavailable.get() {
            return None;
        }
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
        let id = self
            .live
            .borrow_mut()
            .remove(&thumb)
            .expect("destroyed once");
        self.calls.borrow_mut().push(Call::Destroy(id));
    }

    fn commit(&self, _: &mut App, window: NodeId) {
        self.commits.borrow_mut().push(window);
    }
}

impl Sink {
    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    fn commits(&self) -> Vec<NodeId> {
        std::mem::take(&mut *self.commits.borrow_mut())
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

/// The process of the app whose first window is `n`, in the fixtures.
fn pid(n: u32) -> u32 {
    1000 + n
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
        path: PathBuf::from(format!("/usr/share/applications/{id}.desktop")),
        dbus_activatable: false,
    };
    vec![
        entry("camera", "Camera"),
        entry("clock", "Clock"),
        entry("files", "Files"),
    ]
}

/// `(app id, entry, windows)`, most recently used first; the first window
/// is the front.
fn registry(apps: &[(&str, Option<usize>, &[u32])]) -> Registry {
    let apps: Vec<_> = apps.iter().map(|&(a, e, w)| (a, e, w, &[][..])).collect();
    registry_with_dialogs(&apps)
}

/// `(app id, entry, windows, dialogs)`.
type WithDialogs<'a> = (&'a str, Option<usize>, &'a [u32], &'a [u32]);

/// [`registry`], with each app's dialogs last.
fn registry_with_dialogs(apps: &[WithDialogs<'_>]) -> Registry {
    let apps = apps
        .iter()
        .enumerate()
        .map(|(i, &(app_id, entry, windows, dialogs))| RunningApp {
            // Without an entry, a process named by its app id.
            key: match entry {
                Some(_) => AppKey::Entry(app_id.into()),
                None => AppKey::Process(windows[0]),
            },
            entry,
            windows: windows.iter().map(|&n| id(n)).collect(),
            dialogs: dialogs.iter().map(|&n| id(n)).collect(),
            modal: false,
            pids: vec![pid(windows[0])],
            front: id(windows[0]),
            last_used: 100 - i as u64,
            title: if entry.is_some() {
                String::new()
            } else {
                app_id.into()
            },
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
    killed: Rc<RefCell<Vec<u32>>>,
}

fn ui(name: &str, registry: Registry) -> Ui {
    ui_with(name, registry, Rc::default())
}

fn ui_with(name: &str, registry: Registry, sink: Rc<Sink>) -> Ui {
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
    // Timers that never fire on their own: a test fires one with `timeout`.
    app.init_resource::<Timers>();
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

    let activated: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
    let closed: Rc<RefCell<Vec<ToplevelId>>> = Rc::default();
    let killed: Rc<RefCell<Vec<u32>>> = Rc::default();
    let (a, c, k) = (activated.clone(), closed.clone(), killed.clone());
    let dirs = vec![data_dir(name)];
    let screen = RunningApps::with_icon_dirs(IconDirs::new(&dirs, None))
        .activator(move |_, t| a.borrow_mut().push(t))
        .closer(move |_, t| c.borrow_mut().push(t))
        .killer(move |_, pids| k.borrow_mut().extend(pids))
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
        killed,
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

    /// `card`'s close wait runs out.
    fn timeout(&mut self, card: NodeId) {
        let id = self.card(card).wait();
        self.app.emit(Timeout { id }, card);
        self.app.tick();
    }

    fn set_registry(&mut self, r: Registry) {
        self.app.insert_resource(r);
        self.app.signal(RegistryChanged);
        self.app.tick();
        self.app.tick();
    }

    fn contact(&mut self, phase: ContactPhase, at: Point) {
        self.contact_by(0, phase, at);
    }

    fn contact_by(&mut self, touch: u32, phase: ContactPhase, at: Point) {
        let window = self.win;
        self.app.signal(ContactInput {
            window,
            contact: ContactId::Touch(touch),
            phase,
            position: at,
            time: 0,
        });
        self.app.flush();
    }

    /// A finger drag from `card`'s header, `dy` up (negative) or down.
    fn drag(&mut self, card: NodeId, dy: f32) {
        self.drag_by(0, card, Point::new(0.0, dy));
    }

    /// A finger drag from `card`'s header by `d`, with touch point `touch`.
    fn drag_by(&mut self, touch: u32, card: NodeId, d: Point) {
        let at = self.grip(card);
        self.contact_by(touch, ContactPhase::Moved, at);
        self.contact_by(touch, ContactPhase::Pressed, at);
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            let step = Point::new(at.x + d.x * t, at.y + d.y * t);
            self.contact_by(touch, ContactPhase::Moved, step);
        }
        let end = Point::new(at.x + d.x, at.y + d.y);
        self.contact_by(touch, ContactPhase::Released, end);
        self.app.tick();
    }

    /// Where a drag of `card` starts: left of its header's middle.
    fn grip(&self, card: NodeId) -> Point {
        let header = self.card(card).header();
        let r = self.app.component::<ComputedLayout>(header).unwrap().rect;
        Point::new(r.x() + r.width() / 2.0 - 60.0, r.y() + r.height() / 2.0)
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
    let mut ui = ui(
        "x",
        registry(&[("files", Some(2), &[7, 8]), ("clock", Some(1), &[9])]),
    );
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
    ui.set_registry(registry(&[
        ("files", Some(2), &[8]),
        ("clock", Some(1), &[9]),
    ]));
    let files = ui.cards()[0];
    assert!(ui.card(files).closing());
    // Gone.
    ui.set_registry(registry(&[("clock", Some(1), &[9])]));
    assert_eq!(ui.names(), ["Clock"]);
}

#[test]
fn a_dialog_opened_on_close_is_brought_forward() {
    let mut ui = ui(
        "asks",
        registry(&[("files", Some(2), &[7]), ("clock", Some(1), &[9])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    assert!(ui.card(files).closing());

    // "Save first?"
    ui.set_registry(registry_with_dialogs(&[
        ("files", Some(2), &[7], &[10]),
        ("clock", Some(1), &[9], &[]),
    ]));
    let files = ui.cards()[0];
    assert!(!ui.card(files).closing());
    assert_eq!(*ui.activated.borrow(), [id(10)]);
    assert_eq!(ui.app.resource::<Launched>().0, 1);
}

#[test]
fn a_dialog_opened_on_force_close_is_not_brought_forward() {
    let mut ui = ui("force-asks", registry(&[("files", Some(2), &[7])]));
    ui.emit(Shown);
    let files = ui.cards()[0];
    let kill = ui.card(files).kill_button();
    ui.click(kill);
    ui.set_registry(registry_with_dialogs(&[("files", Some(2), &[7], &[10])]));
    let files = ui.cards()[0];
    assert!(ui.card(files).closing(), "the kill comes anyway");
    assert!(ui.activated.borrow().is_empty());
}

#[test]
fn a_dialog_open_before_close_is_not_an_answer() {
    let mut ui = ui(
        "open-dialog",
        registry_with_dialogs(&[("files", Some(2), &[7, 8], &[10])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    ui.set_registry(registry_with_dialogs(&[("files", Some(2), &[7], &[10])]));
    let files = ui.cards()[0];
    assert!(ui.card(files).closing());
    assert!(ui.activated.borrow().is_empty());
}

#[test]
fn the_skull_kills_the_apps_processes_and_dims() {
    let mut ui = ui(
        "kill",
        registry(&[("files", Some(2), &[7]), ("clock", Some(1), &[9])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];
    let kill = ui.card(files).kill_button();
    ui.click(kill);
    assert_eq!(*ui.killed.borrow(), [pid(7)]);
    assert!(ui.closed.borrow().is_empty(), "no close is asked first");
    assert!(ui.activated.borrow().is_empty());
    assert!(ui.card(files).closing());
    ui.click(kill);
    // A second press kills nothing more.
    assert_eq!(ui.killed.borrow().len(), 1);
}

#[test]
fn without_a_process_there_is_no_skull() {
    let mut app = RunningApp {
        key: AppKey::Window(id(5)),
        entry: None,
        windows: vec![id(5)],
        dialogs: Vec::new(),
        modal: false,
        pids: Vec::new(),
        front: id(5),
        last_used: 1,
        title: "Scratch".into(),
    };
    let mut ui = ui("no-pid", Registry::fixture(entries(), vec![app.clone()]));
    ui.emit(Shown);
    let card = ui.cards()[0];
    let kill = ui.card(card).kill_button();
    let display = ui.app.component::<LayoutStyle>(kill).unwrap().display;
    assert_eq!(display, Display::Hidden);
    ui.click(kill);
    assert!(ui.killed.borrow().is_empty());
    assert!(!ui.card(card).closing());

    app.pids = vec![42];
    ui.set_registry(Registry::fixture(entries(), vec![app]));
    let card = ui.cards()[0];
    let kill = ui.card(card).kill_button();
    let display = ui.app.component::<LayoutStyle>(kill).unwrap().display;
    assert_ne!(display, Display::Hidden);
    ui.click(kill);
    assert_eq!(*ui.killed.borrow(), [42]);
}

#[test]
fn a_modal_dialog_already_open_is_brought_forward_not_closed() {
    let mut app = RunningApp {
        key: AppKey::Entry("files".into()),
        entry: Some(2),
        windows: vec![id(7)],
        dialogs: vec![id(10)],
        modal: true,
        pids: vec![pid(7)],
        front: id(7),
        last_used: 1,
        title: String::new(),
    };
    let mut ui = ui("modal", Registry::fixture(entries(), vec![app.clone()]));
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    assert!(ui.closed.borrow().is_empty(), "nothing is closed");
    assert!(!ui.card(files).closing());
    assert_eq!(*ui.activated.borrow(), [id(7)]);
    assert_eq!(ui.app.resource::<Launched>().0, 1);

    // Answered: the next ✕ closes.
    app.dialogs.clear();
    app.modal = false;
    ui.set_registry(Registry::fixture(entries(), vec![app]));
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    assert_eq!(*ui.closed.borrow(), [id(7)]);
}

#[test]
fn an_app_that_stays_open_is_brought_forward() {
    let mut ui = ui("stays", registry(&[("files", Some(2), &[7])]));
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    ui.timeout(files);
    assert!(!ui.card(files).closing());
    assert_eq!(*ui.activated.borrow(), [id(7)]);
    assert_eq!(ui.app.resource::<Launched>().0, 1);
    // It takes taps again.
    ui.click(files);
    assert_eq!(*ui.activated.borrow(), [id(7), id(7)]);
}

#[test]
fn an_app_that_stays_open_while_hidden_only_wakes_its_card() {
    let mut ui = ui("stays-hidden", registry(&[("files", Some(2), &[7])]));
    ui.emit(Shown);
    let files = ui.cards()[0];
    let close = ui.card(files).close_button();
    ui.click(close);
    ui.emit(Hidden);
    ui.timeout(files);
    assert!(!ui.card(files).closing());
    assert!(ui.activated.borrow().is_empty());
    assert_eq!(ui.app.resource::<Launched>().0, 0);
}

#[test]
fn swipe_away_closes() {
    let mut ui = ui(
        "swipe",
        registry(&[("files", Some(2), &[7, 8]), ("clock", Some(1), &[9])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];

    ui.drag(files, -30.0);
    assert!(ui.closed.borrow().is_empty(), "a short drag closes nothing");
    assert!(ui.activated.borrow().is_empty(), "nor taps");
    assert!(!ui.card(files).closing());
    assert_eq!(ui.card(files).offset(), 0.0, "it springs back");

    ui.drag(files, -200.0);
    assert_eq!(*ui.closed.borrow(), [id(7), id(8)]);
    assert!(ui.activated.borrow().is_empty());
    assert!(ui.card(files).closing());
}

#[test]
fn only_an_upward_swipe_closes() {
    let mut ui = ui(
        "updown",
        registry(&[("files", Some(2), &[7]), ("clock", Some(1), &[9])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];
    ui.drag(files, 200.0);
    assert!(ui.closed.borrow().is_empty(), "down closes nothing");
    assert_eq!(ui.card(files).offset(), 0.0);
    ui.drag_by(0, files, Point::new(-200.0, 0.0));
    assert!(
        ui.closed.borrow().is_empty(),
        "sideways scrolls, it closes nothing"
    );
    assert_eq!(ui.card(files).offset(), 0.0);
    assert!(!ui.card(files).closing());
}

#[test]
fn cards_sit_side_by_side() {
    let mut ui = ui("row", three());
    ui.emit(Shown);
    ui.frame();
    let rects: Vec<_> = ui
        .cards()
        .iter()
        .map(|&c| ui.app.component::<Layout>(c).unwrap().rect)
        .collect();
    for pair in rects.windows(2) {
        assert_eq!(pair[0].y(), pair[1].y(), "one row");
        assert!(pair[1].x() >= pair[0].x() + pair[0].width(), "{rects:?}");
    }
    assert!(
        rects[0].x() + rects[0].width() < W,
        "the next card peeks in"
    );
    assert!(
        rects[0].y() >= 0.0 && rects[0].y() + rects[0].height() <= H,
        "fits the height"
    );
}

#[test]
fn swiping_card_fades_its_preview() {
    let mut ui = ui("fade", registry(&[("clock", Some(1), &[9])]));
    ui.emit(Shown);
    ui.frame();
    let (_, rest) = sets(&ui.sink.take())[0];
    let card = ui.cards()[0];
    let header = ui.card(card).header();
    let r = ui.app.component::<ComputedLayout>(header).unwrap().rect;
    let at = Point::new(r.x() + 40.0, r.y() + r.height() / 2.0);
    ui.contact(ContactPhase::Moved, at);
    ui.contact(ContactPhase::Pressed, at);
    ui.contact(ContactPhase::Moved, Point::new(at.x, at.y - 80.0));
    ui.app.tick();
    ui.frame();
    let s = sets(&ui.sink.take());
    assert_eq!(s.len(), 1);
    let (_, p) = s[0];
    assert!(p.alpha < 1.0 && p.alpha > 0.0, "{p:?}");
    assert_eq!(p.y, rest.y - 80, "the preview moves with the card: {p:?}");
    assert_eq!(p.x, rest.x);
}

#[test]
fn shown_creates_thumbnails_and_hidden_destroys_them() {
    let mut ui = ui("thumbs", three());
    assert!(ui.sink.take().is_empty(), "nothing before Shown");
    ui.emit(Shown);
    assert_eq!(
        ui.sink.take(),
        [
            Call::Create(id(1)),
            Call::Create(id(2)),
            Call::Create(id(3))
        ]
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
        assert!(
            (r.height() / r.width() - H / W).abs() < 0.01,
            "box has the output's aspect"
        );
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
    ui.app.component_mut::<Viewport>(list).unwrap().scroll.x = 50.0;
    ui.frame();
    let after = sets(&ui.sink.take());
    assert_eq!(after.len(), 3);
    for ((t0, p0), (t1, p1)) in before.iter().zip(&after) {
        assert_eq!(t0, t1);
        assert_eq!(p1.x, p0.x - 50);
        assert_eq!(p1.y, p0.y);
    }
}

#[test]
fn closed_app_drops_its_card() {
    let mut ui = ui("drop", three());
    ui.emit(Shown);
    ui.frame();
    ui.sink.take();
    ui.set_registry(registry(&[
        ("clock", Some(1), &[1]),
        ("files", Some(2), &[3]),
    ]));
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
    ui.set_registry(registry(&[
        ("camera", Some(0), &[2]),
        ("clock", Some(1), &[1]),
    ]));
    assert_eq!(ui.names(), ["Camera", "Clock"]);
    assert_eq!(ui.sink.live(), [id(1), id(2)]);
}

#[test]
fn placing_requests_a_commit_and_an_idle_frame_does_not() {
    let mut ui = ui("commit", three());
    ui.emit(Shown);
    ui.frame();
    assert_eq!(ui.sink.commits(), [ui.win]);
    ui.frame();
    assert!(ui.sink.commits().is_empty());
}

#[test]
fn in_place_thumbnail_recreation_commits_on_an_idle_frame() {
    let mut ui = ui("recreate", registry(&[("files", Some(2), &[7, 8])]));
    ui.emit(Shown);
    ui.frame();
    let before = sets(&ui.sink.take());
    ui.sink.commits();
    // The front window closes: same card, same box, a new preview.
    ui.set_registry(registry(&[("files", Some(2), &[8])]));
    ui.frame();
    let after = sets(&ui.sink.take());
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].0, id(8));
    assert_eq!(after[0].1, before[0].1, "placed where the old one was");
    assert_eq!(
        ui.sink.commits(),
        [ui.win],
        "nothing else redraws, so ask for the commit"
    );
}

#[test]
fn hidden_mid_swipe_resets_the_card() {
    let mut ui = ui(
        "midswipe",
        registry(&[("files", Some(2), &[7]), ("clock", Some(1), &[9])]),
    );
    ui.emit(Shown);
    let files = ui.cards()[0];
    let at = ui.grip(files);
    ui.contact(ContactPhase::Moved, at);
    ui.contact(ContactPhase::Pressed, at);
    ui.contact(ContactPhase::Moved, Point::new(at.x, at.y - 80.0));
    ui.app.tick();
    assert_eq!(ui.card(files).offset(), -80.0);

    // Swiped home with the finger still down; that touch never ends here.
    ui.emit(Hidden);
    assert_eq!(ui.card(files).offset(), 0.0);
    assert_eq!(ui.card(files).opacity(), 1.0);

    // The next swipe, by a new touch point, works.
    ui.emit(Shown);
    ui.drag_by(1, files, Point::new(0.0, -200.0));
    assert_eq!(*ui.closed.borrow(), [id(7)]);
}

#[test]
fn window_keyed_card_follows_its_title() {
    let lone = |title: &str| {
        Registry::fixture(
            entries(),
            vec![RunningApp {
                key: AppKey::Window(id(5)),
                entry: None,
                windows: vec![id(5)],
                dialogs: Vec::new(),
                modal: false,
                pids: Vec::new(),
                front: id(5),
                last_used: 1,
                title: title.into(),
            }],
        )
    };
    let mut ui = ui("titled", lone("Scratch"));
    assert_eq!(ui.names(), ["Scratch"]);
    ui.set_registry(lone("Renamed"));
    assert_eq!(ui.names(), ["Renamed"]);
}

#[test]
fn no_thumbnails_means_no_preview_hole() {
    let sink = Rc::new(Sink::default());
    sink.unavailable.set(true);
    let mut ui = ui_with("noholes", three(), sink);
    ui.emit(Shown);
    ui.frame();
    for card in ui.cards() {
        let hole = ui.card(card).hole();
        let r = ui.app.component::<Layout>(hole).unwrap().rect;
        assert_eq!(r.height(), 0.0, "header-only card");
    }
    assert!(ui.sink.take().is_empty());
}
