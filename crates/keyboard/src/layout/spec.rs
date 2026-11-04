//! The raw serde structs a squeekboard `layout.yaml` deserializes into, before
//! being resolved into the [`crate::layout::Keymap`] IR.

use serde::Deserialize;
use std::collections::HashMap;

use super::style::KeyStyleSpec;

#[derive(Debug, Deserialize)]
pub(crate) struct Layout {
    #[serde(default)]
    pub(crate) keyboard: KeyboardSpec,
    /// Predictive-touch settings.
    #[serde(default)]
    pub(crate) prediction: PredictionSpec,
    /// Named key colour styles; a button picks one with `key_style:`.
    #[serde(default)]
    pub(crate) key_styles: HashMap<String, KeyStyleSpec>,
    pub(crate) outlines: HashMap<String, Outline>,
    pub(crate) views: HashMap<String, ViewSpec>,
    #[serde(default)]
    pub(crate) buttons: HashMap<String, Button>,
}

/// A view: a plain list of rows, or a map with family/page/extends metadata.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ViewSpec {
    Rows(Vec<RowSpec>),
    Map(ViewMap),
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ViewMap {
    pub(crate) family: Option<String>,
    pub(crate) page: Option<String>,
    /// Copy rows from this view (after this view's own `rows`), then `replace`.
    pub(crate) extends: Option<String>,
    /// Token to token, applied to every occurrence.
    #[serde(default)]
    pub(crate) replace: HashMap<String, String>,
    /// Which set in `more_keys.yaml` this view uses (consumed later).
    #[allow(dead_code)]
    pub(crate) more_keys: Option<String>,
    pub(crate) rows: Option<Vec<RowSpec>>,
}

/// A row: a whitespace-separated token string, or one with a height weight.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum RowSpec {
    Keys(String),
    Sized { height: Option<f32>, keys: String },
}

/// Applies to the entire keyboard container layout.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct KeyboardSpec {
    pub(crate) mode: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) radius: Option<String>,
    pub(crate) gap: Option<f32>,
    #[serde(rename = "row-gap")]
    pub(crate) row_gap: Option<f32>,
    pub(crate) padding: Option<f32>,
    pub(crate) width: Option<f32>,
    pub(crate) height: Option<f32>,
    pub(crate) font: Option<String>,
    #[serde(rename = "font-size")]
    pub(crate) font_size: Option<u16>,
}

/// The raw `prediction:` block; every field is optional.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PredictionSpec {
    pub(crate) enabled: Option<bool>,
    pub(crate) strength: Option<f32>,
    pub(crate) dictionary: Option<String>,
    pub(crate) popup: Option<bool>,
}

/// A button's size. Only the width is used — it sets the key's flex-grow weight
/// within its row. Rows split the keyboard height evenly, so an outline's
/// `height:` in the YAML is accepted but ignored.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct Outline {
    pub(crate) width: f32,
}

/// A squeekboard button's `action:` value — either a bare name (`erase`,
/// `show_prefs`) or a structured single-key map (`set_view`, `locking`). Tried
/// as untagged variants in order; `Other` is the catch-all, so an unrecognised
/// structured action still parses (and later resolves to `Unhandled`) instead
/// of failing the whole layout.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ActionSpec {
    Named(String),
    SetView { set_view: String },
    Page { page: String },
    Locking { locking: LockingSpec },
    Other(serde::de::IgnoredAny),
}

/// The body of a squeekboard `locking` action: the view tapped to (`lock_view`),
/// and the view tapped back to (`unlock_view`) once the lock view is current.
#[derive(Debug, Deserialize)]
pub(crate) struct LockingSpec {
    pub(crate) lock_view: String,
    pub(crate) unlock_view: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Button {
    pub(crate) outline: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) icon: Option<String>,
    /// Emit fields the Key action resolves from; see `resolve_action`.
    pub(crate) keysym: Option<String>,
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) action: Option<ActionSpec>,
    pub(crate) modifier: Option<String>,
    pub(crate) key_style: Option<String>,
}
