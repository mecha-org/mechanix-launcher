//! The PIN pad: three keys a row, an empty corner left of 0.

use mecha_wayland::mechanix_widgets::prelude::{
    Button, ButtonBuilder, ButtonContextExt, ButtonSize, ButtonVariant, button,
};
use mecha_wayland::prelude::*;

use super::LockView;
use crate::lock::UserInput;

/// A numpad key, portrait and landscape.
const KEY_W: f32 = 152.0;
const KEY_W_LANDSCAPE: f32 = 104.0;
pub(super) const KEY_H: f32 = 56.0;
const KEY_H_LANDSCAPE: f32 = 48.0;
const KEY_RADIUS: f32 = 6.0;
pub(super) const KEY_GAP_X: f32 = 10.0;
pub(super) const KEY_GAP_Y: f32 = 12.0;
pub(super) const KEY_TEXT: u16 = 24;
/// Between the dots and the pad, on top of the input's gap.
const PAD_SPACE: f32 = 24.0;
const ICON: u32 = 24;
/// The smallest a portrait key gets.
const KEY_W_MIN: f32 = 64.0;

/// The icons the numpad draws, loaded once.
#[derive(Clone, Copy)]
struct NumpadIcons {
    backspace: SpriteId,
}
impl Resource for NumpadIcons {}

pub(super) fn install(app: &mut App) {
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        NumpadIcons {
            backspace: atlas
                .insert_svg(assets::icons::BACKSPACE, ICON)
                .expect("the backspace icon loads"),
        }
    };
    app.insert_resource(icons);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Digit(char),
    Backspace,
}

impl Key {
    fn input(self) -> UserInput {
        match self {
            Key::Digit(d) => UserInput::Text(d.to_string()),
            Key::Backspace => UserInput::Backspace,
        }
    }
}

/// A key's size: fixed in landscape; in portrait the full width where the
/// window is wide enough, else a third of the width the padding leaves.
pub(super) fn key_size(landscape: bool, width: f32, pad: f32) -> (f32, f32) {
    if landscape {
        (KEY_W_LANDSCAPE, KEY_H_LANDSCAPE)
    } else {
        let fit = (width - 2.0 * pad - 2.0 * KEY_GAP_X) / 3.0;
        (fit.clamp(KEY_W_MIN, KEY_W), KEY_H)
    }
}

/// A standard button: a dark fill and a hairline border for a digit, bare
/// for backspace; lighter while pressed.
fn key_button<W: Widget>(s: &mut Spawner<'_, W>, key: Key, icons: NumpadIcons) -> ButtonBuilder {
    let b = button()
        .size(ButtonSize::XLarge)
        .radius(Corners::all(KEY_RADIUS))
        .width(px(KEY_W))
        .height(px(KEY_H))
        .padding(Insets::all(px(0.0)))
        .text_color(ColorRole::OnSurface)
        .pressed_background(s.color(ColorRole::SurfaceContainerHigh))
        .hover_background(s.color(ColorRole::SurfaceContainerLow));
    match key {
        Key::Digit(d) => b
            .label(d.to_string())
            .background(ColorRole::SurfaceContainerLow)
            .border(Insets::all(1.0), ColorRole::SurfaceContainerHighest),
        Key::Backspace => b
            .variant(ButtonVariant::Text)
            .icon(icons.backspace)
            .hover_background(Color::TRANSPARENT),
    }
}

/// A numpad cell, and its key unless it is the empty corner.
#[derive(Clone, Copy)]
pub(super) struct NumpadCell {
    pub(super) cell: Handle<Div>,
    pub(super) button: Option<Handle<Button>>,
}

/// The numpad, under `input`.
///
/// Each key sits in a cell a gap larger than it, centred, and the cell
/// takes the tap: a finger landing between two keys still presses the
/// nearer one, with the keys drawn as far apart as before.
pub(super) fn spawn(
    s: &mut Spawner<'_, LockView>,
    input: Handle<Div>,
) -> (Handle<Div>, Vec<NumpadCell>) {
    let numpad = s.spawn(
        input,
        div().style(
            LayoutStyle::default()
                .column()
                // Apart from the title and dots: the cells' own padding
                // makes up the rest.
                .margin(Insets::new(
                    px(PAD_SPACE - KEY_GAP_Y / 2.0),
                    px(0.0),
                    px(0.0),
                    px(0.0),
                ))
                .shrink(0.0),
        ),
    );
    let icons = *s.resource::<NumpadIcons>();
    let d = |c| Some(Key::Digit(c));
    let rows: [[Option<Key>; 3]; 4] = [
        [d('1'), d('2'), d('3')],
        [d('4'), d('5'), d('6')],
        [d('7'), d('8'), d('9')],
        // The corner stays empty: a full PIN submits itself.
        [None, d('0'), Some(Key::Backspace)],
    ];
    let cell_style = LayoutStyle::default()
        .size(px(KEY_W + KEY_GAP_X), px(KEY_H + KEY_GAP_Y))
        .padding(Insets::symmetric(px(KEY_GAP_X / 2.0), px(KEY_GAP_Y / 2.0)))
        .align_items(Align::Center)
        .justify(Justify::Center)
        .shrink(0.0);
    let mut cells = Vec::new();
    for row in rows {
        let r = s.spawn(numpad, div().style(LayoutStyle::default().row()));
        for key in row {
            let cell = s.spawn(r, div().style(cell_style.clone()));
            // The corner stays an empty cell, the size of a key.
            let button = key.map(|key| {
                let builder = key_button(s, key, icons);
                let button = s.spawn(cell, builder);
                s.on::<Clicked>(cell, move |ctx, _| ctx.signal(key.input()));
                // A press anywhere in the cell shows on the key.
                s.on::<Press>(cell, move |ctx, _| show_pressed(ctx, button, true));
                s.on::<Release>(cell, move |ctx, _| show_pressed(ctx, button, false));
                s.on::<PressCancelled>(cell, move |ctx, _| {
                    show_pressed(ctx, button, false);
                });
                button
            });
            cells.push(NumpadCell { cell, button });
        }
    }
    (numpad, cells)
}

/// Show `button` pressed or not, for a press on its cell. The button
/// shows its own presses itself; this covers the rest of the cell, and
/// leaves a disabled key be.
fn show_pressed(ctx: &mut Context<'_, LockView>, button: Handle<Button>, pressed: bool) {
    let Some(mut b) = ctx.at(button) else {
        return;
    };
    let state = b.me().state;
    if pressed && state != WidgetState::Disabled {
        b.set_state(WidgetState::Pressed);
    } else if !pressed && state == WidgetState::Pressed {
        b.set_state(WidgetState::Enabled);
    }
}
