//! The power menu: power off, restart or log out, each confirmed by
//! sliding its row to the end before it goes to logind.
//!
//! [`PowerMenuModule`] gives it a window of its own, a full-screen overlay
//! layer above everything else, shown by [`ShowPowerMenu`] and hidden by
//! [`hide_power_menu`]. The menu is also a plain widget,
//! [`PowerMenuBuilder`], for a host that shows it in a window of its own:
//! the lock screen, which nothing else draws over while locked. Such a
//! host opens it with [`OpenPowerMenu`] and closes it with
//! [`ClosePowerMenu`].

mod row;
#[cfg(test)]
mod tests;

use mecha_wayland::mechanix_widgets::prelude::{SliderContextExt, Text, TextContextExt, text};
use mecha_wayland::prelude::*;
use row::{
    Action, ICON_GAP, ICON_SIZE, Row, Slide, divider, padded_row, slide_progress, spawn_row,
};
use services::{LogOut, PowerModule, PowerOff, Restart};

/// Show the power menu's own window.
pub struct ShowPowerMenu;
impl Signal for ShowPowerMenu {}

/// Hide the power menu's own window, as if closed.
pub fn hide_power_menu(app: &mut App) {
    if let Some(overlay) = app
        .has_resource::<Overlay>()
        .then(|| app.resource::<Overlay>().menu)
    {
        app.emit(ClosePowerMenu, overlay);
    }
}

/// The [`PowerMenu`] it is emitted at closed: by its cancel row, or
/// after asking logind.
pub struct PowerMenuClosed;
impl Event for PowerMenuClosed {}

/// Open the [`PowerMenu`] it is emitted at.
pub struct OpenPowerMenu;
impl Event for OpenPowerMenu {}

/// Close the [`PowerMenu`] it is emitted at.
pub struct ClosePowerMenu;
impl Event for ClosePowerMenu {}

const NAMESPACE: &str = "power-menu";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

/// Installs the power menu's window, hidden, and logind's power-off,
/// reboot and log out. Expects the host to have installed
/// `DBusModule::new().system()`, `RingModule` and `WindowModule`, and to
/// have inserted the [`Atlas`] and `FontBook`.
pub struct PowerMenuModule;

/// The power menu's own window and the menu in it.
struct Overlay {
    window: NodeId,
    menu: Handle<PowerMenu>,
}
impl Resource for Overlay {}

impl Module for PowerMenuModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<Overlay>() {
            return;
        }
        app.add_module(PowerModule);
        let root = app.root();
        let role = Role::Layer(LayerRole {
            layer: Layer::Overlay,
            anchor: ALL_SIDES,
            // Over the bars' zones too.
            exclusive_zone: -1,
            namespace: NAMESPACE.into(),
            keyboard_interactivity: KeyboardInteractivity::None,
            shell_role: Some(ShellRole::Prompt),
        });
        let window = app
            .spawn_with(
                root,
                window()
                    .title(NAMESPACE)
                    .layout(LayoutStyle::default())
                    .clear(Color::TRANSPARENT),
                (role,),
            )
            .id();
        let menu = app.spawn(window, PowerMenuBuilder);
        app.insert_resource(Overlay { window, menu });
        set_mapped(app, false);
        app.system(|app: &mut App, _: &ShowPowerMenu| {
            set_mapped(app, true);
            let menu = app.resource::<Overlay>().menu;
            app.emit(OpenPowerMenu, menu);
        })
        .system(|app: &mut App, e: &Emitted<PowerMenuClosed>| {
            if e.targets.contains(&app.resource::<Overlay>().menu.id()) {
                set_mapped(app, false);
            }
        });
    }
}

/// Map or unmap the menu's own window.
fn set_mapped(app: &mut App, mapped: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let window = app.resource::<Overlay>().window;
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_mapped(window, mapped, &mut wl);
}

// ── layout, in percent of the menu ───────────────────────────────────────

const HEADER_HEIGHT: f32 = 12.0;
const CANCEL_HEIGHT: f32 = 12.0;
/// How far along the slide confirms.
const COMPLETE: f32 = 0.95;

/// Power off, restart or log out, each run by sliding its row to the
/// right; letting go first cancels. Fills its parent, hidden until
/// [`OpenPowerMenu`].
pub struct PowerMenu {
    /// "Slide to confirm", or "Release to cancel" while sliding.
    hint: Handle<Text>,
    rows: Vec<Row>,
    slide: Option<Slide>,
}

pub struct PowerMenuBuilder;

impl Build for PowerMenuBuilder {
    type Widget = PowerMenu;
}

/// The menu shows.
fn is_open(ctx: &Context<'_, PowerMenu>) -> bool {
    ctx.style().display != Display::Hidden
}

fn set_open(ctx: &mut Context<'_, PowerMenu>, open: bool) {
    let was = is_open(ctx);
    if ctx.me().slide.take().is_some() {
        render(ctx);
    }
    ctx.set_display(if open { Display::Flex } else { Display::Hidden });
    if was && !open {
        let me = ctx.handle();
        ctx.emit(PowerMenuClosed, me);
    }
}

fn start_slide(ctx: &mut Context<'_, PowerMenu>, row: usize, e: &Press) {
    if !is_open(ctx) || ctx.me().slide.is_some() {
        return;
    }
    // The row keeps the contact even if the finger strays off it.
    ctx.claim_contact(e.contact);
    ctx.me().slide = Some(Slide {
        row,
        contact: e.contact,
        start_x: e.position.x,
        progress: 0.0,
    });
    render(ctx);
}

fn move_slide(ctx: &mut Context<'_, PowerMenu>, e: &PressMoved) {
    let Some(slide) = ctx.me().slide.filter(|s| s.contact == e.contact) else {
        return;
    };
    let node = ctx.me().rows[slide.row].node;
    let Some(rect) = ctx
        .at(node)
        .and_then(|c| c.component::<ComputedLayout>().map(|l| l.rect))
    else {
        return;
    };
    let progress = slide_progress(slide.start_x, e.position.x, rect.x(), rect.width());
    if progress >= COMPLETE {
        let action = ctx.me().rows[slide.row].action;
        set_open(ctx, false);
        match action {
            Action::PowerOff => ctx.signal(PowerOff),
            Action::Restart => ctx.signal(Restart),
            Action::LogOut => ctx.signal(LogOut),
        }
        return;
    }
    if progress != slide.progress {
        ctx.me().slide = Some(Slide { progress, ..slide });
        render(ctx);
    }
}

/// Letting go before the end cancels.
fn end_slide(ctx: &mut Context<'_, PowerMenu>, contact: ContactId) {
    if ctx.me().slide.is_some_and(|s| s.contact == contact) {
        ctx.me().slide = None;
        render(ctx);
    }
}

/// Writes `node`'s style through `f`, unless that leaves it unchanged, so
/// moves that change nothing on screen skip the relayout.
fn restyle(
    ctx: &mut Context<'_, PowerMenu>,
    node: Handle<Div>,
    f: impl FnOnce(LayoutStyle) -> LayoutStyle,
) {
    if let Some(mut c) = ctx.at(node)
        && let Some(mut style) = c.component_mut::<LayoutStyle>()
    {
        let new = f(style.clone());
        if *style != new {
            *style = new;
        }
    }
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

/// Brings the header hint and every row in line with the slide.
fn render(ctx: &mut Context<'_, PowerMenu>) {
    let slide = ctx.me().slide;

    let hint = ctx.me().hint;
    if let Some(mut h) = ctx.at(hint) {
        if slide.is_some() {
            h.set_text("Release to cancel");
            h.set_color(ColorRole::Primary);
        } else {
            h.set_text("Slide to confirm");
            h.set_color(ColorRole::OnSurfaceVariant);
        }
    }

    let (on_surface, dim, arrow_color, fill_color) = (
        ctx.color(ColorRole::OnSurface),
        ctx.color(ColorRole::Outline),
        ctx.color(ColorRole::OnSecondaryFixed),
        ctx.color(ColorRole::SurfaceContainerLow),
    );

    for i in 0..ctx.me().rows.len() {
        let (action, fill, icon, label, arrow, slider) = {
            let r = &ctx.me().rows[i];
            (r.action, r.fill, r.icon, r.label, r.arrow, r.slider)
        };
        // `Some(progress)` on the row being slid.
        let progress = slide.filter(|s| s.row == i).map(|s| s.progress);
        let dimmed = slide.is_some() && progress.is_none();

        let fg = if dimmed { dim } else { on_surface };
        if let Some(mut c) = ctx.at(icon) {
            c.set_color(fg);
        }
        if let Some(mut c) = ctx.at(label) {
            c.set_color(fg);
            c.set_text(if progress.is_some() {
                action.busy_label()
            } else {
                action.label()
            });
        }
        if let Some(mut c) = ctx.at(arrow) {
            c.set_color(if dimmed { dim } else { arrow_color });
            c.set_display(display(progress.is_none()));
        }

        let p = progress.unwrap_or(0.0);
        // The shade ends under the slider's thumb. The slider spans the
        // row from its left edge, so the thumb's centre along the slider
        // is the shade's width.
        let mut thumb = 0.0;
        if let Some(mut c) = ctx.at(slider) {
            c.set_value(p);
            c.set_held(progress.is_some());
            c.set_display(display(progress.is_some()));
            thumb = c.me().thumb_center().round();
        }
        restyle(ctx, fill, |s| {
            s.display(display(progress.is_some())).width(px(thumb))
        });
        if let Some(mut c) = ctx.at(fill) {
            c.set_background(fill_color);
        }
    }
}

impl Widget for PowerMenu {
    type Builder = PowerMenuBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .hidden()
            .absolute()
            .inset(Insets::all(px(0.0)))
            .size(percent(100.0), percent(100.0))
            .column();
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceDim)));

        let header = s.spawn(
            me,
            div().style(padded_row(HEADER_HEIGHT).justify(Justify::SpaceBetween)),
        );
        s.spawn(
            header,
            text("POWER")
                .variant(TextVariant::LabelLarge)
                .color(ColorRole::OnSurfaceVariant),
        );
        let hint = s.spawn(
            header,
            text("Slide to confirm")
                .variant(TextVariant::BodyMedium)
                .color(ColorRole::OnSurfaceVariant),
        );

        let (arrow_sprite, cancel_sprite) = {
            let mut atlas = s.resource_mut::<Atlas>();
            let mut load = |svg| {
                atlas
                    .insert_svg(svg, ICON_SIZE)
                    .expect("embedded icon SVG should render and fit in the atlas")
            };
            (load(assets::icons::RIGHT_ARROW), load(assets::icons::CLEAR))
        };

        let divider_color = s.color(ColorRole::SurfaceContainer);
        let mut rows = Vec::new();
        for action in Action::ALL {
            rows.push(spawn_row(s, me, action, arrow_sprite));
            s.spawn(me, divider().background(divider_color));
        }

        let cancel = s.spawn(
            me,
            div().style(padded_row(CANCEL_HEIGHT).gap(percent(ICON_GAP))),
        );
        s.spawn(
            cancel,
            icon(cancel_sprite)
                .size(Size::new(ICON_SIZE as f32, ICON_SIZE as f32))
                .color(s.color(ColorRole::OnSurface))
                .style(LayoutStyle::default().shrink(0.0)),
        );
        s.spawn(
            cancel,
            text("CANCEL")
                .variant(TextVariant::LabelLarge)
                .color(ColorRole::OnSurfaceVariant),
        );

        for (i, row) in rows.iter().enumerate() {
            s.on::<Press>(row.node, move |ctx, e| start_slide(ctx, i, e));
            s.on::<PressMoved>(row.node, move_slide);
            s.on::<Release>(row.node, |ctx, e| end_slide(ctx, e.contact));
            s.on::<PressCancelled>(row.node, |ctx, e| end_slide(ctx, e.contact));
        }
        s.on::<Clicked>(cancel, |ctx, _| set_open(ctx, false));
        s.on::<OpenPowerMenu>(me, |ctx, _| set_open(ctx, true));
        s.on::<ClosePowerMenu>(me, |ctx, _| set_open(ctx, false));

        PowerMenu {
            hint,
            rows,
            slide: None,
        }
    }
}
