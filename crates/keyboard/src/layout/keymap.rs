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
// family/page/height/spacer/token/Page are consumed by later tasks.
#[allow(dead_code)]
pub struct View {
    pub name: String,
    pub family: Family,
    pub page: Page,
    pub rows: Vec<Row>,
}

/// The kind of text field a view is meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Text,
    Email,
    Url,
    Password,
    Number,
    Phone,
    Terminal,
}

impl Family {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "email" => Some(Self::Email),
            "url" => Some(Self::Url),
            "password" => Some(Self::Password),
            "number" => Some(Self::Number),
            "phone" => Some(Self::Phone),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

/// Which page of a family a view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Letters,
    Symbols,
    MoreSymbols,
    Emoji,
    PhoneSymbols,
}

impl Page {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "letters" => Some(Self::Letters),
            "symbols" => Some(Self::Symbols),
            "more_symbols" => Some(Self::MoreSymbols),
            "emoji" => Some(Self::Emoji),
            "phone_symbols" => Some(Self::PhoneSymbols),
            _ => None,
        }
    }
}

/// One horizontal line of cells within a view.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Height weight relative to a key row (`1.0`).
    pub height: f32,
}

impl Row {
    /// The row's keys, spacers skipped.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.cells.iter().filter_map(|c| match c {
            Cell::Key(k) => Some(k),
            Cell::Spacer(_) => None,
        })
    }
}

/// A slot in a row: a key, or blank space of the given width (outline units).
#[derive(Debug)]
#[allow(dead_code)]
pub enum Cell {
    Key(Key),
    Spacer(f32),
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
#[allow(dead_code)]
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
    /// Show a page of the current family.
    Page(Page),
    /// Toggle shift (tap) / caps lock (double tap).
    Shift,
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
#[allow(dead_code)]
pub struct Key {
    /// The YAML token this key came from.
    pub token: String,
    pub face: KeyFace,
    pub action: KeyAction,
    /// The key's outline width (logical units).
    pub width: f32,
    /// Colours per [`super::style::KeyState`], from the button's `key_style:` (or
    /// `normal`) and the `latched` style.
    pub look: KeyLook,
    /// Corner label (lowercase data; shift is applied at use).
    pub hint: Option<String>,
    /// Long-press choices, first is the default highlight. Lowercase.
    pub choices: Vec<String>,
}

impl Row {
    pub(super) fn keys_mut(&mut self) -> impl Iterator<Item = &mut Key> {
        self.cells.iter_mut().filter_map(|c| match c {
            Cell::Key(k) => Some(k),
            Cell::Spacer(_) => None,
        })
    }
}

#[cfg(test)]
impl Key {
    /// A bare text key for `token`, for tests.
    pub(super) fn test(token: &str) -> Self {
        Key {
            token: token.to_string(),
            face: KeyFace::Text(token.to_string()),
            action: KeyAction::EmitText(token.to_string()),
            width: 1.0,
            look: KeyLook::default(),
            hint: None,
            choices: Vec::new(),
        }
    }
}

impl View {
    /// Every key in the view, flattened across its rows.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.rows.iter().flat_map(|row| row.keys())
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
