//! Prediction profiles: which predictor and how much bias to use for the
//! focused field, plus the hook that sees finished words.

use std::collections::HashMap;

use mecha_wayland::prelude::*;

use super::{Dictionary, FLAT, LetterPrior, Predictor};
use crate::layout::PredictionConfig;

/// How prediction behaves for one kind of field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Profile {
    /// Key of the predictor to use in [`Prediction::predictors`]; `None` means
    /// no knowledge (flat prior).
    pub predictor: Option<String>,
    /// How strongly the prior biases touch targets; 0 disables the bias.
    pub strength: f32,
    /// Whether the key pop-up is shown.
    pub popup: bool,
    /// Whether finished words may be handed to the [`WordRecorder`].
    pub may_record: bool,
}

/// The profile for each kind of field.
pub(crate) struct Profiles {
    default: Profile,
}

impl Profiles {
    /// Profiles derived from the layout's prediction block.
    pub(crate) fn from_config(cfg: &PredictionConfig) -> Profiles {
        Profiles {
            default: Profile {
                predictor: Some(cfg.dictionary.clone()),
                strength: if cfg.enabled { cfg.strength } else { 0.0 },
                popup: cfg.popup,
                may_record: true,
            },
        }
    }

    /// The profile for a field with this purpose and hints. Every field uses
    /// the default for now.
    pub(crate) fn select(
        &self,
        _purpose: ZwpTextInputV3ContentPurpose,
        _hint: ZwpTextInputV3ContentHint,
    ) -> &Profile {
        &self.default
    }
}

/// Receives each word the user finishes typing.
pub(crate) trait WordRecorder: Send + Sync {
    /// Called with a finished word.
    fn record(&mut self, word: &str);
}

/// A recorder that discards every word.
pub(crate) struct NoopRecorder;

impl WordRecorder for NoopRecorder {
    fn record(&mut self, _word: &str) {}
}

/// The predictor with no knowledge: always [`FLAT`].
struct FlatPredictor;

impl Predictor for FlatPredictor {
    fn prior(&self, _prefix: &str) -> LetterPrior {
        FLAT
    }
}

static FLAT_PREDICTOR: FlatPredictor = FlatPredictor;

/// Everything prediction needs at runtime. Lives on the app as a `Resource`.
pub(crate) struct Prediction {
    /// Profile per field kind.
    pub profiles: Profiles,
    /// Loaded predictors, by the key profiles refer to.
    pub predictors: HashMap<String, Box<dyn Predictor>>,
    /// Sees every finished word.
    pub recorder: Box<dyn WordRecorder>,
}
impl Resource for Prediction {}

impl Prediction {
    /// Build from the layout's prediction block: loads the dictionary once and
    /// registers it under its spec string. With prediction off (strength 0)
    /// nothing is loaded and the flat predictor stands in.
    pub(crate) fn from_config(cfg: &PredictionConfig) -> Prediction {
        let profiles = Profiles::from_config(cfg);
        let mut predictors: HashMap<String, Box<dyn Predictor>> = HashMap::new();
        if profiles.default.strength > 0.0 {
            predictors.insert(
                cfg.dictionary.clone(),
                Box::new(Dictionary::load(&cfg.dictionary)),
            );
        }
        Prediction {
            profiles,
            predictors,
            recorder: Box::new(NoopRecorder),
        }
    }

    /// The profile and predictor for a field; a missing or unset predictor
    /// gives a flat one.
    pub(crate) fn active(
        &self,
        purpose: ZwpTextInputV3ContentPurpose,
        hint: ZwpTextInputV3ContentHint,
    ) -> (&Profile, &dyn Predictor) {
        let profile = self.profiles.select(purpose, hint);
        let predictor = profile
            .predictor
            .as_ref()
            .and_then(|key| self.predictors.get(key))
            .map_or(&FLAT_PREDICTOR as &dyn Predictor, |p| p.as_ref());
        (profile, predictor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool) -> PredictionConfig {
        PredictionConfig {
            enabled,
            strength: 0.7,
            dictionary: "bundled".into(),
            popup: false,
        }
    }

    const NONE: ZwpTextInputV3ContentHint = ZwpTextInputV3ContentHint::NONE;

    #[test]
    fn default_profile_follows_config() {
        let profiles = Profiles::from_config(&cfg(true));
        let p = profiles.select(ZwpTextInputV3ContentPurpose::Normal, NONE);
        assert_eq!(p.predictor.as_deref(), Some("bundled"));
        assert_eq!(p.strength, 0.7);
        assert!(!p.popup);
        assert!(p.may_record);
    }

    #[test]
    fn disabled_config_gives_zero_strength() {
        let profiles = Profiles::from_config(&cfg(false));
        let p = profiles.select(ZwpTextInputV3ContentPurpose::Normal, NONE);
        assert_eq!(p.strength, 0.0);
    }

    #[test]
    fn every_purpose_selects_default_for_now() {
        let profiles = Profiles::from_config(&cfg(true));
        let normal = profiles
            .select(ZwpTextInputV3ContentPurpose::Normal, NONE)
            .clone();
        for purpose in [
            ZwpTextInputV3ContentPurpose::Password,
            ZwpTextInputV3ContentPurpose::Url,
            ZwpTextInputV3ContentPurpose::Email,
            ZwpTextInputV3ContentPurpose::Terminal,
        ] {
            assert_eq!(*profiles.select(purpose, NONE), normal);
        }
    }

    #[test]
    fn disabled_config_loads_no_dictionary() {
        let prediction = Prediction::from_config(&cfg(false));
        assert!(prediction.predictors.is_empty());
        let (_, predictor) = prediction.active(ZwpTextInputV3ContentPurpose::Normal, NONE);
        assert_eq!(predictor.prior("th"), FLAT);
        assert!(!Prediction::from_config(&cfg(true)).predictors.is_empty());
    }

    #[test]
    fn missing_predictor_is_flat() {
        let mut prediction = Prediction::from_config(&cfg(true));
        prediction.predictors.clear();
        let (_, predictor) = prediction.active(ZwpTextInputV3ContentPurpose::Normal, NONE);
        assert_eq!(predictor.prior("th"), FLAT);
    }
}
