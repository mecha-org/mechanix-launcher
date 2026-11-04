//! The keymap IR: the resolved views, rows and keys a layout compiles down to.

use xkbcommon::xkb::Keysym;

use super::style::KeyLook;
use super::{KeyboardStyle, PredictionConfig};

/// The resolved keyboard: its views and the container style.
#[derive(Debug)]
pub struct Keymap {
    pub views: Vec<View>,
    pub keyboard: KeyboardStyle,
    pub prediction: PredictionConfig,
}

/// One selectable arrangement of keys (e.g. `base`, `upper`).
#[derive(Debug)]
pub struct View {
    pub name: String,
    pub rows: Vec<Row>,
}

/// One horizontal line of keys within a view.
#[derive(Debug)]
pub struct Row {
    pub keys: Vec<Key>,
}

/// What a key draws in its cell: a text label *or* a symbolic icon, never both.
/// The icon is its name (an SVG's file stem under `resources/icons`); the `Key`
/// widget looks up the rasterized sprite in the `Icons` resource.
#[derive(Debug, Clone)]
pub enum KeyFace {
    Text(String),
    Icon(String),
}

/// A latchable modifier, named as squeekboard's `modifier:` field names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    Control,
    Alt,
}

impl Modifier {
    pub const ALL: [Modifier; 2] = [Modifier::Control, Modifier::Alt];

    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "Control" => Some(Self::Control),
            "Alt" => Some(Self::Alt),
            _ => None,
        }
    }
}

/// What a [`Key`] does when tapped — the behavioural counterpart to [`KeyFace`].
/// Resolved at IR-build time from a squeekboard button's `keysym`/`text`/`action`
/// fields.
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// Emit a single xkb keysym (letters, digits, Return, BackSpace…).
    EmitKeysym(Keysym),
    /// Emit literal text, one keysym per char (e.g. the space key's `" "`).
    EmitText(String),
    /// Switch to a named view and stay there (squeekboard `set_view`).
    SetView(String),
    /// Flip between two views: go to `lock`, or back to `unlock` when `lock`
    /// is already current (squeekboard `locking`).
    ToggleView { lock: String, unlock: String },
    /// Arm a modifier for the next keystroke only (one-shot latch). Several
    /// modifiers can be armed at once; tapping again disarms.
    LatchModifier(Modifier),
    /// Hide the keyboard (`action: hide_keyboard`).
    HideKeyboard,
    /// Flip whether the shown keyboard reserves its height, so apps shrink
    /// above it rather than sit under it (`action: toggle_exclusive_zone`).
    ToggleExclusiveZone,
    /// An action not wired this pass (prefs, modifiers other than Control/Alt).
    /// The key still draws and hit-tests; tapping it logs but types nothing.
    Unhandled(String),
}

/// One drawable key: what to draw, what it does, and where (logical units).
#[derive(Debug, Clone)]
pub struct Key {
    pub face: KeyFace,
    pub action: KeyAction,
    /// The key's outline width (logical units).
    pub width: f32,
    /// Colours per [`super::style::KeyState`], from the button's `key_style:` (or
    /// `normal`) and the `latched` style.
    pub look: KeyLook,
}

impl View {
    /// Every key in the view, flattened across its rows.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.rows.iter().flat_map(|row| row.keys.iter())
    }
}

impl Keymap {
    #[cfg(test)]
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.views.iter().flat_map(|view| view.keys())
    }

    #[cfg(test)]
    pub fn view(&self, name: &str) -> Option<&View> {
        self.views.iter().find(|v| v.name == name)
    }
}
