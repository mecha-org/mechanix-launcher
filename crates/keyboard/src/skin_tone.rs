//! The emoji skin tone. Long-pressing an emoji that takes one (👍, 🏃‍♀️)
//! offers it untoned and in the five tones; the one picked is remembered
//! (in memory, in the [`Emoji`] resource) and shown and typed from then on
//! by every emoji that takes a tone: on the emoji pages' keys, in the
//! suggestion bar and in the emoji search results.

use mecha_wayland::prelude::*;

use crate::layout::KeyAction;
use crate::layout::emoji::Emoji;
use crate::widget::key::{Key, Label};
use crate::widget::keyboard::Keyboard;
use crate::widget::suggestion_bar::BarFeedback;

/// The keyboard's cue to pick a skin tone: an index into
/// [`SKIN_TONES`](crate::layout::emoji::SKIN_TONES), `None` for the
/// default. Handled by [`pick`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct PickSkinTone(pub(crate) Option<usize>);
impl Event for PickSkinTone {}

/// Pick skin tone `tone` (an index into
/// [`SKIN_TONES`](crate::layout::emoji::SKIN_TONES), `None` for the
/// default): redraw every key and bar emoji that takes one in it, and make
/// those keys type it. Nothing is redrawn if it is the tone already picked.
pub(crate) fn pick(ctx: &mut Context<'_, Keyboard>, tone: Option<usize>) {
    if ctx.resource::<Emoji>().skin_tone() == tone {
        return;
    }
    ctx.resource_mut::<Emoji>().set_skin_tone(tone);
    retone_keys(ctx);
    let (bar, results) = {
        let me = ctx.me();
        (me.bar.id(), me.results_bar.id())
    };
    ctx.emit(BarFeedback::Retone, [bar, results]);
}

/// The choice of a skin-tone key's row that `tone` is: 0 untoned, then
/// light to dark.
pub(crate) fn choice_of(tone: Option<usize>) -> usize {
    tone.map_or(0, |t| t + 1)
}

/// The skin tone choice `choice` of a skin-tone key's row picks.
pub(crate) fn tone_of(choice: usize) -> Option<usize> {
    choice.checked_sub(1)
}

/// Give every emoji key that takes a skin tone, in every view, the picked
/// one: its face and what it types.
fn retone_keys(ctx: &mut Context<'_, Keyboard>) {
    let keys: Vec<(usize, usize, String, Handle<Key>)> = ctx
        .me()
        .keys
        .iter()
        .enumerate()
        .flat_map(|(v, view)| {
            view.iter()
                .enumerate()
                .filter_map(move |(i, k)| Some((v, i, k.skin_tone_base.clone()?, k.node)))
        })
        .collect();
    for (v, i, base, node) in keys {
        let (emoji, sprite) = {
            let (mut emoji, mut atlas) = ctx.fetch::<(ResMut<Emoji>, ResMut<Atlas>)>();
            let toned = emoji.toned(&base);
            let sprite = emoji.ensure(&mut atlas, &toned);
            (toned, sprite)
        };
        ctx.me().keys[v][i].action = KeyAction::EmitText(emoji);
        let image = ctx.widget::<Key>(node).and_then(|k| match k.label {
            Label::Image(h) => Some(h),
            _ => None,
        });
        if let (Some(image), Some(e)) = (image, sprite)
            && let Some(mut at) = ctx.at(image)
        {
            at.set_sprite(e.sprite);
            at.set_style(LayoutStyle::default().size(px(e.width), px(e.height)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_and_tones_map_both_ways() {
        assert_eq!(choice_of(None), 0);
        assert_eq!(choice_of(Some(4)), 5);
        assert_eq!(tone_of(0), None);
        assert_eq!(tone_of(3), Some(2));
        for c in 0..6 {
            assert_eq!(choice_of(tone_of(c)), c);
        }
    }
}
