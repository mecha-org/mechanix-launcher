//! Where the view's parts go, for its window's shape and what it shows.
//!
//! - Portrait: a column, the time at the top left, the input centred in
//!   the middle, the bar centred at the bottom.
//! - Landscape: two halves, the time and the bar on the left, the input
//!   centred on the right.
//! - With the keyboard up, either way: a column ending in the keyboard,
//!   which needs the width; landscape drops the time for room.

use ::status_bar::DEFAULT_BAR_HEIGHT;
use mecha_wayland::mechanix_widgets::prelude::ButtonContextExt;
use mecha_wayland::prelude::*;

use super::numpad::{KEY_GAP_X, KEY_GAP_Y, key_size};
use super::{Entry, LockView, display, shrink_time};

const PADDING: f32 = 48.0;
const PADDING_LANDSCAPE: f32 = 24.0;
/// Narrower than this, portrait pads like landscape.
const NARROW: f32 = 480.0;

/// Set a node's style, keeping its display, if it changed.
fn restyle<V: Widget>(ctx: &mut Context<'_, LockView>, node: Handle<V>, style: LayoutStyle) {
    let mut c = ctx.at(node).unwrap();
    let display = c.style().display;
    c.set_style(style.display(display));
}

/// Resize a node, keeping the rest of its style.
fn resize<V: Widget>(ctx: &mut Context<'_, LockView>, node: Handle<V>, w: f32, h: f32) {
    let mut c = ctx.at(node).unwrap();
    c.set_width(px(w));
    c.set_height(px(h));
}

/// Lay the view out for its window's shape and what it shows.
pub(super) fn layout(ctx: &mut Context<'_, LockView>) {
    let (expanded, kind, dots_shown, busy) = ctx
        .me()
        .shown
        .as_ref()
        .map_or((false, Entry::None, false, false), |m| {
            (m.expanded, m.entry_kind, m.dots(), m.busy)
        });
    let keyboard_up = ctx.me().keys_shown();
    let (p, size) = (ctx.me().parts, ctx.me().size);
    let body_h = (size.height - DEFAULT_BAR_HEIGHT).max(0.0);
    let landscape = size.width > body_h;
    // The full padding where there is room for it.
    let pad = if landscape || size.width < NARROW {
        PADDING_LANDSCAPE
    } else {
        PADDING
    };
    // The bar's items, centred in it.
    let bar_row = LayoutStyle::default()
        .row()
        .justify(Justify::Center)
        .align_items(Align::Center)
        .gap(px(16.0))
        .shrink(0.0);
    let input_base = LayoutStyle::default()
        .column()
        .justify(Justify::Center)
        .gap(px(16.0))
        .shrink(0.0);
    let time_base = LayoutStyle::default().column().gap(px(8.0)).shrink(0.0);
    let body_base = LayoutStyle::default()
        .width(percent(100.0))
        .grow(1.0)
        .min_height(px(0.0));

    if landscape && !keyboard_up {
        // Two halves with the same padding: the time top left and the bar
        // across the bottom of the left half; the input filling the right
        // half, its contents centred both ways.
        restyle(ctx, p.body, body_base.relative());
        let (pd, a, zero, half) = (px(pad), auto(), px(0.0), percent(50.0));
        let sides = Insets::symmetric(pd, zero);
        let at = |st: LayoutStyle, top, right, bottom, left| {
            st.absolute().inset(Insets::new(top, right, bottom, left))
        };
        restyle(ctx, p.time, at(time_base.clone(), pd, a, a, pd));
        restyle(
            ctx,
            p.bar,
            at(bar_row.clone().width(half).padding(sides), a, a, pd, zero),
        );
        restyle(
            ctx,
            p.input,
            at(
                input_base.clone().align_items(Align::Center).padding(sides),
                zero,
                zero,
                zero,
                half,
            ),
        );
    } else {
        // A column. Without the keyboard: padded all round, the parts
        // spread top to bottom. With it: the keyboard last, edge to edge;
        // the input takes the height left, and the bar sits as far above
        // the keys as it does above the edge without them. Landscape is
        // short: no top padding, so the keys fit.
        let top = if landscape { 0.0 } else { pad / 2.0 };
        let style = if keyboard_up {
            body_base
                .column()
                .padding(Insets::new(px(top), px(0.0), px(0.0), px(0.0)))
        } else {
            body_base
                .column()
                .justify(Justify::SpaceBetween)
                .align_items(Align::Start)
                .padding(Insets::all(px(pad)))
        };
        restyle(ctx, p.body, style);
        let side = if keyboard_up {
            Insets::symmetric(px(pad), px(0.0))
        } else {
            Insets::all(px(0.0))
        };
        restyle(ctx, p.time, time_base.clone().padding(side));
        restyle(
            ctx,
            p.input,
            // What is asked, the dots or field, and the keys, centred
            // across the width.
            {
                let st = input_base
                    .clone()
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .padding(side);
                if keyboard_up {
                    // It takes the height the keys leave, and gives it up
                    // before they lose any.
                    let st = st.grow(1.0).shrink(1.0).min_height(px(0.0));
                    if landscape { st.gap(px(8.0)) } else { st }
                } else {
                    st
                }
            },
        );
        let bar_style = bar_row.clone().width(percent(100.0)).padding(side);
        restyle(
            ctx,
            p.bar,
            if keyboard_up {
                bar_style.margin(Insets::new(px(0.0), px(0.0), px(pad), px(0.0)))
            } else {
                bar_style
            },
        );
    }
    ctx.at(p.time)
        .unwrap()
        .set_display(display(!(keyboard_up && landscape)));
    // Smaller and a little higher everywhere but the glance screen.
    let glancing = ctx.me().glancing();
    shrink_time(ctx, if glancing { 0.0 } else { 1.0 });
    ctx.at(p.input)
        .unwrap()
        .set_display(display(expanded && kind != Entry::Unlock));
    ctx.at(p.numpad)
        .unwrap()
        .set_display(display(kind == Entry::Pin && dots_shown));
    // Out of the column while the keys hide: empty, it would still be
    // spaced in, and push the bar off the bottom.
    ctx.at(p.keyboard)
        .unwrap()
        .set_display(display(keyboard_up));
    let (w, h) = key_size(landscape, size.width, pad);
    for c in ctx.me().cells.clone() {
        resize(ctx, c.cell, w + KEY_GAP_X, h + KEY_GAP_Y);
        if let Some(k) = c.button {
            resize(ctx, k, w, h);
            ctx.at(k).unwrap().set_disabled(busy);
        }
    }
}
