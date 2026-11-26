//! One running app: a header (icon, name, ✕) over the preview hole. Or
//! one app starting: its icon where the preview goes, until its window
//! comes.

use std::time::Duration;

use apps::{AppKey, LaunchId, RowIcon, ShowLaunch, StopLaunch};
use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;
use screen::AppLaunched;

use crate::kill::KILL_WAIT;
use crate::view::View;

/// Between the screen's edges and a card.
pub(crate) const SIDE: f32 = 24.0;
/// Between neighbouring cards.
pub(crate) const GAP: f32 = 24.0;
pub(crate) const HEADER: f32 = 56.0;
const ICON: f32 = 32.0;
/// A starting app's icon, in its preview's place.
const LAUNCH_ICON: f32 = 96.0;
const NAME_PX: f32 = 20.0;
/// The ✕'s touch target.
const CLOSE: f32 = 48.0;
/// An upward move past this many pixels makes the press a swipe.
const SLOP: f32 = 12.0;
/// A swipe released this far up closes the app.
const SWIPE_AWAY: f32 = 120.0;
/// A swipe this far out has faded as far as it fades.
const FADE_OVER: f32 = 240.0;
/// The lowest opacity a swipe fades to.
const FADE_TO: f32 = 0.2;
/// A card whose app was asked to close.
const DIMMED: f32 = 0.4;
/// How long an app asked to close has to go before it is brought back.
pub(crate) const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// Asks the screen's hooks to activate a window.
pub(crate) struct Activate(pub ToplevelId);
impl Signal for Activate {}

/// Asks the screen's hooks to close every one of these windows.
pub(crate) struct CloseWindows(pub Vec<ToplevelId>);
impl Signal for CloseWindows {}

/// Asks the screen's hooks to kill these processes.
pub(crate) struct ForceClose(pub Vec<u32>);
impl Signal for ForceClose {}

/// What a card is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardKey {
    /// A running app.
    App(AppKey),
    /// An app starting.
    Launch(LaunchId),
}

/// The app's windows changed: a closing card looks for a new dialog.
pub(crate) struct WindowsChanged;
impl Event for WindowsChanged {}

/// How long a force closed app has to go before it is brought back: it
/// may take all of `KILL_WAIT` to end.
const FORCE_WAIT: Duration = Duration::from_secs(KILL_WAIT.as_secs() + CLOSE_WAIT.as_secs());

/// An app on its way out.
#[derive(Clone)]
pub(crate) enum Closing {
    /// ✕: its windows were asked to close, with these dialogs open. One
    /// opened since is the app asking something.
    Asked { dialogs: Vec<ToplevelId> },
    /// The skull: its processes were signalled.
    Forced,
}

impl Closing {
    /// How long the app has to go.
    fn wait(&self) -> Duration {
        match self {
            Closing::Asked { .. } => CLOSE_WAIT,
            Closing::Forced => FORCE_WAIT,
        }
    }
}

pub(crate) struct CardBuilder {
    pub key: CardKey,
    pub name: String,
    pub icon: RowIcon,
    pub cross: SpriteId,
    pub skull: SpriteId,
    /// The window to bring forward; `None` for an app starting.
    pub front: Option<ToplevelId>,
    pub windows: Vec<ToplevelId>,
    pub dialogs: Vec<ToplevelId>,
    /// One of the dialogs is modal.
    pub modal: bool,
    /// The app's processes: without any, there is nothing to force close.
    pub pids: Vec<u32>,
    /// The screen the card is on.
    pub view: NodeId,
    pub width: f32,
    pub hole_height: f32,
    pub closing: Option<Closing>,
}

impl Build for CardBuilder {
    type Widget = Card;
}

struct Drag {
    contact: ContactId,
    origin: Point,
    /// Past the slop upward: the card holds the contact.
    swiping: bool,
}

pub(crate) struct Card {
    key: CardKey,
    name: String,
    icon: RowIcon,
    pub(crate) front: Option<ToplevelId>,
    pub(crate) windows: Vec<ToplevelId>,
    pub(crate) dialogs: Vec<ToplevelId>,
    pub(crate) modal: bool,
    pub(crate) pids: Vec<u32>,
    view: NodeId,
    closing: Option<Closing>,
    /// Runs while closing, for [`Closing::wait`].
    wait: TimerId,
    drag: Option<Drag>,
    /// How far a swipe has moved the card up (negative).
    dy: f32,
    width: f32,
    /// The release that ended a swipe is followed by a `Clicked`.
    swallow_click: bool,
    header: NodeId,
    kill: NodeId,
    close: NodeId,
    hole: NodeId,
    veil: Handle<Div>,
}

impl Card {
    pub(crate) fn key(&self) -> &CardKey {
        &self.key
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn icon(&self) -> RowIcon {
        self.icon
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn front(&self) -> Option<ToplevelId> {
        self.front
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn closing(&self) -> bool {
        self.closing.is_some()
    }

    #[cfg(test)]
    pub(crate) fn wait(&self) -> TimerId {
        self.wait
    }

    /// For a card rebuilt in its place.
    pub(crate) fn closing_state(&self) -> Option<Closing> {
        self.closing.clone()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn offset(&self) -> f32 {
        self.dy
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn header(&self) -> NodeId {
        self.header
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn close_button(&self) -> NodeId {
        self.close
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn kill_button(&self) -> NodeId {
        self.kill
    }

    /// The preview's box.
    pub(crate) fn hole(&self) -> NodeId {
        self.hole
    }

    /// How opaque the card draws, its preview included.
    pub(crate) fn opacity(&self) -> f32 {
        if self.closing.is_some() {
            return DIMMED;
        }
        let t = (self.dy.abs() / FADE_OVER).min(1.0);
        1.0 - t * (1.0 - FADE_TO)
    }
}

fn card_style(dy: f32, width: f32) -> LayoutStyle {
    LayoutStyle::default()
        .column()
        .relative()
        .inset(Insets::new(px(dy), auto(), auto(), auto()))
        .width(px(width))
        .shrink(0.0)
        .margin(Insets::new(px(0.0), px(GAP), px(0.0), px(0.0)))
}

pub(crate) fn hole_style(height: f32) -> LayoutStyle {
    LayoutStyle::default()
        .center()
        .width(percent(100.0))
        .height(px(height))
        .shrink(0.0)
}

impl Widget for Card {
    type Builder = CardBuilder;

    fn build(b: CardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = card_style(0.0, b.width);

        let header = s.spawn(
            me,
            div().background(s.color(ColorRole::SurfaceBright)).style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .gap(px(12.0))
                    .size(percent(100.0), px(HEADER))
                    .padding(Insets::new(px(0.0), px(4.0), px(0.0), px(12.0))),
            ),
        );
        b.icon.spawn(s, header, ICON);
        let label = s.spawn(
            header,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .grow(1.0)
                    .min_width(px(0.0)),
            ),
        );
        s.spawn(
            label,
            mw::text(b.name.clone())
                .size(NAME_PX)
                .color(ColorRole::OnSurface)
                .wrap(TextWrap::NoWrap),
        );
        let button = || {
            LayoutStyle::default()
                .size(px(CLOSE), px(CLOSE))
                .shrink(0.0)
                .center()
        };
        let cross_color = s.color(ColorRole::OnSurface);
        // Only with a process to kill.
        let kill_style = if b.pids.is_empty() {
            button().hidden()
        } else {
            button()
        };
        // The skull and ✕ side by side, without the header's gap between
        // them: their touch targets already keep the glyphs apart.
        let actions = s.spawn(
            header,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .shrink(0.0),
            ),
        );
        let kill = s.spawn(actions, div().style(kill_style));
        s.spawn(kill, icon(b.skull).color(s.color(ColorRole::Error)));
        let close = s.spawn(actions, div().style(button()));
        s.spawn(close, icon(b.cross).color(cross_color));

        // Paints nothing: the compositor draws the preview here. An app
        // starting has none yet: its icon shows instead.
        let hole = match b.key {
            CardKey::App(_) => s.spawn(me, div().style(hole_style(b.hole_height))),
            CardKey::Launch(_) => {
                let surface = s.color(ColorRole::Surface);
                let hole = s.spawn(
                    me,
                    div().background(surface).style(hole_style(b.hole_height)),
                );
                b.icon.spawn(s, hole, LAUNCH_ICON);
                hole
            }
        };

        // Over everything: fades the card toward the screen's black.
        let veil = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::all(px(0.0))),
            ),
        );

        // Rebuilt while closing, it waits again from now.
        let wait = match &b.closing {
            Some(closing) => s.set_timeout(closing.wait(), |ctx, _| still_open(ctx, None)),
            None => s.add_timeout(CLOSE_WAIT, |ctx, _| still_open(ctx, None)),
        };

        s.on::<Clicked>(close.id(), |ctx, _| close_app(ctx))
            .on::<Clicked>(kill.id(), |ctx, _| force_close(ctx))
            .on::<WindowsChanged>(me, |ctx, _| {
                let me = ctx.me();
                let asked = match &me.closing {
                    Some(Closing::Asked { dialogs }) => {
                        me.dialogs.iter().find(|d| !dialogs.contains(d)).copied()
                    }
                    _ => None,
                };
                if asked.is_some() {
                    still_open(ctx, asked);
                }
            })
            .on::<Press>(me, on_press)
            .on::<PressMoved>(me, on_moved)
            .on::<PressCancelled>(me, |ctx, _| {
                ctx.me().drag = None;
                set_offset(ctx, 0.0);
            })
            .on::<Release>(me, on_release)
            .on::<Clicked>(me, |ctx, _| tap(ctx));

        let card = Card {
            key: b.key,
            name: b.name,
            icon: b.icon,
            front: b.front,
            windows: b.windows,
            dialogs: b.dialogs,
            modal: b.modal,
            pids: b.pids,
            view: b.view,
            closing: b.closing,
            wait,
            drag: None,
            dy: 0.0,
            width: b.width,
            swallow_click: false,
            header: header.id(),
            kill: kill.id(),
            close: close.id(),
            hole: hole.id(),
            veil,
        };
        *s.component_mut::<Paint>(veil).unwrap() = veil_paint(card.opacity());
        card
    }
}

fn veil_paint(opacity: f32) -> Paint {
    if opacity >= 1.0 {
        Paint::None
    } else {
        Paint::Quad(Quad::new(Color::BLACK.with_alpha(1.0 - opacity)))
    }
}

/// Moves the card up and repaints its veil to match.
fn set_offset(ctx: &mut Context<'_, Card>, dy: f32) {
    ctx.me().dy = dy;
    restyle(ctx);
}

fn restyle(ctx: &mut Context<'_, Card>) {
    let (dy, width, veil, paint) = {
        let c = ctx.me();
        (c.dy, c.width, c.veil, veil_paint(c.opacity()))
    };
    let style = card_style(dy, width);
    if *ctx.component::<LayoutStyle>().unwrap() != style {
        *ctx.component_mut::<LayoutStyle>().unwrap() = style;
    }
    if let Some(mut v) = ctx.at(veil)
        && *v.component::<Paint>().unwrap() != paint
    {
        *v.component_mut::<Paint>().unwrap() = paint;
    }
}

/// Sets the card's width, for the window it shows in.
pub(crate) fn set_width(app: &mut App, card: NodeId, width: f32) {
    let Some(c) = app.widget_mut::<Card>(card) else {
        return;
    };
    c.width = width;
    let style = card_style(c.dy, width);
    if app
        .component::<LayoutStyle>(card)
        .is_some_and(|s| *s != style)
    {
        *app.component_mut::<LayoutStyle>(card).unwrap() = style;
    }
}

/// Sets the app's processes, showing the skull only while there are any.
pub(crate) fn set_pids(app: &mut App, card: NodeId, pids: Vec<u32>) {
    let Some(c) = app.widget_mut::<Card>(card) else {
        return;
    };
    let display = if pids.is_empty() {
        Display::Hidden
    } else {
        Display::Flex
    };
    c.pids = pids;
    let kill = c.kill;
    if app.component::<LayoutStyle>(kill).unwrap().display != display {
        app.component_mut::<LayoutStyle>(kill).unwrap().display = display;
    }
}

/// Drops any press or swipe in progress and puts the card back in place.
/// For a screen hidden mid-gesture: that contact's release may never
/// reach the card, and a held drag would ignore every later press.
pub(crate) fn reset_gesture(app: &mut App, card: NodeId) {
    let Some(c) = app.widget_mut::<Card>(card) else {
        return;
    };
    c.drag = None;
    c.dy = 0.0;
    c.swallow_click = false;
    let (veil, paint) = (c.veil.id(), veil_paint(c.opacity()));
    let style = card_style(0.0, c.width);
    if app
        .component::<LayoutStyle>(card)
        .is_some_and(|s| *s != style)
    {
        *app.component_mut::<LayoutStyle>(card).unwrap() = style;
    }
    if app.component::<Paint>(veil).is_some_and(|p| *p != paint) {
        *app.component_mut::<Paint>(veil).unwrap() = paint;
    }
}

/// ✕ and swipe-away: every window is asked to close; the card dims and
/// ignores input until the registry drops it, or the app stays open. An
/// app with a modal dialog open is asking something already: it comes to
/// the front on the dialog instead, and nothing is closed. An app starting
/// is stopped; its card goes with its launch.
fn close_app(ctx: &mut Context<'_, Card>) {
    if ctx.me().closing.is_some() {
        return;
    }
    if let CardKey::Launch(launch) = ctx.me().key {
        ctx.signal(StopLaunch(launch));
        return;
    }
    if ctx.me().modal {
        ctx.me().dy = 0.0;
        restyle(ctx);
        if let Some(front) = ctx.me().front {
            bring_forward(ctx, front);
        }
        return;
    }
    let dialogs = ctx.me().dialogs.clone();
    let windows = ctx.me().windows.clone();
    start_closing(ctx, Closing::Asked { dialogs }, CloseWindows(windows));
}

/// The skull: the app's processes are told to end, and killed if they
/// don't (see `crate::kill`); the card dims as for ✕ until its windows go,
/// waiting [`FORCE_WAIT`]. A dialog opened meanwhile isn't brought
/// forward: the kill comes anyway.
///
/// FIXME: ask first. That wants a shell prompt: one generic structure for
/// every question the shell puts to the user, as for NetworkManager's
/// secret agent, polkit and the portals' dialogs.
fn force_close(ctx: &mut Context<'_, Card>) {
    if ctx.me().closing.is_some() || ctx.me().pids.is_empty() {
        return;
    }
    let pids = ctx.me().pids.clone();
    start_closing(ctx, Closing::Forced, ForceClose(pids));
}

/// The card dims and waits out `closing`; `close` goes out.
fn start_closing(ctx: &mut Context<'_, Card>, closing: Closing, close: impl Signal) {
    let wait = ctx.me().wait;
    let after = closing.wait();
    ctx.me().closing = Some(closing);
    ctx.start_timer_for(wait, after);
    ctx.signal(close);
    restyle(ctx);
}

/// `window` to the front, and the screen out of its way.
fn bring_forward(ctx: &mut Context<'_, Card>, window: ToplevelId) {
    ctx.signal(Activate(window));
    ctx.signal(AppLaunched);
}

/// The app asked to close is still open: it opened `dialog`, asking
/// something (save first?), or it didn't go within its wait. The
/// card takes input again and, while the screen is up, the app comes to
/// the front on the dialog, or on its own modal one, to be answered.
fn still_open(ctx: &mut Context<'_, Card>, dialog: Option<ToplevelId>) {
    if ctx.me().closing.take().is_none() {
        return;
    }
    let (wait, view, front) = {
        let c = ctx.me();
        (c.wait, c.view, c.front)
    };
    ctx.cancel_timer(wait);
    restyle(ctx);
    if ctx.widget::<View>(view).is_some_and(View::shown)
        && let Some(window) = dialog.or(front)
    {
        bring_forward(ctx, window);
    }
}

/// To the app: its front window, or for an app starting, its placeholder.
fn tap(ctx: &mut Context<'_, Card>) {
    if std::mem::take(&mut ctx.me().swallow_click) || ctx.me().closing.is_some() {
        return;
    }
    if let CardKey::Launch(launch) = ctx.me().key {
        ctx.signal(ShowLaunch(launch));
        ctx.signal(AppLaunched);
        return;
    }
    if let Some(front) = ctx.me().front {
        bring_forward(ctx, front);
    }
}

fn on_press(ctx: &mut Context<'_, Card>, e: &Press) {
    let c = ctx.me();
    if c.closing.is_some() || c.drag.is_some() {
        return;
    }
    c.swallow_click = false;
    c.drag = Some(Drag {
        contact: e.contact,
        origin: e.position,
        swiping: false,
    });
}

fn on_moved(ctx: &mut Context<'_, Card>, e: &PressMoved) {
    let Some(drag) = ctx.me().drag.as_ref() else {
        return;
    };
    if drag.contact != e.contact {
        return;
    }
    let (dx, dy) = (e.position.x - drag.origin.x, e.position.y - drag.origin.y);
    if !drag.swiping {
        // Only up: sideways belongs to the list's scroll.
        if -dy <= SLOP || -dy <= dx.abs() {
            return;
        }
        if !ctx.claim_contact(e.contact) {
            return;
        }
        if let Some(d) = ctx.me().drag.as_mut() {
            d.swiping = true;
        }
    }
    set_offset(ctx, dy.min(0.0));
}

fn on_release(ctx: &mut Context<'_, Card>, e: &Release) {
    let Some(drag) = ctx.me().drag.take_if(|d| d.contact == e.contact) else {
        return;
    };
    if !drag.swiping {
        return;
    }
    ctx.me().swallow_click = true;
    let away = drag.origin.y - e.position.y >= SWIPE_AWAY;
    ctx.me().dy = 0.0;
    if away {
        close_app(ctx);
    } else {
        restyle(ctx);
    }
}
