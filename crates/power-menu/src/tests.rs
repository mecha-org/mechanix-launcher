use super::*;
use mecha_wayland::mechanix_widgets::prelude::{FontBook, Slider, SliderSize};

#[derive(Default)]
struct Seen {
    shutdowns: usize,
    restarts: usize,
    logouts: usize,
    closed: usize,
}
impl Resource for Seen {}

fn app() -> App {
    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(WindowModule)
        .add_module(PaintModule)
        .add_module(InteractivityModule)
        .add_module(MechanixTheme::dark());
    let mut atlas = Atlas::new();
    let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
    app.insert_resource(atlas);
    app.insert_resource(FontBook::new(font));
    app.init_resource::<Seen>();
    app.system(|app: &mut App, _: &PowerOff| app.resource_mut::<Seen>().shutdowns += 1)
        .system(|app: &mut App, _: &Restart| app.resource_mut::<Seen>().restarts += 1)
        .system(|app: &mut App, _: &LogOut| app.resource_mut::<Seen>().logouts += 1)
        .system(|app: &mut App, _: &Emitted<PowerMenuClosed>| {
            app.resource_mut::<Seen>().closed += 1;
        });
    app
}

/// A menu in a 400 × 800 screen, laid out.
fn menu(app: &mut App) -> NodeId {
    let screen = app
        .spawn_with(
            app.root(),
            div().style(LayoutStyle::default().size(px(400.0), px(800.0))),
            (LayoutRoot(true),),
        )
        .id();
    app.spawn(screen, PowerMenuBuilder).id()
}

fn open(app: &mut App, menu: NodeId) {
    app.emit(OpenPowerMenu, menu);
    app.flush();
    app.tick();
}

fn row(app: &App, menu: NodeId, i: usize) -> NodeId {
    app.widget::<PowerMenu>(menu).unwrap().rows[i].node.id()
}

fn slider(app: &App, menu: NodeId, i: usize) -> &Slider {
    let id = app.widget::<PowerMenu>(menu).unwrap().rows[i].slider.id();
    app.widget::<Slider>(id).unwrap()
}

fn slider_shown(app: &App, menu: NodeId, i: usize) -> bool {
    let id = app.widget::<PowerMenu>(menu).unwrap().rows[i].slider.id();
    shown(app, id)
}

fn rect(app: &App, node: NodeId) -> Rect {
    app.component::<ComputedLayout>(node).unwrap().rect
}

fn shown(app: &App, node: NodeId) -> bool {
    app.component::<LayoutStyle>(node).unwrap().display == Display::Flex
}

fn slide(app: &App, menu: NodeId) -> Option<Slide> {
    app.widget::<PowerMenu>(menu).unwrap().slide
}

const CONTACT: ContactId = ContactId::Mouse;

fn press(app: &mut App, node: NodeId, x: f32) {
    let r = rect(app, node);
    let y = r.y() + r.height() / 2.0;
    app.emit(
        Press {
            contact: CONTACT,
            position: Point::new(x, y),
            time: 0,
        },
        node,
    );
    app.flush();
}

fn drag(app: &mut App, node: NodeId, x: f32) {
    let r = rect(app, node);
    let y = r.y() + r.height() / 2.0;
    app.emit(
        PressMoved {
            contact: CONTACT,
            position: Point::new(x, y),
            time: 0,
        },
        node,
    );
    app.flush();
}

fn release(app: &mut App, node: NodeId, x: f32) {
    let r = rect(app, node);
    let y = r.y() + r.height() / 2.0;
    app.emit(
        Release {
            contact: CONTACT,
            position: Point::new(x, y),
            time: 0,
        },
        node,
    );
    app.flush();
}

#[test]
fn progress_runs_from_the_press_to_the_right_edge() {
    assert_eq!(slide_progress(20.0, 20.0, 0.0, 400.0), 0.0);
    assert_eq!(slide_progress(20.0, 210.0, 0.0, 400.0), 0.5);
    assert_eq!(slide_progress(20.0, 400.0, 0.0, 400.0), 1.0);
    // Backwards stays at the start.
    assert_eq!(slide_progress(20.0, 0.0, 0.0, 400.0), 0.0);
    // Near the edge, it still takes half the row.
    assert_eq!(slide_progress(380.0, 480.0, 0.0, 400.0), 0.5);
    assert_eq!(slide_progress(0.0, 10.0, 0.0, 0.0), 0.0);
}

#[test]
fn the_xsmall_slider_shows_only_while_its_row_is_slid() {
    let mut app = app();
    let menu = menu(&mut app);
    open(&mut app, menu);
    for i in 0..3 {
        assert_eq!(slider(&app, menu, i).size(), SliderSize::XSmall);
        assert!(!slider_shown(&app, menu, i));
    }

    // Power off's slider is XSmall's full height; the smaller rows'
    // are three quarters of it.
    let height = |app: &App, i: usize| {
        let id = app.widget::<PowerMenu>(menu).unwrap().rows[i].slider.id();
        rect(app, id).height()
    };
    let hero = row(&app, menu, 0);
    press(&mut app, hero, 20.0);
    app.tick();
    assert_eq!(height(&app, 0), 32.0);
    release(&mut app, hero, 20.0);

    let node = row(&app, menu, 1);
    press(&mut app, node, 20.0);
    app.tick();
    assert_eq!(height(&app, 1), 24.0);
    assert!(slider_shown(&app, menu, 1));
    assert!(!slider_shown(&app, menu, 0));
    assert!(!slider_shown(&app, menu, 2));

    drag(&mut app, node, 210.0);
    app.tick();
    let s = slider(&app, menu, 1);
    assert_eq!(s.value(), 0.5);
    assert!(s.tick_count() > 0);
    assert!(s.is_thumb_shown());
    assert_eq!(s.active_color(), Some(ColorRole::Primary.into()));
    assert_eq!(s.thumb_color(), Some(ColorRole::OnSurface.into()));
    // The icon and label blend over the shade.
    let (icon, label) = {
        let r = &app.widget::<PowerMenu>(menu).unwrap().rows[1];
        (r.icon.id(), r.label.id())
    };
    let native = app.children(label).unwrap()[0];
    for id in [icon, native] {
        match app.component::<Paint>(id).unwrap() {
            Paint::Monochrome(sprites) => {
                assert!(sprites.iter().all(|s| !s.is_opaque), "{id:?}")
            }
            other => panic!("expected Monochrome, got {other:?}"),
        }
    }
    // The shade ends under the thumb.
    let fill = app.widget::<PowerMenu>(menu).unwrap().rows[1].fill.id();
    let row_left = rect(&app, node).x();
    let center = s.thumb_center().round();
    assert_eq!(rect(&app, fill).right() - row_left, center);

    release(&mut app, node, 210.0);
    app.tick();
    assert!(!slider_shown(&app, menu, 1));
    assert_eq!(slider(&app, menu, 1).value(), 0.0);
    assert!(!slider(&app, menu, 1).is_thumb_shown());
}

#[test]
fn sliding_to_the_end_confirms_each_action() {
    let mut app = app();
    let menu = menu(&mut app);
    let expect = [(0, [1, 0, 0]), (1, [1, 1, 0]), (2, [1, 1, 1])];
    for (i, counts) in expect {
        open(&mut app, menu);
        let node = row(&app, menu, i);
        press(&mut app, node, 20.0);
        assert!(slide(&app, menu).is_some());
        drag(&mut app, node, 200.0);
        assert!(shown(&app, menu));
        drag(&mut app, node, 399.0);
        let seen = app.resource::<Seen>();
        assert_eq!([seen.shutdowns, seen.restarts, seen.logouts], counts);
        assert!(!shown(&app, menu));
        assert_eq!(slide(&app, menu), None);
        assert!(!slider_shown(&app, menu, i));
        release(&mut app, node, 399.0);
    }
    assert_eq!(app.resource::<Seen>().closed, 3);
}

#[test]
fn letting_go_early_cancels_and_resets() {
    let mut app = app();
    let menu = menu(&mut app);
    open(&mut app, menu);
    let node = row(&app, menu, 0);
    let parts = |app: &App| {
        let r = &app.widget::<PowerMenu>(menu).unwrap().rows[0];
        (r.fill.id(), r.arrow.id())
    };
    let (fill, arrow) = parts(&app);

    press(&mut app, node, 20.0);
    drag(&mut app, node, 250.0);
    assert!(shown(&app, fill));
    assert!(!shown(&app, arrow));
    let p = slide(&app, menu).unwrap().progress;
    assert!(p > 0.5 && p < COMPLETE, "{p}");

    release(&mut app, node, 250.0);
    assert_eq!(slide(&app, menu), None);
    assert!(!shown(&app, fill));
    assert!(shown(&app, arrow));
    assert!(shown(&app, menu));
    let seen = app.resource::<Seen>();
    assert_eq!([seen.shutdowns, seen.restarts, seen.logouts], [0, 0, 0]);
}

#[test]
fn hidden_it_ignores_slides_and_cancel_closes() {
    let mut app = app();
    let menu = menu(&mut app);
    app.tick();
    let node = row(&app, menu, 0);
    app.emit(
        Press {
            contact: CONTACT,
            position: Point::ZERO,
            time: 0,
        },
        node,
    );
    app.flush();
    assert_eq!(slide(&app, menu), None);

    open(&mut app, menu);
    // The last row.
    let cancel = *app.children(menu).unwrap().last().unwrap();
    app.emit(
        Clicked {
            contact: CONTACT,
            position: Point::ZERO,
        },
        cancel,
    );
    app.flush();
    assert!(!shown(&app, menu));
    assert_eq!(app.resource::<Seen>().closed, 1);
}

#[test]
fn the_module_shows_and_hides_its_window() {
    let mut app = app();
    // PresentationModule registers Role; it needs a live compositor.
    app.register_component::<Role>();
    app.add_module(RingModule::default())
        .add_module(DBusModule::new().system())
        .add_module(PowerMenuModule);
    app.flush();
    let menu = app.resource::<Overlay>().menu.id();
    let window = app.resource::<Overlay>().window;
    assert_eq!(app.parent(menu), Some(window));
    assert!(!shown(&app, menu));

    app.signal(ShowPowerMenu);
    app.flush();
    assert!(shown(&app, menu));

    hide_power_menu(&mut app);
    app.flush();
    assert!(!shown(&app, menu));
    assert_eq!(app.resource::<Seen>().closed, 1);
}
