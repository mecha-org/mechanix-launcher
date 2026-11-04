//! Spec to IR: turns the raw [`super::spec`] structs into a [`Keymap`].

use std::collections::HashMap;
use tracing::warn;
use xkbcommon::xkb::{self, Keysym};

use super::keymap::{Cell, Family, Key, KeyAction, KeyFace, Modifier, Page, Row, View};
use super::more_keys::{self, MoreKeys};
use super::spec::{ActionSpec, Button, Layout, Outline, RowSpec, ViewSpec};
use super::style::{self, KeyLook, KeyStyle};
use super::{KeyboardStyle, Keymap, PredictionConfig};

/// Size used when a button names an outline that isn't defined (and no
/// `default` outline exists to fall back to). Keeps conversion total.
const FALLBACK_OUTLINE: Outline = Outline { width: 50.0 };

/// A row after `extends`/`replace` expansion: tokens still unresolved.
struct RawRow {
    height: f32,
    keys: String,
}

impl RowSpec {
    fn raw(&self) -> RawRow {
        match self {
            RowSpec::Keys(keys) => RawRow {
                height: 1.0,
                keys: keys.clone(),
            },
            RowSpec::Sized { height, keys } => RawRow {
                height: height.unwrap_or(1.0),
                keys: keys.clone(),
            },
        }
    }
}

/// The rows of view `name` with `extends` expanded. A view's own `rows` come
/// first, then the extended view's rows; `replace` is applied to all of them.
/// An unknown target or a cycle warns and contributes nothing.
fn expand_rows(layout: &Layout, name: &str, visiting: &mut Vec<String>) -> Vec<RawRow> {
    if visiting.iter().any(|v| v == name) {
        warn!("view {name:?} extends itself through a cycle; stopping");
        return Vec::new();
    }
    let Some(spec) = layout.views.get(name) else {
        warn!("view {name:?} is not defined");
        return Vec::new();
    };
    let map = match spec {
        ViewSpec::Rows(rows) => return rows.iter().map(RowSpec::raw).collect(),
        ViewSpec::Map(map) => map,
    };
    let mut out: Vec<RawRow> = map.rows.iter().flatten().map(RowSpec::raw).collect();
    match &map.extends {
        Some(target) => {
            visiting.push(name.to_string());
            out.extend(expand_rows(layout, target, visiting));
            visiting.pop();
        }
        None if map.rows.is_none() => {
            warn!("view {name:?} has neither `rows` nor `extends`; it is empty");
        }
        None => {}
    }
    if !map.replace.is_empty() {
        for row in &mut out {
            row.keys = row
                .keys
                .split_whitespace()
                .map(|t| map.replace.get(t).map_or(t, String::as_str))
                .collect::<Vec<_>>()
                .join(" ");
        }
    }
    out
}

/// `[N]` with N a positive number is a spacer of width N; anything else is not.
fn spacer_width(token: &str) -> Option<f32> {
    let inner = token.strip_prefix('[')?.strip_suffix(']')?;
    inner
        .parse::<f32>()
        .ok()
        .filter(|w| w.is_finite() && *w > 0.0)
}

impl View {
    /// Resolve one view's rows into cells. Positioning is left to the flex
    /// layout; each key carries only its outline width.
    fn resolve(
        layout: &Layout,
        styles: &HashMap<String, KeyStyle>,
        name: &str,
        spec: &ViewSpec,
    ) -> View {
        let (family, page) = match spec {
            ViewSpec::Rows(_) => (None, None),
            ViewSpec::Map(m) => (m.family.as_deref(), m.page.as_deref()),
        };
        let family = family.map_or(Family::Text, |f| {
            Family::from_name(f).unwrap_or_else(|| {
                warn!("view {name:?}: unknown family {f:?}; using text");
                Family::Text
            })
        });
        let page = match page {
            Some(p) => Page::from_name(p).unwrap_or_else(|| {
                warn!("view {name:?}: unknown page {p:?}; using letters");
                Page::Letters
            }),
            None => Page::from_name(name).unwrap_or(Page::Letters),
        };

        let out_rows = expand_rows(layout, name, &mut Vec::new())
            .into_iter()
            .map(|row| {
                let cells = row
                    .keys
                    .split_whitespace()
                    .map(|token| match spacer_width(token) {
                        Some(w) => Cell::Spacer(w),
                        None => Cell::Key(resolve_key(layout, styles, token)),
                    })
                    .collect();
                Row {
                    cells,
                    height: row.height,
                }
            })
            .collect();

        let mut view = View {
            name: name.to_string(),
            family,
            page,
            rows: out_rows,
        };
        let set_name = match spec {
            ViewSpec::Rows(_) => None,
            ViewSpec::Map(m) => m.more_keys.as_deref(),
        };
        if let Some(set_name) = set_name {
            match MoreKeys::bundled().set(set_name) {
                Some(set) => more_keys::attach(&mut view, set),
                None => warn!("view {name:?}: more_keys set {set_name:?} is not defined"),
            }
        }
        view
    }
}

fn resolve_key(layout: &Layout, styles: &HashMap<String, KeyStyle>, token: &str) -> Key {
    let button = layout.buttons.get(token);
    let action = resolve_action(token, button);
    let look = KeyLook::for_button(
        styles,
        token,
        button.and_then(|b| b.key_style.as_deref()),
        types_digit(&action),
    );
    Key {
        token: token.to_string(),
        face: resolve_face(token, button),
        action,
        width: resolve_outline(layout, button).width,
        look,
        hint: None,
        choices: Vec::new(),
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
            .map(|(name, spec)| View::resolve(layout, &styles, name, spec))
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
        | KeyAction::LatchModifier(_)
        | KeyAction::Page(_)
        | KeyAction::Shift
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
            Some(ActionSpec::Named(a)) if a == "shift" => return KeyAction::Shift,
            Some(ActionSpec::Page { page }) => match Page::from_name(page) {
                Some(p) => return KeyAction::Page(p),
                None => {
                    warn!("button {token:?}: unknown page {page:?}");
                    return KeyAction::Unhandled(token.to_string());
                }
            },
            Some(ActionSpec::Named(a)) => return KeyAction::Unhandled(a.clone()),
            Some(ActionSpec::SetView { set_view }) => {
                return KeyAction::SetView(set_view.clone());
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

    /// A `Button` with only the fields a test cares about set.
    fn button(f: impl FnOnce(&mut Button)) -> Button {
        let mut b = Button::default();
        f(&mut b);
        b
    }

    fn keymap(yaml: &str) -> Keymap {
        let layout: Layout = yaml_serde::from_str(yaml).expect("parses");
        Keymap::from_layout(&layout)
    }

    fn tokens(row: &Row) -> Vec<&str> {
        row.keys().map(|k| k.token.as_str()).collect()
    }

    const HEAD: &str = "outlines: { default: { width: 10 } }\n";

    #[test]
    fn spacer_token_is_a_spacer_cell() {
        let km = keymap(&format!("{HEAD}views: {{ v: [\"[5] a s [5]\"] }}"));
        let cells = &km.view("v").unwrap().rows[0].cells;
        assert!(matches!(cells[0], Cell::Spacer(w) if w == 5.0));
        assert!(matches!(&cells[1], Cell::Key(k) if k.token == "a"));
        assert!(matches!(&cells[2], Cell::Key(k) if k.token == "s"));
        assert!(matches!(cells[3], Cell::Spacer(w) if w == 5.0));
    }

    #[test]
    fn bracket_tokens_that_are_not_numbers_are_keys() {
        for t in ["[", "[]", "[x]", "[0]"] {
            assert!(spacer_width(t).is_none(), "{t}");
        }
        assert_eq!(spacer_width("[2.5]"), Some(2.5));
    }

    #[test]
    fn row_map_sets_height() {
        let km = keymap(&format!(
            "{HEAD}views: {{ v: [ {{ height: 0.75, keys: Esc }} ] }}"
        ));
        assert_eq!(km.view("v").unwrap().rows[0].height, 0.75);
    }

    #[test]
    fn plain_row_height_is_one() {
        let km = keymap(&format!("{HEAD}views: {{ v: [\"a b\"] }}"));
        assert_eq!(km.view("v").unwrap().rows[0].height, 1.0);
    }

    const EXT: &str = "views:
  base: [\"q w\", \"?123 comma emoji space period Return\"]
  email: { family: email, extends: base, replace: { comma: at, emoji: dot_com } }
";

    #[test]
    fn extends_copies_rows_and_replace_swaps_tokens() {
        let km = keymap(&format!("{HEAD}{EXT}"));
        let email = km.view("email").unwrap();
        assert_eq!(email.rows.len(), 2);
        assert_eq!(
            tokens(&email.rows[1]),
            ["?123", "at", "dot_com", "space", "period", "Return"]
        );
        // The base view is untouched.
        assert_eq!(tokens(&km.view("base").unwrap().rows[1])[1], "comma");
    }

    #[test]
    fn replace_applies_to_every_occurrence() {
        let km = keymap(&format!(
            "{HEAD}views:\n  b: [\"x y x\", \"x\"]\n  c: {{ extends: b, replace: {{ x: z }} }}\n"
        ));
        let c = km.view("c").unwrap();
        assert_eq!(tokens(&c.rows[0]), ["z", "y", "z"]);
        assert_eq!(tokens(&c.rows[1]), ["z"]);
    }

    #[test]
    fn extends_unknown_view_warns_and_yields_empty_view() {
        let km = keymap(&format!("{HEAD}views: {{ v: {{ extends: nope }} }}"));
        assert!(km.view("v").unwrap().rows.is_empty());
    }

    #[test]
    fn extends_cycle_and_empty_map_do_not_panic() {
        let km = keymap(&format!(
            "{HEAD}views: {{ a: {{ extends: b }}, b: {{ extends: a }}, e: {{ family: text }} }}"
        ));
        assert!(km.view("a").unwrap().rows.is_empty());
        assert!(km.view("e").unwrap().rows.is_empty());
    }

    #[test]
    fn extends_chain_expands_target_first() {
        let km = keymap(&format!(
            "{HEAD}views:\n  a: [\"x\"]\n  b: {{ extends: a, replace: {{ x: y }} }}\n  c: {{ extends: b, replace: {{ y: z }} }}\n"
        ));
        assert_eq!(tokens(&km.view("c").unwrap().rows[0]), ["z"]);
    }

    #[test]
    fn view_map_sets_family_and_page() {
        let km = keymap(&format!(
            "{HEAD}views: {{ v: {{ family: email, page: symbols, rows: [\"a\"] }} }}"
        ));
        let v = km.view("v").unwrap();
        assert_eq!(v.family, Family::Email);
        assert_eq!(v.page, Page::Symbols);
    }

    #[test]
    fn list_view_defaults_to_text_family() {
        let km = keymap(&format!(
            "{HEAD}views: {{ symbols: [\"a\"], other: [\"a\"] }}"
        ));
        let s = km.view("symbols").unwrap();
        assert_eq!((s.family, s.page), (Family::Text, Page::Symbols));
        let o = km.view("other").unwrap();
        assert_eq!((o.family, o.page), (Family::Text, Page::Letters));
    }

    #[test]
    fn page_action_resolves() {
        let b = button(|b| {
            b.action = Some(ActionSpec::Page {
                page: "more_symbols".into(),
            })
        });
        assert!(matches!(
            resolve_action("x", Some(&b)),
            KeyAction::Page(Page::MoreSymbols)
        ));
    }

    #[test]
    fn inline_layout_with_shift_action_drives_shift_state() {
        use crate::modifiers::shift::{ShiftState, cased, shiftable_letter};
        let km = keymap(&format!(
            "{HEAD}views: {{ v: [\"q 1 S\"] }}\nbuttons:\n  S:\n    action: shift\n    icon: key-shift-symbolic\n"
        ));
        let keys: Vec<_> = km.view("v").unwrap().keys().collect();
        assert!(matches!(keys[2].action, KeyAction::Shift));
        // Letters follow shift; digits and the shift key do not.
        assert_eq!(shiftable_letter(&keys[0].action), Some('q'));
        assert_eq!(shiftable_letter(&keys[1].action), None);
        assert_eq!(shiftable_letter(&keys[2].action), None);
        assert_eq!(cased('q', ShiftState::Off), 'q');
        assert_eq!(cased('q', ShiftState::OneShot), 'Q');
        assert_eq!(cased('q', ShiftState::Locked), 'Q');
    }

    #[test]
    fn shift_action_resolves() {
        let b = button(|b| b.action = Some(ActionSpec::Named("shift".into())));
        assert!(matches!(resolve_action("x", Some(&b)), KeyAction::Shift));
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
    fn views_get_long_press_data_by_set() {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("fallback parses");
        let keymap = Keymap::from_layout(&layout);
        let find = |view: &str, token: &str| {
            keymap
                .view(view)
                .unwrap()
                .keys()
                .find(|k| k.token == token)
                .unwrap()
                .clone()
        };
        assert_eq!(find("base", "q").hint.as_deref(), Some("1"));
        assert_eq!(find("base", "e").choices.len(), 6);
        assert_eq!(find("symbols", "$").choices.len(), 5);

        let yaml = "outlines: { default: { width: 10 } }\nviews:\n  v:\n    family: text\n    page: letters\n    rows: [\"q\"]\n  w:\n    more_keys: terminal\n    rows: [\"q\"]\n";
        let layout: Layout = yaml_serde::from_str(yaml).unwrap();
        let km = Keymap::from_layout(&layout);
        assert_eq!(km.view("v").unwrap().keys().next().unwrap().hint, None);
        assert_eq!(
            km.view("w").unwrap().keys().next().unwrap().hint.as_deref(),
            Some("1")
        );
    }

    #[test]
    fn fallback_structured_actions_resolve() {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("fallback parses");
        let keymap = Keymap::from_layout(&layout);

        assert!(
            keymap
                .keys()
                .any(|k| matches!(k.action, KeyAction::Page(Page::Symbols))),
            "?123 should be Page(Symbols)"
        );

        assert!(
            keymap.keys().any(|k| matches!(k.action, KeyAction::Shift)),
            "a shift key should be KeyAction::Shift"
        );
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
        // 26 letters plus `, . space ⌫ ⏎`.
        assert!(
            typeable >= 30,
            "base view should have many typeable keys, got {typeable}"
        );
    }

    // ── the bundled AOSP layout ─────────────────────────────────────────

    fn bundled() -> Keymap {
        let layout: Layout = yaml_serde::from_str(FALLBACK_LAYOUT).expect("bundled parses");
        Keymap::from_layout(&layout)
    }

    fn is_keysym(action: &KeyAction, name: &str) -> bool {
        let want = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        matches!(action, KeyAction::EmitKeysym(ks) if *ks == want)
    }

    /// The rows of a view without the terminal strip (`height < 1`).
    fn key_rows(view: &View) -> Vec<&Row> {
        view.rows.iter().filter(|r| r.height >= 1.0).collect()
    }

    #[test]
    fn every_view_obeys_the_fixed_key_rule() {
        let km = bundled();
        assert!(km.views.len() >= 12, "all pages are present");
        for view in &km.views {
            let rows = key_rows(view);
            let third = rows
                .get(2)
                .unwrap_or_else(|| panic!("{} has a third row", view.name));
            let last = third.keys().last().unwrap();
            assert!(
                is_keysym(&last.action, "BackSpace"),
                "{}: third row ends in {:?}",
                view.name,
                last.token
            );
            let last = rows.last().unwrap().keys().last().unwrap();
            assert!(
                is_keysym(&last.action, "Return"),
                "{}: bottom row ends in {:?}",
                view.name,
                last.token
            );
        }
    }

    #[test]
    fn every_family_has_a_letters_page() {
        let km = bundled();
        for family in [
            Family::Text,
            Family::Email,
            Family::Url,
            Family::Password,
            Family::Number,
            Family::Phone,
            Family::Terminal,
        ] {
            assert!(
                km.views
                    .iter()
                    .any(|v| v.family == family && v.page == Page::Letters),
                "no letters page for {family:?}"
            );
        }
    }

    #[test]
    fn terminal_letters_have_no_digit_row_and_an_eight_key_strip() {
        let km = bundled();
        let v = km.view("terminal").unwrap();
        assert_eq!(v.rows[0].height, 0.75);
        assert_eq!(
            tokens(&v.rows[0]),
            ["Esc", "Tab", "Ctrl", "Alt", "←", "↓", "↑", "→"]
        );
        assert_eq!(tokens(&v.rows[1])[0], "q");
        assert!(
            v.keys()
                .all(|k| !matches!(k.token.as_str(), "1" | "2" | "8"))
        );
        // Page keys stay in the family and keep the strip.
        for name in ["terminal_symbols", "terminal_more_symbols"] {
            let v = km.view(name).unwrap();
            assert_eq!(v.family, Family::Terminal);
            assert_eq!(v.rows[0].height, 0.75, "{name} keeps the strip");
            assert_eq!(v.rows[0].keys().count(), 8);
        }
    }

    #[test]
    fn terminal_bottom_rows_match_the_spec() {
        let km = bundled();
        let bottom = |n: &str| {
            let v = km.view(n).unwrap();
            tokens(v.rows.last().unwrap())
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bottom("terminal"),
            ["to_symbols", "slash", "dot", "space", "minus", "Return"]
        );
        assert_eq!(
            bottom("terminal_symbols"),
            [
                "to_letters",
                "/",
                "_",
                "space_short",
                "|",
                "minus",
                "Return"
            ]
        );
        assert_eq!(
            bottom("terminal_more_symbols"),
            [
                "to_letters",
                "/",
                "<",
                "space_short",
                ">",
                "minus",
                "Return"
            ]
        );
    }

    #[test]
    fn email_bottom_row_is_at_dot_com() {
        let km = bundled();
        let v = km.view("email").unwrap();
        assert_eq!(
            tokens(v.rows.last().unwrap()),
            ["to_symbols", "at", "dot_com", "space", "period", "Return"]
        );
        let v = km.view("url").unwrap();
        assert_eq!(
            tokens(v.rows.last().unwrap()),
            [
                "to_symbols",
                "slash",
                "dot_com",
                "space",
                "period",
                "Return"
            ]
        );
        let v = km.view("password").unwrap();
        assert_eq!(
            tokens(v.rows.last().unwrap()),
            ["to_symbols", "comma", "at", "space", "period", "Return"]
        );
    }

    #[test]
    fn password_and_terminal_have_no_emoji_key() {
        let km = bundled();
        for name in [
            "password",
            "terminal",
            "terminal_symbols",
            "terminal_more_symbols",
        ] {
            assert!(
                km.view(name)
                    .unwrap()
                    .keys()
                    .all(|k| !matches!(k.action, KeyAction::Page(Page::Emoji))),
                "{name} has an emoji key"
            );
        }
        let base = km.view("base").unwrap();
        assert!(
            base.keys()
                .any(|k| matches!(k.action, KeyAction::Page(Page::Emoji)))
        );
    }

    #[test]
    fn main_quote_keys_are_plain() {
        let km = bundled();
        let sym = km.view("symbols").unwrap();
        for (ch, name) in [('"', "quotedbl"), ('\'', "apostrophe")] {
            let key = sym.keys().find(|k| k.token == ch.to_string()).unwrap();
            assert!(is_keysym(&key.action, name), "{ch} is its plain keysym");
        }
        for v in &km.views {
            assert!(
                v.keys().all(|k| k.token != "”" && k.token != "’"),
                "{} has a curly quote as a main key",
                v.name
            );
        }
    }

    #[test]
    fn base_row_widths_match_aosp() {
        let km = bundled();
        for name in [
            "base",
            "symbols",
            "more_symbols",
            "email",
            "url",
            "password",
            "terminal",
            "terminal_symbols",
            "terminal_more_symbols",
        ] {
            for (i, row) in km.view(name).unwrap().rows.iter().enumerate() {
                let sum: f32 = row
                    .cells
                    .iter()
                    .map(|c| match c {
                        Cell::Key(k) => k.width,
                        Cell::Spacer(w) => *w,
                    })
                    .sum();
                assert!((sum - 100.0).abs() < 0.01, "{name} row {i} sums to {sum}");
            }
        }
    }

    #[test]
    fn pages_reach_each_other_and_the_phone_pad_types_pause_and_wait() {
        let km = bundled();
        let find = |view: &str, token: &str| {
            km.view(view)
                .unwrap()
                .keys()
                .find(|k| k.token == token)
                .unwrap()
                .clone()
        };
        let page = |view: &str, token: &str| match find(view, token).action {
            KeyAction::Page(p) => p,
            a => panic!("{view}/{token}: {a:?}"),
        };
        assert_eq!(page("base", "to_symbols"), Page::Symbols);
        assert_eq!(page("symbols", "to_more_symbols"), Page::MoreSymbols);
        assert_eq!(page("more_symbols", "to_symbols"), Page::Symbols);
        assert_eq!(page("symbols", "to_letters"), Page::Letters);
        assert_eq!(page("phone", "to_phone_symbols"), Page::PhoneSymbols);
        assert_eq!(page("phone_symbols", "to_phone"), Page::Letters);
        assert!(
            matches!(find("phone_symbols", "pause").action, KeyAction::EmitText(t) if t == ",")
        );
        assert!(matches!(find("phone_symbols", "wait").action, KeyAction::EmitText(t) if t == ";"));
        // The period on the ?123 page is the literal `.` so `…` attaches.
        assert_eq!(find("symbols", ".").choices, ["…"]);
        assert_eq!(find("number", "minus").choices, ["+"]);
        assert!(find("phone", "2").choices.is_empty());
    }

    #[test]
    fn notice_file_is_bundled() {
        let notice = include_str!("../../resources/NOTICE-aosp.txt");
        assert!(notice.contains("AOSP LatinIME"));
        assert!(notice.contains("Apache License"));
    }
}
