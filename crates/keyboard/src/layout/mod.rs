//! The keymap: the resolved IR built from a squeekboard `layout.yaml`, plus the
//! loading and resolution that turns the raw [`spec`] structs into [`Key`]s.

mod build;
pub(crate) mod icons;
mod keymap;
mod spec;
pub(crate) mod style;
pub(crate) mod tokens;

use mecha_wayland::prelude::{ColorRole, ThemeMode};
use std::{env, fs};
use tracing::{info, warn};

use spec::{KeyboardSpec, Layout, PredictionSpec};
use tokens::Font;

// `Row` and `View` are part of the module's public surface even where no
// sibling currently names them.
#[allow(unused_imports)]
pub use keymap::{Cell, Family, Key, KeyAction, KeyFace, Keymap, Modifier, Page, Row, View};

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
