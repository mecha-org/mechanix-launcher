//! Spec to IR: turns the raw [`super::spec`] structs into a [`Keymap`].

use std::collections::HashMap;
use tracing::warn;
use xkbcommon::xkb::{self, Keysym};

use super::keymap::{Key, KeyAction, KeyFace, Modifier, Row, View};
use super::spec::{ActionSpec, Button, Layout, Outline};
use super::style::{self, KeyLook, KeyStyle};
use super::{KeyboardStyle, Keymap, PredictionConfig};

/// Size used when a button names an outline that isn't defined (and no
/// `default` outline exists to fall back to). Keeps conversion total.
const FALLBACK_OUTLINE: Outline = Outline { width: 50.0 };

impl View {
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
    pub(super) fn from_layout(layout: &Layout) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::FALLBACK_LAYOUT;
    use crate::layout::spec::LockingSpec;

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
}
