//! One running app: a header (icon, name, ✕) over the preview hole.

use apps::{AppKey, RowIcon};
use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;
use screen::AppLaunched;

/// Between the screen's edges and a card.
pub(crate) const SIDE: f32 = 24.0;
/// Under each card.
pub(crate) const GAP: f32 = 24.0;
const HEADER: f32 = 56.0;
const ICON: f32 = 32.0;
const NAME_PX: f32 = 20.0;
/// The ✕'s touch target.
const CLOSE: f32 = 48.0;
/// A sideways move past this many pixels makes the press a swipe.
const SLOP: f32 = 12.0;
/// A swipe released this far out closes the app.
const SWIPE_AWAY: f32 = 120.0;
/// A swipe this far out has faded as far as it fades.
const FADE_OVER: f32 = 240.0;
/// The lowest opacity a swipe fades to.
const FADE_TO: f32 = 0.2;
/// A card whose app was asked to close.
const DIMMED: f32 = 0.4;

/// Asks the screen's hooks to activate a window.
pub(crate) struct Activate(pub ToplevelId);
impl Signal for Activate {}

/// Asks the screen's hooks to close every one of these windows.
pub(crate) struct CloseWindows(pub Vec<ToplevelId>);
impl Signal for CloseWindows {}

pub(crate) struct CardBuilder {
    pub key: AppKey,
    pub name: String,
    pub icon: RowIcon,
    pub cross: SpriteId,
    pub front: ToplevelId,
    pub windows: Vec<ToplevelId>,
    pub hole_height: f32,
    pub closing: bool,
}

impl Build for CardBuilder {
    type Widget = Card;
}

struct Drag {
    contact: ContactId,
    origin: Point,
    /// Past the slop sideways: the card holds the contact.
    swiping: bool,
}

pub(crate) struct Card {
    key: AppKey,
    name: String,
    icon: RowIcon,
    pub(crate) front: ToplevelId,
    pub(crate) windows: Vec<ToplevelId>,
    closing: bool,
    drag: Option<Drag>,
    /// How far a swipe has moved the card sideways.
    dx: f32,
    /// The release that ended a swipe is followed by a `Clicked`.
    swallow_click: bool,
    header: NodeId,
    close: NodeId,
    hole: NodeId,
    veil: Handle<Div>,
}

impl Card {
    pub(crate) fn key(&self) -> &AppKey {
        &self.key
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn icon(&self) -> RowIcon {
        self.icon
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn front(&self) -> ToplevelId {
        self.front
    }

    pub(crate) fn closing(&self) -> bool {
        self.closing
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn offset(&self) -> f32 {
        self.dx
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn header(&self) -> NodeId {
        self.header
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn close_button(&self) -> NodeId {
        self.close
    }

    /// The preview's box.
    pub(crate) fn hole(&self) -> NodeId {
        self.hole
    }

    /// How opaque the card draws, its preview included.
    pub(crate) fn opacity(&self) -> f32 {
        if self.closing {
            return DIMMED;
        }
        let t = (self.dx.abs() / FADE_OVER).min(1.0);
        1.0 - t * (1.0 - FADE_TO)
    }
}

fn card_style(dx: f32) -> LayoutStyle {
    LayoutStyle::default()
        .column()
        .relative()
        .inset(Insets::new(auto(), auto(), auto(), px(dx)))
        .width(percent(100.0))
        .margin(Insets::new(px(0.0), px(0.0), px(GAP), px(0.0)))
}

pub(crate) fn hole_style(height: f32) -> LayoutStyle {
    LayoutStyle::default()
        .width(percent(100.0))
        .height(px(height))
        .shrink(0.0)
}

impl Widget for Card {
    type Builder = CardBuilder;

    fn build(b: CardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = card_style(0.0);

        let header = s.spawn(
            me,
            div()
                .background(s.color(ColorRole::SurfaceBright))
                .style(
                    LayoutStyle::default()
                        .row()
                        .align_items(Align::Center)
                        .gap(px(12.0))
                        .size(percent(100.0), px(HEADER))
                        .padding(Insets::new(px(0.0), px(4.0), px(0.0), px(12.0))),
                ),
        );
        let size = LayoutStyle::default().size(px(ICON), px(ICON)).shrink(0.0);
        match b.icon {
            RowIcon::Image(sprite) => {
                s.spawn(header, image(sprite).style(size));
            }
            RowIcon::Glyph(sprite) => {
                let color = s.color(ColorRole::OnSurfaceVariant);
                s.spawn(
                    header,
                    icon(sprite)
                        .size(Size::new(ICON, ICON))
                        .color(color)
                        .style(size),
                );
            }
        }
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
        let close = s.spawn(
            header,
            div().style(LayoutStyle::default().size(px(CLOSE), px(CLOSE)).shrink(0.0).center()),
        );
        let cross_color = s.color(ColorRole::OnSurface);
        s.spawn(close, icon(b.cross).color(cross_color));

        // Paints nothing: the compositor draws the preview here.
        let hole = s.spawn(me, div().style(hole_style(b.hole_height)));

        // Over everything: fades the card toward the screen's black.
        let veil = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::all(px(0.0))),
            ),
        );

        s.on::<Clicked>(close.id(), |ctx, _| close_app(ctx))
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
            closing: b.closing,
            drag: None,
            dx: 0.0,
            swallow_click: false,
            header: header.id(),
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

/// Moves the card sideways and repaints its veil to match.
fn set_offset(ctx: &mut Context<'_, Card>, dx: f32) {
    ctx.me().dx = dx;
    restyle(ctx);
}

fn restyle(ctx: &mut Context<'_, Card>) {
    let (dx, veil, paint) = {
        let c = ctx.me();
        (c.dx, c.veil, veil_paint(c.opacity()))
    };
    let style = card_style(dx);
    if *ctx.component::<LayoutStyle>().unwrap() != style {
        *ctx.component_mut::<LayoutStyle>().unwrap() = style;
    }
    if let Some(mut v) = ctx.at(veil)
        && *v.component::<Paint>().unwrap() != paint
    {
        *v.component_mut::<Paint>().unwrap() = paint;
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
    c.dx = 0.0;
    c.swallow_click = false;
    let (veil, paint) = (c.veil.id(), veil_paint(c.opacity()));
    let style = card_style(0.0);
    if app.component::<LayoutStyle>(card).is_some_and(|s| *s != style) {
        *app.component_mut::<LayoutStyle>(card).unwrap() = style;
    }
    if app.component::<Paint>(veil).is_some_and(|p| *p != paint) {
        *app.component_mut::<Paint>(veil).unwrap() = paint;
    }
}

/// ✕ and swipe-away: every window is asked to close; the card dims and
/// ignores input until the registry drops it.
fn close_app(ctx: &mut Context<'_, Card>) {
    if ctx.me().closing {
        return;
    }
    ctx.me().closing = true;
    let windows = ctx.me().windows.clone();
    ctx.signal(CloseWindows(windows));
    restyle(ctx);
}

fn tap(ctx: &mut Context<'_, Card>) {
    if std::mem::take(&mut ctx.me().swallow_click) || ctx.me().closing {
        return;
    }
    let front = ctx.me().front;
    ctx.signal(Activate(front));
    ctx.signal(AppLaunched);
}

fn on_press(ctx: &mut Context<'_, Card>, e: &Press) {
    let c = ctx.me();
    if c.closing || c.drag.is_some() {
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
        if dx.abs() <= SLOP || dx.abs() <= dy.abs() {
            return;
        }
        if !ctx.claim_contact(e.contact) {
            return;
        }
        if let Some(d) = ctx.me().drag.as_mut() {
            d.swiping = true;
        }
    }
    set_offset(ctx, dx);
}

fn on_release(ctx: &mut Context<'_, Card>, e: &Release) {
    let Some(drag) = ctx.me().drag.take_if(|d| d.contact == e.contact) else {
        return;
    };
    if !drag.swiping {
        return;
    }
    ctx.me().swallow_click = true;
    let away = (e.position.x - drag.origin.x).abs() >= SWIPE_AWAY;
    ctx.me().dx = 0.0;
    if away {
        close_app(ctx);
    } else {
        restyle(ctx);
    }
}
