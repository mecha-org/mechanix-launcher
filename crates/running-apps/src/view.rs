//! The screen: a row of cards kept in step with the registry,
//! and the previews that go with them while it is shown.

use std::rc::Rc;

use apps::{AppKey, IconCache, IconDirs, Registry, RegistryChanged, RowIcon, RunningApp};
use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;
use screen::{Hidden, Shown};

use crate::card::{
    self, Activate, Card, CardBuilder, CloseWindows, ForceClose, HEADER, SIDE, WindowsChanged,
    hole_style,
};
use crate::thumbs::{Place, ThumbKey, ThumbSink};
use crate::{Killer, WindowAction};

/// Glyph size of the ✕.
const CROSS: u32 = 20;
/// A card's share of the window's width, leaving the next one peeking in.
const CARD_SHARE: f32 = 0.75;

pub(crate) struct ViewBuilder {
    pub icon_dirs: Rc<IconDirs>,
    pub thumbs: Rc<dyn ThumbSink>,
}

impl Build for ViewBuilder {
    type Widget = View;
}

/// A card and its preview.
struct Slot {
    card: NodeId,
    /// The live preview and the window it shows.
    thumb: Option<(ThumbKey, ToplevelId)>,
    /// What the preview was last placed at.
    sent: Option<Place>,
}

pub(crate) struct View {
    icon_dirs: Rc<IconDirs>,
    thumbs: Rc<dyn ThumbSink>,
    cross: SpriteId,
    skull: SpriteId,
    list: Handle<Scrollable>,
    empty: NodeId,
    slots: Vec<Slot>,
    shown: bool,
    /// Each card's width, and its preview box's height for the window's
    /// aspect; 0 before the first layout.
    card_width: f32,
    hole_height: f32,
}

impl View {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn list(&self) -> NodeId {
        self.list.id()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn empty(&self) -> NodeId {
        self.empty
    }

    pub(crate) fn shown(&self) -> bool {
        self.shown
    }
}

impl Widget for View {
    type Builder = ViewBuilder;

    fn build(b: ViewBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().column().fill();
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        let empty = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .center()
                    .width(percent(100.0))
                    .grow(1.0),
            ),
        );
        s.spawn(
            empty,
            mw::text("No apps open")
                .size(20.0)
                .color(ColorRole::OnSurfaceVariant)
                .wrap(TextWrap::NoWrap),
        );
        let list = s.spawn(
            me,
            scrollable().axis(Axis::Horizontal).style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0))
                    .padding(Insets::all(px(SIDE)))
                    .hidden(),
            ),
        );
        let (cross, skull) = {
            let mut atlas = s.resource_mut::<Atlas>();
            (
                atlas
                    .insert_svg(assets::icons::CLEAR, CROSS)
                    .expect("embedded clear glyph is a valid SVG"),
                atlas
                    .insert_svg(assets::icons::SKULL, CROSS)
                    .expect("embedded skull glyph is a valid SVG"),
            )
        };
        View {
            icon_dirs: b.icon_dirs,
            thumbs: b.thumbs,
            cross,
            skull,
            list,
            empty: empty.id(),
            slots: Vec::new(),
            shown: false,
            card_width: 0.0,
            hole_height: 0.0,
        }
    }
}

/// What the screen does with a window or an app, swappable for tests.
struct Hooks {
    activator: WindowAction,
    closer: WindowAction,
    killer: Killer,
}
impl Resource for Hooks {}

/// Registers the screen's systems; once, however many screens build.
pub(crate) fn install(
    app: &mut App,
    activator: WindowAction,
    closer: WindowAction,
    killer: Killer,
) {
    let first = !app.has_resource::<Hooks>();
    app.insert_resource(Hooks {
        activator,
        closer,
        killer,
    });
    if !app.has_resource::<IconCache>() {
        let cache = IconCache::new(&mut app.resource_mut::<Atlas>());
        app.insert_resource(cache);
    }
    if first {
        app.system(on_shown)
            .system(on_hidden)
            .system(on_registry)
            .system(fit_holes)
            .system(|app: &mut App, e: &Activate| {
                let f = app.resource::<Hooks>().activator.clone();
                f(app, e.0);
            })
            .system(|app: &mut App, e: &CloseWindows| {
                let f = app.resource::<Hooks>().closer.clone();
                for &id in &e.0 {
                    f(app, id);
                }
            })
            .system(|app: &mut App, e: &ForceClose| {
                let f = app.resource::<Hooks>().killer.clone();
                f(app, &e.0);
            });
    }
}

/// Builds the first cards; the screen is then hidden until `Shown`.
pub(crate) fn spawned(app: &mut App, view: NodeId) {
    sync(app, view);
}

fn views(app: &mut App) -> Vec<NodeId> {
    app.widgets::<View>().map(|(id, _)| id).collect()
}

fn on_shown(app: &mut App, e: &Emitted<Shown>) {
    for &view in e.targets.iter() {
        let Some(v) = app.widget_mut::<View>(view) else {
            continue;
        };
        v.shown = true;
        sync(app, view);
    }
}

fn on_hidden(app: &mut App, e: &Emitted<Hidden>) {
    for &view in e.targets.iter() {
        let Some(v) = app.widget_mut::<View>(view) else {
            continue;
        };
        v.shown = false;
        let sink = v.thumbs.clone();
        let cards: Vec<NodeId> = v.slots.iter().map(|s| s.card).collect();
        let thumbs: Vec<ThumbKey> = v
            .slots
            .iter_mut()
            .filter_map(|s| {
                s.sent = None;
                s.thumb.take().map(|(k, _)| k)
            })
            .collect();
        for key in thumbs {
            sink.destroy(app, key);
        }
        for card in cards {
            card::reset_gesture(app, card);
        }
    }
}

fn on_registry(app: &mut App, _: &RegistryChanged) {
    for view in views(app) {
        sync(app, view);
    }
}

/// One running app as the registry has it, as a card shows it.
struct Want {
    key: AppKey,
    name: String,
    icon: RowIcon,
    front: ToplevelId,
    windows: Vec<ToplevelId>,
    dialogs: Vec<ToplevelId>,
    modal: bool,
    pids: Vec<u32>,
}

fn wanted(app: &mut App, view: NodeId) -> Vec<Want> {
    if !app.has_resource::<Registry>() {
        return Vec::new();
    }
    let dirs = app.widget::<View>(view).unwrap().icon_dirs.clone();
    // Each app with the name and icon file its card shows.
    let found: Vec<(RunningApp, String, Option<apps::IconFile>)> = {
        let registry = app.resource::<Registry>();
        registry
            .apps()
            .iter()
            .map(|a| {
                let entry = a.entry.and_then(|i| registry.entries().get(i));
                let name = entry.map_or_else(|| a.title.clone(), |e| e.name.clone());
                let file = entry
                    .and_then(|e| e.icon.as_deref())
                    .and_then(|i| dirs.find(i));
                (a.clone(), name, file)
            })
            .collect()
    };
    let (mut atlas, mut cache) = app.query::<(ResMut<Atlas>, ResMut<IconCache>)>();
    found
        .into_iter()
        .map(|(a, name, file)| Want {
            key: a.key,
            name,
            icon: cache.get(&mut atlas, file.as_ref()),
            front: a.front,
            windows: a.windows,
            dialogs: a.dialogs,
            modal: a.modal,
            pids: a.pids,
        })
        .collect()
}

/// Brings the cards in line with the registry and, while shown, the
/// previews in line with the cards.
fn sync(app: &mut App, view: NodeId) {
    let want = wanted(app, view);
    let sink = app.widget::<View>(view).unwrap().thumbs.clone();

    // Cards whose app is gone go, with their previews.
    let slots = std::mem::take(&mut app.widget_mut::<View>(view).unwrap().slots);
    let mut kept = Vec::new();
    for slot in slots {
        let key = app.widget::<Card>(slot.card).map(|c| c.key().clone());
        if key.is_some_and(|k| want.iter().any(|w| w.key == k)) {
            kept.push(slot);
        } else {
            drop_slot(app, &*sink, slot);
        }
    }

    // The rest stay as they are when nothing but their windows changed;
    // otherwise every card is rebuilt in the registry's order.
    let in_place = kept.len() == want.len()
        && kept.iter().zip(&want).all(|(s, w)| {
            app.widget::<Card>(s.card)
                .is_some_and(|c| *c.key() == w.key && c.name() == w.name && c.icon() == w.icon)
        });
    if in_place {
        for (slot, w) in kept.iter().zip(&want) {
            let card = app.widget_mut::<Card>(slot.card).unwrap();
            card.front = w.front;
            card.windows = w.windows.clone();
            card.dialogs = w.dialogs.clone();
            card.modal = w.modal;
            card::set_pids(app, slot.card, w.pids.clone());
        }
    } else {
        let closing: Vec<(AppKey, card::Closing)> = kept
            .iter()
            .filter_map(|s| app.widget::<Card>(s.card))
            .filter_map(|c| Some((c.key().clone(), c.closing_state()?)))
            .collect();
        for slot in std::mem::take(&mut kept) {
            drop_slot(app, &*sink, slot);
        }
        let (list, cross, skull, width, hole_height) = {
            let v = app.widget::<View>(view).unwrap();
            (v.list, v.cross, v.skull, v.card_width, v.hole_height)
        };
        for w in want {
            let closing = closing
                .iter()
                .find(|(k, _)| *k == w.key)
                .map(|(_, c)| c.clone());
            let card = app
                .spawn(
                    list,
                    CardBuilder {
                        key: w.key,
                        name: w.name,
                        icon: w.icon,
                        cross,
                        skull,
                        front: w.front,
                        windows: w.windows,
                        dialogs: w.dialogs,
                        modal: w.modal,
                        pids: w.pids,
                        view,
                        width,
                        hole_height,
                        closing,
                    },
                )
                .id();
            kept.push(Slot {
                card,
                thumb: None,
                sent: None,
            });
        }
    }

    for slot in &kept {
        app.emit(WindowsChanged, slot.card);
    }
    let (list, empty, none) = {
        let v = app.widget_mut::<View>(view).unwrap();
        v.slots = kept;
        (v.list.id(), v.empty, v.slots.is_empty())
    };
    let (show_list, show_empty) = if none {
        (Display::Hidden, Display::Flex)
    } else {
        (Display::Flex, Display::Hidden)
    };
    for (node, display) in [(list, show_list), (empty, show_empty)] {
        if app.component::<LayoutStyle>(node).unwrap().display != display {
            app.component_mut::<LayoutStyle>(node).unwrap().display = display;
        }
    }

    if app.widget::<View>(view).unwrap().shown {
        sync_thumbs(app, view);
    }
}

fn drop_slot(app: &mut App, sink: &dyn ThumbSink, slot: Slot) {
    if let Some((key, _)) = slot.thumb {
        sink.destroy(app, key);
    }
    app.remove(slot.card);
}

/// Every card gets a preview of its front window: a new one when it has
/// none or its front changed.
fn sync_thumbs(app: &mut App, view: NodeId) {
    let Some(window) = app.component::<InWindow>(view).and_then(|w| w.0) else {
        return;
    };
    let (sink, plan) = {
        let v = app.widget::<View>(view).unwrap();
        let plan: Vec<(usize, Option<ThumbKey>, ToplevelId)> = v
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let front = app.widget::<Card>(s.card)?.front;
                match s.thumb {
                    Some((_, shown)) if shown == front => None,
                    Some((key, _)) => Some((i, Some(key), front)),
                    None => Some((i, None, front)),
                }
            })
            .collect();
        (v.thumbs.clone(), plan)
    };
    if !plan.is_empty() {
        // A new preview shows once placed, which the next frame does.
        app.signal(RequestFrame(window));
    }
    for (i, old, front) in plan {
        if let Some(key) = old {
            sink.destroy(app, key);
        }
        let thumb = sink.create(app, window, front).map(|k| (k, front));
        let slot = &mut app.widget_mut::<View>(view).unwrap().slots[i];
        slot.thumb = thumb;
        slot.sent = None;
    }
}

/// The preview box keeps this screen's aspect: apps on the phone fill the
/// area below the status bar, and this screen does too. A card takes most
/// of the width, narrower when header and box would not fit the height.
/// With no previews to show (a compositor without thumbnail views) there is
/// no box: the cards are headers only.
fn fit_holes(app: &mut App, _: &LayoutDone) {
    for view in views(app) {
        let Some(size) = view_size(app, view) else {
            continue;
        };
        let aspect = size.height / size.width;
        let (width, height) = if app.widget::<View>(view).unwrap().thumbs.available(app) {
            let tall = (size.height - 2.0 * SIDE - HEADER).max(0.0);
            let width = (size.width * CARD_SHARE).min(tall / aspect).round();
            (width, (width * aspect).round())
        } else {
            ((size.width * CARD_SHARE).round(), 0.0)
        };
        let v = app.widget_mut::<View>(view).unwrap();
        if (v.card_width, v.hole_height) == (width, height) {
            continue;
        }
        v.card_width = width;
        v.hole_height = height;
        let cards: Vec<NodeId> = v.slots.iter().map(|s| s.card).collect();
        for card in cards {
            card::set_width(app, card, width);
            let Some(hole) = app.widget::<Card>(card).map(Card::hole) else {
                continue;
            };
            *app.component_mut::<LayoutStyle>(hole).unwrap() = hole_style(height);
        }
    }
}

/// The screen's own box: the area an app gets, which its window, laid
/// over the status bar too, is not.
fn view_size(app: &App, view: NodeId) -> Option<Size> {
    let rect = app.component::<ComputedLayout>(view)?.rect;
    (rect.width() > 0.0 && rect.height() > 0.0).then(|| Size::new(rect.width(), rect.height()))
}

/// Each frame of the screen's window, after layout and scroll and before
/// presentation commits: every preview whose box or fade changed since it
/// was last placed is placed again, and the surface is made to commit.
pub(crate) fn place_thumbnails(app: &mut App, f: &Frame) {
    for view in views(app) {
        let Some(v) = app.widget::<View>(view) else {
            continue;
        };
        if !v.shown || app.component::<InWindow>(view).and_then(|w| w.0) != Some(f.0) {
            continue;
        }
        let Some(size) = view_size(app, view) else {
            continue;
        };
        let sink = v.thumbs.clone();
        let mut sends = Vec::new();
        for (i, slot) in v.slots.iter().enumerate() {
            let (Some((key, _)), Some(card)) = (slot.thumb, app.widget::<Card>(slot.card)) else {
                continue;
            };
            let Some(place) = place_of(app, card, size.width) else {
                continue;
            };
            if slot.sent != Some(place) {
                sends.push((i, key, place));
            }
        }
        if sends.is_empty() {
            continue;
        }
        for (i, key, place) in sends {
            sink.set(app, key, place);
            app.widget_mut::<View>(view).unwrap().slots[i].sent = Some(place);
        }
        // A placement applies at the surface's next commit, which a frame
        // with nothing to redraw would not make.
        sink.commit(app, f.0);
    }
}

/// Where `card`'s preview goes: its box as drawn, less the scroll of every
/// viewport above it.
fn place_of(app: &App, card: &Card, window_width: f32) -> Option<Place> {
    let hole = card.hole();
    let rect = app.component::<Layout>(hole)?.rect;
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let mut shift = Point::ZERO;
    for a in app.ancestors(hole) {
        if let Some(vp) = app.component::<Viewport>(a) {
            shift.x += vp.scroll.x;
            shift.y += vp.scroll.y;
        }
    }
    Some(Place {
        x: (rect.x() - shift.x).round() as i32,
        y: (rect.y() - shift.y).round() as i32,
        scale: rect.width() / window_width,
        alpha: card.opacity(),
    })
}
