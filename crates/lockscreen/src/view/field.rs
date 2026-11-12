//! The field for an answer that isn't a PIN: a button over its right end
//! that shows or hides a secret as typed, and beside it the button that
//! sends the answer, as Return does.

use mecha_wayland::mechanix_widgets::prelude::{
    Button, ButtonContextExt, ButtonSize, ButtonVariant, FontContextExt, FontWeight, button,
};
use mecha_wayland::prelude::*;

use super::numpad::{KEY_H, KEY_TEXT};
use super::{LockView, display};
use crate::lock::UserInput;

/// The widest the field and its send button get.
const FIELD_W: f32 = 476.0;
const ICON: u32 = 24;
/// Between the field and the send button.
const GAP: f32 = 8.0;

/// The buttons' icons, loaded once. The eye's is what a press does.
#[derive(Clone, Copy)]
struct Icons {
    show: SpriteId,
    hide: SpriteId,
    send: SpriteId,
}
impl Resource for Icons {}

pub(super) fn install(app: &mut App) {
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        let mut load = |svg, what| {
            atlas
                .insert_svg(svg, ICON)
                .unwrap_or_else(|e| panic!("the {what} icon loads: {e:?}"))
        };
        Icons {
            show: load(assets::icons::EYE, "eye"),
            hide: load(assets::icons::EYE_SLASH, "eye-slash"),
            send: load(assets::icons::ARROW_RIGHT, "arrow"),
        }
    };
    app.insert_resource(icons);
}

#[derive(Clone, Copy)]
pub(super) struct Field {
    /// The field and its buttons: shown while the field takes the answer.
    row: Handle<Div>,
    pub(super) input: Handle<Input>,
    /// Shows or hides the text: there while the answer is secret.
    reveal: Handle<Div>,
    eye: Handle<Icon>,
    /// Sends the answer: for a pointer, where the keys have Return.
    send: Handle<Button>,
}

/// The field, hidden. Return, or the send button, answers with what it
/// holds. Focused, it raises the on-screen keyboard as any field does.
pub(super) fn spawn(s: &mut Spawner<'_, LockView>, parent: Handle<Div>) -> Field {
    let row = s.spawn(
        parent,
        div().style(
            LayoutStyle::default()
                .row()
                .align_items(Align::Center)
                .gap(px(GAP))
                .width(percent(100.0))
                .max_width(px(FIELD_W))
                .shrink(0.0)
                .hidden(),
        ),
    );
    // The field with the eye over its right end.
    let field = s.spawn(
        row,
        div().style(
            LayoutStyle::default()
                .relative()
                .grow(1.0)
                .min_width(px(0.0)),
        ),
    );
    let font = s.font(FontWeight::Regular);
    // `style` is the field's inner box. Its text never runs under the eye.
    let box_style = LayoutStyle::default()
        .row()
        .align_items(Align::Center)
        .size(percent(100.0), px(KEY_H))
        .padding(Insets::new(px(0.0), px(KEY_H), px(0.0), px(16.0)));
    let builder = input(font)
        .size(KEY_TEXT)
        .color(s.color(ColorRole::OnSurface))
        .caret(s.color(ColorRole::Primary))
        .background(s.color(ColorRole::SurfaceContainerLow))
        .border(1.0, s.color(ColorRole::SurfaceContainerHighest))
        .content_hint(ContentHint::HIDDENTEXT | ContentHint::SENSITIVEDATA)
        .content_purpose(ContentPurpose::Password)
        .style(box_style);
    let input = s.spawn(field, builder);
    *s.component_mut::<LayoutStyle>(input).unwrap() = LayoutStyle::default().width(percent(100.0));
    // Over the field's right end, so a press on it is the field's too, and
    // keeps its focus.
    let reveal = s.spawn(
        field,
        div().style(
            LayoutStyle::default()
                .absolute()
                .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
                .width(px(KEY_H))
                .center(),
        ),
    );
    let icons = *s.resource::<Icons>();
    let eye = s.spawn(
        reveal,
        icon(icons.show).color(s.color(ColorRole::OnSurfaceVariant)),
    );
    // Square like the field, and as tall; in the accent, as the date is.
    let send = s.spawn(
        row,
        button()
            .variant(ButtonVariant::Text)
            .size(ButtonSize::XLarge)
            .width(px(KEY_H))
            .height(px(KEY_H))
            .padding(Insets::all(px(0.0)))
            .radius(Corners::all(0.0))
            .text_color(ColorRole::Primary)
            .background(ColorRole::SurfaceContainerLow)
            .border(Insets::all(1.0), ColorRole::Primary)
            .pressed_background(s.color(ColorRole::SurfaceContainerHigh))
            .icon(icons.send),
    );
    s.on::<InputSubmit>(input, move |ctx, _| submit(ctx, input));
    s.on::<Clicked>(send, move |ctx, _| submit(ctx, input));
    s.on::<Clicked>(reveal, |ctx, _| {
        ctx.me().revealed ^= true;
        sync(ctx);
    });
    Field {
        row,
        input,
        reveal,
        eye,
        send,
    }
}

/// Send what the field holds. It keeps the text, disabled, while it is
/// checked; the view empties it after.
fn submit(ctx: &mut Context<'_, LockView>, input: Handle<Input>) {
    let Some(answer) = ctx
        .widget::<Input>(input.id())
        .filter(|i| !i.is_disabled())
        .map(Input::secret)
    else {
        return;
    };
    ctx.signal(UserInput::Answer(answer));
}

/// Show the field while it takes the answer, disabled with its send button
/// while the answer is checked; a secret masked unless revealed, with the
/// button to switch.
pub(super) fn sync(ctx: &mut Context<'_, LockView>) {
    let (f, shown, typing, secret, revealed) = {
        let v = ctx.me();
        let secret = v.shown.as_ref().is_some_and(|m| m.hidden);
        (
            v.parts.field,
            v.field_shown(),
            v.typing(),
            secret,
            v.revealed,
        )
    };
    let icons = *ctx.resource::<Icons>();
    ctx.at(f.row).unwrap().set_display(display(shown));
    {
        let mut input = ctx.at(f.input).unwrap();
        input.set_hidden(secret && !revealed);
        input.set_disabled(shown && !typing);
    }
    ctx.at(f.send).unwrap().set_disabled(!typing);
    ctx.at(f.reveal).unwrap().set_display(display(secret));
    ctx.at(f.eye)
        .unwrap()
        .set_sprite(if revealed { icons.hide } else { icons.show });
}
