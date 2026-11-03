//! Selecting which keymap view is shown: by the focused field's content
//! purpose, by an explicit `set_view`/`locking` key tap, and the surface-wide
//! [`SwitchView`] event the keyboard widget subscribes to.

use mecha_wayland::prelude::*;

use crate::input::end_word;
use crate::protocol::input_method::TextInputFocused;
use crate::widget::keyboard::Keyboard;

/// The default view: shown when the keyboard first appears and for text
/// fields with no more specific purpose. Must exist in `layout.yaml`.
pub(crate) const BASE_VIEW: &str = "base";
/// The view shown for `terminal`-purpose fields.
const TERMINAL_VIEW: &str = "terminal";
/// The view shown for `number`/`digits`-purpose fields.
const NUMBER_VIEW: &str = "numbers";

/// The view to show for a focused text field's content purpose.
fn view_for_purpose(purpose: ZwpTextInputV3ContentPurpose) -> &'static str {
    match purpose {
        ZwpTextInputV3ContentPurpose::Terminal => TERMINAL_VIEW,
        ZwpTextInputV3ContentPurpose::Number | ZwpTextInputV3ContentPurpose::Digits => NUMBER_VIEW,
        _ => BASE_VIEW,
    }
}

/// Ask the keyboard to show a named view.
pub(crate) struct SwitchView {
    pub(crate) view: &'static str,
}
impl Event for SwitchView {}

/// Switch the current view to `target`, hiding the old and showing the new via
/// `Display::Hidden` / `Display::Flex`. Unknown targets are logged and ignored.
/// A view without letters finishes the word being typed; switching between
/// letter views (Shift) keeps it.
pub(crate) fn switch_view(ctx: &mut Context<'_, Keyboard>, target: &str) {
    let Some(idx) = ctx.me().index_of(target) else {
        tracing::warn!(view = %target, "view switch to unknown view; ignored");
        return;
    };
    let old_idx = ctx.me().current_view;
    if idx == old_idx {
        return;
    }
    let (old, new) = {
        let me = ctx.me();
        (me.views[old_idx].node, me.views[idx].node)
    };
    ctx.at(old).unwrap().set_display(Display::Hidden);
    ctx.at(new).unwrap().set_display(Display::Flex);
    ctx.me().current_view = idx;
    if !ctx.me().letter_views[idx] {
        end_word(ctx);
    }
    tracing::info!(view = %target, "switched view");
}

/// System: a text field gained focus or changed purpose — show the view for
/// its purpose.
pub(crate) fn on_text_input_focused(app: &mut App, focused: &TextInputFocused) {
    let view = view_for_purpose(focused.purpose);
    let targets: Vec<NodeId> = app.widgets::<Keyboard>().map(|(id, _)| id).collect();
    app.emit(SwitchView { view }, targets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    #[test]
    fn purpose_picks_view() {
        use ZwpTextInputV3ContentPurpose as P;
        assert_eq!(view_for_purpose(P::Terminal), TERMINAL_VIEW);
        assert_eq!(view_for_purpose(P::Number), NUMBER_VIEW);
        assert_eq!(view_for_purpose(P::Digits), NUMBER_VIEW);
        assert_eq!(view_for_purpose(P::Normal), BASE_VIEW);
        assert_eq!(view_for_purpose(P::Email), BASE_VIEW);
    }

    #[test]
    fn auto_switch_views_exist_in_bundled_layout() {
        let keymap = layout::load_keymap();
        for view in [BASE_VIEW, TERMINAL_VIEW, NUMBER_VIEW] {
            assert!(
                keymap.views.iter().any(|v| v.name == view),
                "layout.yaml has no `{view}` view"
            );
        }
    }
}
