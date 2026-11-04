//! Touch handling for the on-screen keyboard. Every touch on the keyboard
//! arrives here ([`attach`]), is resolved to a key (`resolve`, kept free of
//! widgets), tracked through press, slide and release (`tracker`), and turned
//! into keystrokes and [`KeyFeedback`] for the key widgets, which only draw.

mod defer;
mod flow;
mod glue;
pub(crate) mod long_press;
mod resolve;
mod tracker;

#[cfg(test)]
mod sim;

use mecha_wayland::prelude::*;

use crate::layout::{KeyAction, KeyFace, Keymap};
use crate::protocol::virtual_keyboard::keysym_text;
use crate::widget::key::Key;

pub(crate) use glue::{TouchState, attach, long_pressed, view_switched};
pub(crate) use tracker::TouchTracker;

/// The input layer's word to the UI: a key gained or lost the pressed state.
#[derive(Clone, Copy, Debug)]
pub(crate) enum KeyFeedback {
    /// The key is under the finger.
    Pressed { key: NodeId },
    /// The key is no longer under the finger.
    Released { key: NodeId },
}
impl Event for KeyFeedback {}

/// One key of a view, as the input layer sees it.
pub(crate) struct KeyEntry {
    /// The key's widget: its layout gives the key's rectangle.
    pub node: Handle<Key>,
    /// What the key does.
    pub action: KeyAction,
    /// The key's lowercase `a`-`z` letter, if it types one.
    pub letter: Option<char>,
    /// The key's long-press choices (lowercase), if any.
    pub choices: Vec<String>,
}

impl KeyEntry {
    /// The entry for a spawned key with this action and long-press choices.
    pub(crate) fn new(node: Handle<Key>, action: KeyAction, choices: Vec<String>) -> Self {
        let letter = letter_of(&action);
        Self {
            node,
            action,
            letter,
            choices,
        }
    }
}

/// The single character a key's action types, if it types exactly one.
pub(crate) fn single_char(action: &KeyAction) -> Option<char> {
    let text = match action {
        KeyAction::EmitKeysym(ks) => keysym_text(*ks)?,
        KeyAction::EmitText(t) => t.clone(),
        _ => return None,
    };
    let mut chars = text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// The lowercase letter a key types, counting only `a`-`z` (either case).
pub(crate) fn letter_of(action: &KeyAction) -> Option<char> {
    let c = single_char(action)?.to_ascii_lowercase();
    c.is_ascii_lowercase().then_some(c)
}

/// Per view, in keymap order: whether any of its keys types a letter.
pub(crate) fn letter_views(keymap: &Keymap) -> Vec<bool> {
    keymap
        .views
        .iter()
        .map(|v| v.keys().any(|k| letter_of(&k.action).is_some()))
        .collect()
}

/// The text a key's pop-up shows: its label, for a text-faced key that types
/// visible text. `None` for whitespace, control keys, icons and every
/// non-typing key.
pub(crate) fn popup_text(key: &crate::layout::Key) -> Option<String> {
    let KeyFace::Text(label) = &key.face else {
        return None;
    };
    let types = match &key.action {
        KeyAction::EmitKeysym(ks) => keysym_text(*ks),
        KeyAction::EmitText(t) => Some(t.clone()),
        _ => None,
    };
    types
        .filter(|t| !t.trim().is_empty())
        .map(|_| label.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{self, KeyFace, Keymap};

    /// The bundled keymap's key in `view` whose face is the text `face`.
    fn key<'a>(keymap: &'a Keymap, view: &str, face: &str) -> &'a layout::Key {
        keymap
            .view(view)
            .unwrap()
            .keys()
            .find(|k| matches!(&k.face, KeyFace::Text(t) if t == face))
            .unwrap_or_else(|| panic!("no `{face}` key in `{view}`"))
    }

    /// The bundled keymap's key in `view` with an icon face.
    fn icon_key<'a>(keymap: &'a Keymap, view: &str, icon: &str) -> &'a layout::Key {
        keymap
            .view(view)
            .unwrap()
            .keys()
            .find(|k| matches!(&k.face, KeyFace::Icon(i) if i == icon))
            .unwrap_or_else(|| panic!("no `{icon}` icon key in `{view}`"))
    }

    #[test]
    fn letter_of_counts_a_to_z_lowercased() {
        let keymap = layout::load_keymap();
        assert_eq!(letter_of(&key(&keymap, "upper", "Q").action), Some('q'));
        assert_eq!(letter_of(&key(&keymap, "base", "q").action), Some('q'));
        assert_eq!(letter_of(&key(&keymap, "numbers", "1").action), None);
        assert_eq!(letter_of(&key(&keymap, "base", ".").action), None);
        assert_eq!(letter_of(&key(&keymap, "base", " ").action), None);
        assert_eq!(letter_of(&key(&keymap, "eschars", "ä").action), None);
    }

    #[test]
    fn letter_views_flags() {
        let keymap = layout::load_keymap();
        let flags = letter_views(&keymap);
        assert_eq!(flags.len(), keymap.views.len());
        let flag = |name: &str| flags[keymap.views.iter().position(|v| v.name == name).unwrap()];
        assert!(flag("base"));
        assert!(flag("upper"));
        assert!(!flag("numbers"));
        assert!(!flag("symbols"));
    }

    #[test]
    fn popup_text_rules() {
        let keymap = layout::load_keymap();
        assert_eq!(popup_text(key(&keymap, "base", "a")), Some("a".to_string()));
        assert_eq!(
            popup_text(key(&keymap, "upper", "A")),
            Some("A".to_string())
        );
        assert_eq!(
            popup_text(key(&keymap, "numbers", "1")),
            Some("1".to_string())
        );
        assert_eq!(popup_text(key(&keymap, "base", " ")), None);
        assert_eq!(
            popup_text(icon_key(&keymap, "base", "edit-clear-symbolic")),
            None
        );
        assert_eq!(
            popup_text(icon_key(&keymap, "base", "key-shift-symbolic")),
            None
        );
        assert_eq!(
            popup_text(icon_key(&keymap, "base", "key-enter-symbolic")),
            None
        );
    }
}
