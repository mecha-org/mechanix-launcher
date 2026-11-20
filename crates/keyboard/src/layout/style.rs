//! Named key styles from the `key_styles:` block of `layout.yaml`. Each style
//! sets a background and a foreground (label) colour role, and optionally the
//! label's font and size. A button picks one with `key_style:`. Without one, a
//! key that types a digit uses `number` and any other key uses `normal`. An
//! armed modifier uses `latched`.

use super::tokens::{self, Font};
use mecha_wayland::prelude::ColorRole;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::warn;

pub const NORMAL: &str = "normal";
pub const NUMBER: &str = "number";
pub const LATCHED: &str = "latched";
/// A key that switches to the view it's on (the shown tab).
pub const CURRENT: &str = "current";

/// One named style as it appears in `layout.yaml`.
#[derive(Debug, Default, Deserialize)]
pub struct KeyStyleSpec {
    background: Option<String>,
    foreground: Option<String>,
    font: Option<String>,
    #[serde(rename = "font-size")]
    font_size: Option<u16>,
}

/// A resolved style: unset fields fall back to the keyboard's defaults.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct KeyStyle {
    pub background: Option<ColorRole>,
    pub foreground: Option<ColorRole>,
    pub font: Option<Font>,
    pub font_size: Option<u16>,
}

/// Which state a key is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Normal,
    Latched,
}

impl KeyState {
    /// [`Latched`](Self::Latched) when `armed`, else [`Normal`](Self::Normal).
    pub fn latched_if(armed: bool) -> Self {
        if armed { Self::Latched } else { Self::Normal }
    }
}

/// Every state a key can be drawn with — one [`KeyStyle`] per [`KeyState`].
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct KeyLook {
    normal: KeyStyle,
    latched: KeyStyle,
}

impl KeyLook {
    /// Resolve a button's look: its normal style (named, else `number` for
    /// digit-typing keys, else `normal`) and the global `latched` style.
    pub fn for_button(
        styles: &HashMap<String, KeyStyle>,
        token: &str,
        name: Option<&str>,
        types_digit: bool,
    ) -> Self {
        Self {
            normal: for_button(styles, token, name, types_digit),
            latched: styles.get(LATCHED).copied().unwrap_or_default(),
        }
    }

    /// This look with `normal` as its normal style.
    pub fn with_normal(self, normal: KeyStyle) -> Self {
        Self { normal, ..self }
    }

    /// The style for a given state.
    pub fn style(&self, state: KeyState) -> KeyStyle {
        match state {
            KeyState::Normal => self.normal,
            KeyState::Latched => self.latched,
        }
    }
}

/// Resolve every named style spec into a [`KeyStyle`], dropping bad values.
pub fn resolve_all(specs: &HashMap<String, KeyStyleSpec>) -> HashMap<String, KeyStyle> {
    specs
        .iter()
        .map(|(name, spec)| {
            let style = KeyStyle {
                background: role(name, "background", spec.background.as_deref()),
                foreground: role(name, "foreground", spec.foreground.as_deref()),
                font: font(name, spec.font.as_deref()),
                font_size: font_size(name, spec.font_size),
            };
            (name.clone(), style)
        })
        .collect()
}

fn for_button(
    styles: &HashMap<String, KeyStyle>,
    token: &str,
    name: Option<&str>,
    types_digit: bool,
) -> KeyStyle {
    if let Some(name) = name {
        if let Some(style) = styles.get(name) {
            return *style;
        }
        warn!("button {token:?} uses key_style {name:?}, which isn't defined; using the default");
    }
    let automatic = if types_digit { NUMBER } else { NORMAL };
    styles
        .get(automatic)
        .or_else(|| styles.get(NORMAL))
        .copied()
        .unwrap_or_default()
}

fn role(style: &str, field: &str, name: Option<&str>) -> Option<ColorRole> {
    let name = name?;
    let role = tokens::color_role(name);
    if role.is_none() {
        warn!("key_style {style:?} {field} {name:?} is not a colour role; ignoring it");
    }
    role
}

fn font(style: &str, name: Option<&str>) -> Option<Font> {
    let name = name?;
    let font = Font::from_name(name);
    if font.is_none() {
        warn!("key_style {style:?} font {name:?} is not a bundled font; ignoring it");
    }
    font
}

fn font_size(style: &str, size: Option<u16>) -> Option<u16> {
    size.filter(|&px| {
        let ok = px > 0;
        if !ok {
            warn!("key_style {style:?} font-size must be greater than 0; ignoring it");
        }
        ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles() -> HashMap<String, KeyStyle> {
        let specs: HashMap<String, KeyStyleSpec> = yaml_serde::from_str(
            "normal:   { background: surface-container-high, foreground: on-surface }\n\
             number:   { font: geist-mono }\n\
             function: { background: surface-container-highest, font-size: 18 }\n\
             broken:   { background: not-a-role, font: not-a-font, font-size: 0 }\n",
        )
        .unwrap();
        resolve_all(&specs)
    }

    #[test]
    fn button_gets_its_named_style() {
        let style = for_button(&styles(), "Shift_L", Some("function"), false);
        assert_eq!(style.background, Some(ColorRole::SurfaceContainerHighest));
        assert_eq!(style.foreground, None);
        assert_eq!(style.font_size, Some(18));
    }

    #[test]
    fn unstyled_or_unknown_style_falls_back_to_normal() {
        let normal = styles()[NORMAL];
        assert_eq!(for_button(&styles(), "q", None, false), normal);
        assert_eq!(for_button(&styles(), "q", Some("nope"), false), normal);
    }

    #[test]
    fn digit_gets_number_style() {
        let style = for_button(&styles(), "1", None, true);
        assert_eq!(style, styles()[NUMBER]);
        assert_eq!(style.font, Some(Font::GeistMono));
    }

    #[test]
    fn digit_without_number_style_falls_back_to_normal() {
        let mut styles = styles();
        styles.remove(NUMBER);
        assert_eq!(for_button(&styles, "1", None, true), styles[NORMAL]);
    }

    #[test]
    fn explicit_style_beats_number() {
        let style = for_button(&styles(), "1", Some("function"), true);
        assert_eq!(style, styles()["function"]);
    }

    #[test]
    fn look_picks_style_by_state() {
        let mut styles = styles();
        styles.insert(
            LATCHED.into(),
            KeyStyle {
                background: Some(ColorRole::Primary),
                ..KeyStyle::default()
            },
        );
        let look = KeyLook::for_button(&styles, "Ctrl", Some("function"), false);
        assert_eq!(look.style(KeyState::Normal), styles["function"]);
        assert_eq!(look.style(KeyState::Latched), styles[LATCHED]);
    }

    #[test]
    fn bad_values_leave_those_fields_unset() {
        assert_eq!(styles()["broken"], KeyStyle::default());
    }
}
