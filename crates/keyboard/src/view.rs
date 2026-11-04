//! Selecting which keymap view is shown: by the focused field's content
//! purpose, by an explicit `set_view`/`locking` key tap, and the surface-wide
//! [`SwitchView`] event the keyboard widget subscribes to.

use mecha_wayland::prelude::*;

use crate::field::Field;
use crate::input::end_word;
use crate::layout::{Family, Page};
use crate::protocol::input_method::TextInputFocused;
use crate::widget::keyboard::{Keyboard, ViewNode};

/// The default view: shown when the keyboard first appears and for text
/// fields with no more specific purpose. Must exist in `layout.yaml`.
pub(crate) const BASE_VIEW: &str = "base";
/// The view for a field family and page: the view with exactly that
/// (family, page); else the first text-family view of that page. `None` when
/// the layout has neither.
pub(crate) fn view_for(views: &[ViewNode], family: Family, page: Page) -> Option<usize> {
    pick_view(
        views.iter().map(|v| (v.name.as_str(), v.family, v.page)),
        family,
        page,
    )
}

fn pick_view<'a>(
    views: impl Iterator<Item = (&'a str, Family, Page)> + Clone,
    family: Family,
    page: Page,
) -> Option<usize> {
    let find = |f: Family| views.clone().position(|(_, vf, vp)| vf == f && vp == page);
    if let Some(idx) = find(family) {
        return Some(idx);
    }
    // Legacy names until the AOSP layout lands (Task 11): today's layout has
    // no family views, so its letters pages are found by view name.
    if page == Page::Letters {
        let legacy = match family {
            Family::Terminal => Some("terminal"),
            Family::Number => Some("numbers"),
            _ => None,
        };
        if let Some(idx) = legacy.and_then(|n| views.clone().position(|(name, ..)| name == n)) {
            return Some(idx);
        }
    }
    find(Family::Text)
}

/// Ask the keyboard to show a named view.
pub(crate) struct SwitchView {
    pub(crate) field: Field,
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
    // The surface keeps its size; its zone and input region follow the view.
    crate::widget::surface::view_shown(ctx);
    tracing::info!(view = %target, "switched view");
}

/// System: a text field gained focus or changed purpose — show the view for
/// its purpose.
pub(crate) fn on_text_input_focused(app: &mut App, focused: &TextInputFocused) {
    let field = Field::from_purpose(focused.purpose, focused.hint);
    let targets: Vec<NodeId> = app.widgets::<Keyboard>().map(|(id, _)| id).collect();
    app.emit(SwitchView { field }, targets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    fn keymap_views(keymap: &layout::Keymap) -> Vec<(&str, Family, Page)> {
        keymap
            .views
            .iter()
            .map(|v| (v.name.as_str(), v.family, v.page))
            .collect()
    }

    #[test]
    fn view_for_falls_back_to_text_family() {
        let views = [
            ("base", Family::Text, Page::Letters),
            ("symbols", Family::Text, Page::Symbols),
            ("email", Family::Email, Page::Letters),
        ];
        let idx = pick_view(views.iter().copied(), Family::Email, Page::Symbols);
        assert_eq!(idx, Some(1));
        assert_eq!(
            pick_view(views.iter().copied(), Family::Email, Page::Emoji),
            None
        );
    }

    #[test]
    fn view_for_prefers_own_family() {
        let views = [
            ("symbols", Family::Text, Page::Symbols),
            ("terminal_symbols", Family::Terminal, Page::Symbols),
        ];
        let idx = pick_view(views.iter().copied(), Family::Terminal, Page::Symbols);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn todays_layout_still_maps_terminal_and_numbers() {
        let keymap = layout::load_keymap();
        let views = keymap_views(&keymap);
        let name = |f| pick_view(views.iter().copied(), f, Page::Letters).map(|i| views[i].0);
        assert_eq!(name(Family::Terminal), Some("terminal"));
        assert_eq!(name(Family::Number), Some("numbers"));
        assert_eq!(name(Family::Text), Some(BASE_VIEW));
        assert_eq!(name(Family::Email), Some(BASE_VIEW));
    }

    #[test]
    fn auto_switch_views_exist_in_bundled_layout() {
        let keymap = layout::load_keymap();
        for view in [BASE_VIEW, "terminal", "numbers"] {
            assert!(
                keymap.views.iter().any(|v| v.name == view),
                "layout.yaml has no `{view}` view"
            );
        }
    }
}
