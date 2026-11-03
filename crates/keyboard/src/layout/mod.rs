//! The keymap: the resolved IR built from a squeekboard `layout.yaml`, plus the
//! loading and resolution that turns the raw [`spec`] structs into [`Key`]s.

pub(crate) mod icons;
mod spec;
pub(crate) mod style;
pub(crate) mod tokens;

use mecha_wayland::prelude::{ColorRole, ThemeMode};
use std::collections::HashMap;
use std::{env, fs};
use tracing::{info, warn};
use xkbcommon::xkb::{self, Keysym};

use spec::{ActionSpec, Button, KeyboardSpec, Layout, Outline, PredictionSpec};
use style::{KeyLook, KeyStyle};
use tokens::Font;

/// Size used when a button names an outline that isn't defined (and no
/// `default` outline exists to fall back to). Keeps conversion total.
const FALLBACK_OUTLINE: Outline = Outline { width: 50.0 };

/// The resolved keyboard: its views and the container style.
#[derive(Debug)]
pub struct Keymap {
    pub views: Vec<View>,
    pub keyboard: KeyboardStyle,
    pub prediction: PredictionConfig,
}

/// Default predictive-touch bias strength.
pub const DEFAULT_STRENGTH: f32 = 0.25;

/// Resolved `prediction:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionConfig {
    /// `false` = plain geometry for every key.
    pub enabled: bool,
    /// 0 = no bias; higher = stronger lean toward likely next letters.
    pub strength: f32,
    /// Bundled list name, or a path to a `word<TAB>count` file.
    pub dictionary: String,
    /// Show a pop-up of the pressed character key.
    pub popup: bool,
}

impl Default for PredictionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: DEFAULT_STRENGTH,
            dictionary: "en".to_string(),
            popup: true,
        }
    }
}

impl PredictionConfig {
    /// Resolve the raw `prediction:` block; a negative or non-finite strength
    /// warns and keeps the default.
    fn resolve(spec: &PredictionSpec) -> Self {
        let default = Self::default();
        let strength = match spec.strength {
            None => default.strength,
            Some(s) if s.is_finite() && s >= 0.0 => s,
            Some(s) => {
                warn!("prediction strength {s} is not a non-negative number; using the default");
                default.strength
            }
        };
        Self {
            enabled: spec.enabled.unwrap_or(default.enabled),
            strength,
            dictionary: spec.dictionary.clone().unwrap_or(default.dictionary),
            popup: spec.popup.unwrap_or(default.popup),
        }
    }
}

/// Style applied to the keyboard container.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardStyle {
    pub mode: ThemeMode,
    pub background_color: ColorRole,
    /// Key corner radius in logical px, resolved from a `Shape` token.
    pub radius: f32,
    /// Spacing in logical px, each resolved from a `Spacing` token.
    pub gap: f32,
    pub row_gap: f32,
    pub padding: f32,
    /// Window width in logical px; `None` spans the whole output.
    pub width: Option<f32>,
    /// Window height in logical px.
    pub height: f32,
    pub font: Font,
    pub font_size: u16,
}

impl Default for KeyboardStyle {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            background_color: ColorRole::Surface,
            radius: 6.0,
            gap: 4.0,
            row_gap: 4.0,
            padding: 4.0,
            width: None,
            height: 280.0,
            font: Font::Geist,
            font_size: 16,
        }
    }
}

/// Keep a size only if it's greater than zero, warning otherwise. Works for
/// any numeric field (`f32` dimensions, `u16` font size); `NaN` is rejected
/// since it doesn't compare greater than zero.
fn positive<T>(field: &str, value: Option<T>) -> Option<T>
where
    T: Copy + Default + PartialOrd + std::fmt::Display,
{
    value.filter(|&v| {
        let ok = v > T::default();
        if !ok {
            warn!("keyboard {field} {v} must be greater than 0; using the default");
        }
        ok
    })
}

/// Resolve an optional `Spacing` token to px, warning and keeping `default`
/// for a value that isn't a token.
fn resolve_spacing(field: &str, value: Option<f32>, default: f32) -> f32 {
    match value {
        None => default,
        Some(px) => match tokens::spacing(px) {
            Some(token) => token.dp(),
            None => {
                warn!("keyboard {field} {px} is not a spacing token; using {default}");
                default
            }
        },
    }
}

impl KeyboardStyle {
    /// Resolve the raw `keyboard:` block; an unrecognised value warns and
    /// keeps its default.
    fn resolve(spec: &KeyboardSpec) -> Self {
        let default = Self::default();
        let mode = match spec.mode.as_deref() {
            None => default.mode,
            Some("dark") => ThemeMode::Dark,
            Some("light") => ThemeMode::Light,
            Some(other) => {
                warn!(
                    "keyboard mode {other:?} is unknown (expected `dark` or `light`); using dark"
                );
                default.mode
            }
        };
        let background_color = match spec.background.as_deref() {
            None => default.background_color,
            Some(name) => tokens::color_role(name).unwrap_or_else(|| {
                warn!("keyboard background {name:?} is not a colour role; using `surface`");
                default.background_color
            }),
        };
        let radius = match spec.radius.as_deref() {
            None => default.radius,
            Some(name) => match tokens::shape(name) {
                Some(shape) => shape.resolve_radius_dp(0.0),
                None => {
                    warn!("keyboard radius {name:?} is not a shape token; using the default");
                    default.radius
                }
            },
        };
        let font = match spec.font.as_deref() {
            None => default.font,
            Some(name) => Font::from_name(name).unwrap_or_else(|| {
                warn!("keyboard font {name:?} is not a bundled font; using the default");
                default.font
            }),
        };
        Self {
            mode,
            background_color,
            radius,
            gap: resolve_spacing("gap", spec.gap, default.gap),
            row_gap: resolve_spacing("row-gap", spec.row_gap, default.row_gap),
            padding: resolve_spacing("padding", spec.padding, default.padding),
            width: positive("width", spec.width),
            height: positive("height", spec.height).unwrap_or(default.height),
            font,
            font_size: positive("font-size", spec.font_size).unwrap_or(default.font_size),
        }
    }
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

    fn from_name(name: &str) -> Option<Self> {
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
    /// Colours per [`style::KeyState`], from the button's `key_style:` (or
    /// `normal`) and the `latched` style.
    pub look: KeyLook,
}

impl View {
    /// Every key in the view, flattened across its rows.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.rows.iter().flat_map(|row| row.keys.iter())
    }

    /// Resolve one view's rows into keys. Positioning is left to the flex
    /// layout; each key carries only its outline width.
    fn resolve(
        layout: &Layout,
        styles: &HashMap<String, KeyStyle>,
        name: &str,
        rows: &[String],
    ) -> View {
        let out_rows = rows
            .iter()
            .map(|row| {
                let keys = row
                    .split_whitespace()
                    .map(|token| {
                        let button = layout.buttons.get(token);
                        let action = resolve_action(token, button);
                        let look = KeyLook::for_button(
                            styles,
                            token,
                            button.and_then(|b| b.key_style.as_deref()),
                            types_digit(&action),
                        );
                        Key {
                            face: resolve_face(token, button),
                            action,
                            width: resolve_outline(layout, button).width,
                            look,
                        }
                    })
                    .collect();
                Row { keys }
            })
            .collect();

        View {
            name: name.to_string(),
            rows: out_rows,
        }
    }
}

impl Keymap {
    /// Convert a parsed squeekboard layout into the IR, resolving each key's
    /// face, action, width and look.
    fn from_layout(layout: &Layout) -> Self {
        let styles = style::resolve_all(&layout.key_styles);
        let views = layout
            .views
            .iter()
            .map(|(name, rows)| View::resolve(layout, &styles, name, rows))
            .collect();
        let keyboard = KeyboardStyle::resolve(&layout.keyboard);
        let prediction = PredictionConfig::resolve(&layout.prediction);
        Keymap {
            views,
            keyboard,
            prediction,
        }
    }

    #[cfg(test)]
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.views.iter().flat_map(|view| view.keys())
    }

    #[cfg(test)]
    pub fn view(&self, name: &str) -> Option<&View> {
        self.views.iter().find(|v| v.name == name)
    }
}

/// Resolve a button token's face. Icon wins over label when both are set
/// (matching squeekboard) — and warns. With neither icon nor label, the
/// button's own name is the label.
fn resolve_face(token: &str, button: Option<&Button>) -> KeyFace {
    if let Some(icon) = button.and_then(|b| b.icon.as_deref()) {
        if button.and_then(|b| b.label.as_deref()).is_some() {
            warn!("button {token:?} sets both `icon` and `label`; using icon {icon:?}");
        }
        return KeyFace::Icon(icon.to_string());
    }
    let label = button
        .and_then(|b| b.label.clone())
        .unwrap_or_else(|| token.to_string());
    KeyFace::Text(label)
}

fn types_digit(action: &KeyAction) -> bool {
    let text = match action {
        KeyAction::EmitKeysym(ks) => char::from_u32(xkb::keysym_to_utf32(*ks)),
        KeyAction::EmitText(text) => {
            let mut chars = text.chars();
            chars.next().filter(|_| chars.next().is_none())
        }
        KeyAction::SetView(_)
        | KeyAction::ToggleView { .. }
        | KeyAction::LatchModifier(_)
        | KeyAction::HideKeyboard
        | KeyAction::ToggleExclusiveZone
        | KeyAction::Unhandled(_) => None,
    };
    text.is_some_and(|c| c.is_ascii_digit())
}

/// Resolve a button token's Key action — what it emits when tapped. Priority:
/// explicit `keysym:` → `action: erase` (→ BackSpace) → `text:` → a single-char
/// token/label (→ its keysym). A `modifier`, a structured/other `action`, or a
/// multi-char label with no keysym all resolve to `Unhandled` this pass.
fn resolve_action(token: &str, button: Option<&Button>) -> KeyAction {
    if let Some(b) = button {
        if let Some(name) = b.keysym.as_deref() {
            let ks = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
            if ks.raw() != 0 {
                return KeyAction::EmitKeysym(ks);
            }
            warn!("keysym {name:?} on button {token:?} is unknown; key won't type");
            return KeyAction::Unhandled(token.to_string());
        }
        match &b.action {
            Some(ActionSpec::Named(a)) if a == "erase" => {
                return KeyAction::EmitKeysym(xkb::keysym_from_name(
                    "BackSpace",
                    xkb::KEYSYM_NO_FLAGS,
                ));
            }
            Some(ActionSpec::Named(a)) if a == "hide_keyboard" => return KeyAction::HideKeyboard,
            Some(ActionSpec::Named(a)) if a == "toggle_exclusive_zone" => {
                return KeyAction::ToggleExclusiveZone;
            }
            Some(ActionSpec::Named(a)) => return KeyAction::Unhandled(a.clone()),
            Some(ActionSpec::SetView { set_view }) => {
                return KeyAction::SetView(set_view.clone());
            }
            Some(ActionSpec::Locking { locking }) => {
                return KeyAction::ToggleView {
                    lock: locking.lock_view.clone(),
                    unlock: locking.unlock_view.clone(),
                };
            }
            Some(ActionSpec::Other(_)) => return KeyAction::Unhandled(token.to_string()),
            None => {}
        }
        // A modifier button latches for the next keystroke. Only Control and Alt
        // are wired; every other modifier still defers to `Unhandled`.
        if let Some(m) = b.modifier.as_deref() {
            return match Modifier::from_name(m) {
                Some(m) => KeyAction::LatchModifier(m),
                None => KeyAction::Unhandled(token.to_string()),
            };
        }
        if let Some(text) = &b.text {
            return KeyAction::EmitText(text.clone());
        }
        if let Some(ks) = b.label.as_deref().and_then(single_char_keysym) {
            return KeyAction::EmitKeysym(ks);
        }
    }
    // No button entry (or nothing above matched): a single-char token is itself
    // the keysym — this covers every plain letter/digit/punctuation key.
    if let Some(ks) = single_char_keysym(token) {
        return KeyAction::EmitKeysym(ks);
    }
    KeyAction::Unhandled(token.to_string())
}

/// The keysym for a single-character string, or `None` if `s` isn't exactly one
/// char or that char has no keysym.
fn single_char_keysym(s: &str) -> Option<Keysym> {
    let mut chars = s.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let ks = Keysym::from_char(c);
    (ks.raw() != 0).then_some(ks)
}

/// Resolve a button's outline size: the button's named outline, else the
/// `default` outline, else a hard fallback. Warns on a dangling outline name.
fn resolve_outline(layout: &Layout, button: Option<&Button>) -> Outline {
    let name = button
        .and_then(|b| b.outline.as_deref())
        .unwrap_or("default");
    if let Some(outline) = layout.outlines.get(name) {
        return *outline;
    }
    warn!("outline {name:?} not defined; falling back to `default`");
    layout
        .outlines
        .get("default")
        .copied()
        .unwrap_or(FALLBACK_OUTLINE)
}

static FALLBACK_LAYOUT: &str = include_str!("../../resources/layout.yaml");

/// Load and resolve the layout on startup. Checks `MECHA_KBD_LAYOUT` env var,
/// then `layout.yaml` in the working directory, then the bundled fallback.
pub fn load_keymap() -> Keymap {
    let contents = match env::var("MECHA_KBD_LAYOUT") {
        Ok(path) => fs::read_to_string(path).expect("Error: Failed to read layout file"),
        Err(_) => {
            let local = std::path::Path::new("layout.yaml");
            if local.is_file() {
                info!("Using layout.yaml from working directory");
                fs::read_to_string(local).expect("Error: Failed to read layout file")
            } else {
                warn!("Warning: No config path provided! Using fallback...");
                FALLBACK_LAYOUT.into()
            }
        }
    };

    let layout: Layout = yaml_serde::from_str(&contents).expect("Error: Failed to parse yaml");
    let keymap = Keymap::from_layout(&layout);

    info!("loaded keymap: {} view(s)", keymap.views.len());
    for view in &keymap.views {
        info!("  view {}: {} key(s)", view.name, view.keys().count());
    }

    keymap
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec::LockingSpec;

    #[test]
    fn positive_keeps_only_sizes_above_zero() {
        assert_eq!(positive("height", Some(280.0)), Some(280.0));
        assert_eq!(positive("height", Some(0.0)), None);
        assert_eq!(positive("height", Some(-1.0)), None);
        assert_eq!(positive("height", Some(f32::NAN)), None);
        assert_eq!(positive::<f32>("height", None), None);
        assert_eq!(positive("font-size", Some(16u16)), Some(16));
        assert_eq!(positive("font-size", Some(0u16)), None);
    }

    /// A `Button` with only the fields a test cares about set.
    fn button(f: impl FnOnce(&mut Button)) -> Button {
        let mut b = Button::default();
        f(&mut b);
        b
    }

    #[test]
    fn plain_char_token_resolves_to_its_keysym() {
        for tok in ["q", "1", ",", "."] {
            let c = tok.chars().next().unwrap();
            assert!(
                matches!(resolve_action(tok, None), KeyAction::EmitKeysym(ks) if ks == Keysym::from_char(c)),
                "token {tok:?} should emit its own keysym",
            );
        }
    }

    #[test]
    fn explicit_keysym_field_wins() {
        let b = button(|b| b.keysym = Some("Return".into()));
        let want = xkb::keysym_from_name("Return", xkb::KEYSYM_NO_FLAGS);
        assert!(
            matches!(resolve_action("Return", Some(&b)), KeyAction::EmitKeysym(ks) if ks == want)
        )
    }

    #[test]
    fn keyboard_actions_resolve_by_name() {
        let named = |n: &str| button(|b| b.action = Some(ActionSpec::Named(n.into())));
        assert!(matches!(
            resolve_action("hide", Some(&named("hide_keyboard"))),
            KeyAction::HideKeyboard
        ));
        assert!(matches!(
            resolve_action("zone", Some(&named("toggle_exclusive_zone"))),
            KeyAction::ToggleExclusiveZone
        ));
    }

    #[test]
    fn erase_action_maps_to_backspace() {
        let b = button(|b| b.action = Some(ActionSpec::Named("erase".into())));
        let want = xkb::keysym_from_name("BackSpace", xkb::KEYSYM_NO_FLAGS);
        assert!(
            matches!(resolve_action("BackSpace", Some(&b)), KeyAction::EmitKeysym(ks) if ks == want)
        )
    }

    #[test]
    fn text_field_resolves_to_emit_text() {
        let b = button(|b| b.text = Some(" ".into()));
        assert!(matches!(resolve_action("space", Some(&b)), KeyAction::EmitText(t) if t == " "))
    }

    #[test]
    fn set_view_action_resolves_to_setview() {
        let b = button(|b| {
            b.action = Some(ActionSpec::SetView {
                set_view: "symbols".into(),
            })
        });
        assert!(matches!(
            resolve_action("show_symbols", Some(&b)),
            KeyAction::SetView(v) if v == "symbols"
        ))
    }

    #[test]
    fn locking_action_resolves_to_toggleview() {
        let b = button(|b| {
            b.action = Some(ActionSpec::Locking {
                locking: LockingSpec {
                    lock_view: "upper".into(),
                    unlock_view: "base".into(),
                },
            })
        });
        assert!(matches!(
            resolve_action("Shift_L", Some(&b)),
            KeyAction::ToggleView { lock, unlock } if lock == "upper" && unlock == "base"
        ))
    }

    #[test]
    fn deferred_actions_are_unhandled() {
        let unknown = button(|b| b.action = Some(ActionSpec::Other(serde::de::IgnoredAny)));
        assert!(matches!(
            resolve_action("mystery", Some(&unknown)),
            KeyAction::Unhandled(_)
        ));

        let prefs = button(|b| b.action = Some(ActionSpec::Named("show_prefs".into())));
        assert!(matches!(
            resolve_action("preferences", Some(&prefs)),
            KeyAction::Unhandled(_)
        ));

        // Super is a modifier, but not wired to latch this pass — still deferred.
        let sup = button(|b| b.modifier = Some("Super".into()));
        assert!(matches!(
            resolve_action("Super", Some(&sup)),
            KeyAction::Unhandled(_)
        ));
    }

    #[test]
    fn control_modifier_latches() {
        let ctrl = button(|b| b.modifier = Some("Control".into()));
        assert!(matches!(
            resolve_action("Ctrl", Some(&ctrl)),
            KeyAction::LatchModifier(Modifier::Control)
        ))
    }

    #[test]
    fn alt_modifier_latches() {
        let alt = button(|b| b.modifier = Some("Alt".into()));
        assert!(matches!(
            resolve_action("Alt", Some(&alt)),
            KeyAction::LatchModifier(Modifier::Alt)
        ))
    }

    #[test]
    fn fallback_bottom_rows_have_hide_and_zone_keys() {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("fallback parses");
        let keymap = Keymap::from_layout(&layout);
        for name in ["base", "upper", "numbers", "symbols"] {
            let view = keymap.view(name).expect("view exists");
            assert!(
                view.keys()
                    .any(|k| matches!(k.action, KeyAction::HideKeyboard)),
                "{name} has a hide key"
            );
            assert!(
                view.keys()
                    .any(|k| matches!(k.action, KeyAction::ToggleExclusiveZone)),
                "{name} has an exclusive-zone key"
            );
        }
    }

    #[test]
    fn fallback_structured_actions_resolve() {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("fallback parses");
        let keymap = Keymap::from_layout(&layout);

        let has_set_view = keymap
            .keys()
            .any(|k| matches!(&k.action, KeyAction::SetView(v) if v == "symbols"));
        assert!(has_set_view, "a `set_view: symbols` key should be SetView");

        let has_toggle = keymap.keys().any(|k| {
            matches!(&k.action, KeyAction::ToggleView { lock, unlock } if lock == "upper" && unlock == "base")
        });
        assert!(has_toggle, "Shift_L should be ToggleView upper/base");
    }

    #[test]
    fn fallback_layout_base_view_has_expected_actions() {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("fallback parses");
        let keymap = Keymap::from_layout(&layout);
        let base = keymap.view("base").expect("base view exists");

        // Every base-view key resolves to *some* action, and the ones that type
        // are keysym/text — no base key is left ambiguous.
        let typeable = base
            .keys()
            .filter(|k| matches!(k.action, KeyAction::EmitKeysym(_) | KeyAction::EmitText(_)))
            .count();
        // 26 letters plus `. space ⌫ DONE`; `,` lives in the numbers view.
        assert!(
            typeable >= 30,
            "base view should have many typeable keys, got {typeable}"
        );
    }

    const MINIMAL_YAML: &str =
        "outlines:\n  default: { width: 50 }\nviews:\n  base:\n    - \"a\"\n";

    #[test]
    fn prediction_block_defaults_when_missing() {
        let km = Keymap::from_layout(&yaml_serde::from_str::<Layout>(MINIMAL_YAML).unwrap());
        assert!(km.prediction.enabled && km.prediction.popup);
        assert_eq!(km.prediction.strength, DEFAULT_STRENGTH);
        assert_eq!(km.prediction.dictionary, "en");
    }

    #[test]
    fn prediction_block_parses_and_rejects_negative_strength() {
        let yaml = format!(
            "{MINIMAL_YAML}prediction:\n  enabled: false\n  strength: -2\n  dictionary: /tmp/x.tsv\n  popup: false\n"
        );
        let km = Keymap::from_layout(&yaml_serde::from_str::<Layout>(&yaml).unwrap());
        assert!(!km.prediction.enabled);
        assert_eq!(km.prediction.strength, DEFAULT_STRENGTH);
        assert_eq!(km.prediction.dictionary, "/tmp/x.tsv");
        assert!(!km.prediction.popup);
    }

    #[test]
    fn prediction_block_keeps_valid_strength() {
        let yaml = format!("{MINIMAL_YAML}prediction:\n  strength: 0\n");
        let km = Keymap::from_layout(&yaml_serde::from_str::<Layout>(&yaml).unwrap());
        assert_eq!(km.prediction.strength, 0.0);
    }

    #[test]
    fn bundled_layout_has_prediction_block() {
        assert!(load_keymap().prediction.enabled);
    }
}
