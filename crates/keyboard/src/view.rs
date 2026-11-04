//! Selecting which keymap view is shown: by the focused field's family (its
//! content purpose and hint), by a page key tap within that family, and the
//! surface-wide [`SwitchView`] event the keyboard widget subscribes to.

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
    // The keymap's view order comes from a hash map, so ties are broken by
    // name (`base` first) to keep the choice deterministic.
    let find = |f: Family| {
        views
            .clone()
            .enumerate()
            .filter(|(_, (_, vf, vp))| *vf == f && *vp == page)
            .min_by_key(|(_, (name, ..))| (*name != BASE_VIEW, *name))
            .map(|(i, _)| i)
    };
    if let Some(idx) = find(family) {
        return Some(idx);
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
    fn bundled_layout_maps_every_family_to_its_letters_page() {
        let keymap = layout::load_keymap();
        let views = keymap_views(&keymap);
        let name = |f| pick_view(views.iter().copied(), f, Page::Letters).map(|i| views[i].0);
        assert_eq!(name(Family::Terminal), Some("terminal"));
        assert_eq!(name(Family::Number), Some("number"));
        assert_eq!(name(Family::Phone), Some("phone"));
        assert_eq!(name(Family::Text), Some(BASE_VIEW));
        assert_eq!(name(Family::Email), Some("email"));
        assert_eq!(name(Family::Url), Some("url"));
        assert_eq!(name(Family::Password), Some("password"));
        let sym = |f| pick_view(views.iter().copied(), f, Page::Symbols).map(|i| views[i].0);
        assert_eq!(sym(Family::Terminal), Some("terminal_symbols"));
        assert_eq!(sym(Family::Email), Some("symbols"));
    }

    #[test]
    fn pick_is_independent_of_view_order() {
        let keymap = layout::load_keymap();
        let mut views = keymap_views(&keymap);
        fn expect<'a>(views: &[(&'a str, Family, Page)]) -> [Option<&'a str>; 4] {
            [
                Family::Terminal,
                Family::Number,
                Family::Text,
                Family::Email,
            ]
            .map(|f| pick_view(views.iter().copied(), f, Page::Letters).map(|i| views[i].0))
        }
        let want = expect(&views);
        for rot in 0..views.len() {
            views.rotate_left(1);
            assert_eq!(expect(&views), want, "rotation {rot}");
        }
        views.reverse();
        assert_eq!(expect(&views), want);
        assert_eq!(want[2], Some(BASE_VIEW));
    }

    #[test]
    fn auto_switch_views_exist_in_bundled_layout() {
        let keymap = layout::load_keymap();
        for view in [BASE_VIEW, "terminal", "number"] {
            assert!(
                keymap.views.iter().any(|v| v.name == view),
                "layout.yaml has no `{view}` view"
            );
        }
    }
}
