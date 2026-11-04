//! Prediction profiles: which predictor and how much bias to use for the
//! focused field, plus the hook that sees finished words.

use std::collections::HashMap;

use mecha_wayland::prelude::*;

use super::{Dictionary, FLAT, LetterPrior, Predictor};
use crate::field::Field;
use crate::layout::{Family, PredictionConfig};

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
    /// Ordinary text.
    text: Profile,
    /// Email addresses and URLs.
    email_url: Profile,
    /// Terminal input.
    terminal: Profile,
    /// Sensitive, numeric and phone fields: no knowledge, nothing learned.
    off: Profile,
}

impl Profiles {
    /// Profiles derived from the layout's prediction block.
    pub(crate) fn from_config(cfg: &PredictionConfig) -> Profiles {
        let strength = if cfg.enabled { cfg.strength } else { 0.0 };
        let profile = |dictionary: &str, strength: f32| Profile {
            predictor: Some(dictionary.to_string()),
            strength,
            popup: cfg.popup,
            may_record: true,
        };
        Profiles {
            text: profile(&cfg.dictionary, strength),
            // Addresses follow the word list less closely: half strength.
            email_url: profile(&cfg.dictionary, strength * 0.5),
            terminal: profile(&cfg.terminal_dictionary, strength),
            off: Profile {
                predictor: None,
                strength: 0.0,
                popup: cfg.popup,
                may_record: false,
            },
        }
    }

    /// The profile for this field. Sensitive fields are always off.
    pub(crate) fn select(&self, field: Field) -> &Profile {
        if field.sensitive {
            return &self.off;
        }
        match field.family {
            Family::Text => &self.text,
            Family::Email | Family::Url => &self.email_url,
            Family::Terminal => &self.terminal,
            _ => &self.off,
        }
    }

    /// Every profile, for deciding which dictionaries are needed.
    fn all(&self) -> [&Profile; 4] {
        [&self.text, &self.email_url, &self.terminal, &self.off]
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
    /// Build from the layout's prediction block: loads each dictionary a
    /// profile with strength above 0 refers to, once, under its spec string.
    /// With prediction off nothing is loaded and the flat predictor stands in.
    pub(crate) fn from_config(cfg: &PredictionConfig) -> Prediction {
        let profiles = Profiles::from_config(cfg);
        let mut predictors: HashMap<String, Box<dyn Predictor>> = HashMap::new();
        for profile in profiles.all() {
            if profile.strength > 0.0
                && let Some(key) = &profile.predictor
            {
                predictors
                    .entry(key.clone())
                    .or_insert_with(|| Box::new(Dictionary::load(key)));
            }
        }
        Prediction {
            profiles,
            predictors,
            recorder: Box::new(NoopRecorder),
        }
    }

    /// The profile and predictor for a field; a missing or unset predictor
    /// gives a flat one.
    pub(crate) fn active(&self, field: Field) -> (&Profile, &dyn Predictor) {
        let profile = self.profiles.select(field);
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
            dictionary: "en".into(),
            terminal_dictionary: "terminal".into(),
            popup: false,
        }
    }

    fn field(family: Family, sensitive: bool) -> Field {
        Field { family, sensitive }
    }

    #[test]
    fn text_uses_en_at_configured_strength() {
        let profiles = Profiles::from_config(&cfg(true));
        let p = profiles.select(field(Family::Text, false));
        assert_eq!(p.predictor.as_deref(), Some("en"));
        assert_eq!(p.strength, 0.7);
        assert!(!p.popup);
        assert!(p.may_record);
    }

    #[test]
    fn disabled_config_gives_zero_strength() {
        let profiles = Profiles::from_config(&cfg(false));
        for family in [Family::Text, Family::Email, Family::Terminal] {
            assert_eq!(profiles.select(field(family, false)).strength, 0.0);
        }
    }

    #[test]
    fn terminal_field_uses_terminal_dictionary() {
        let profiles = Profiles::from_config(&cfg(true));
        let p = profiles.select(field(Family::Terminal, false));
        assert_eq!(p.predictor.as_deref(), Some("terminal"));
        assert_eq!(p.strength, 0.7);
        assert!(p.may_record);
    }

    #[test]
    fn sensitive_terminal_is_off_and_records_nothing() {
        let profiles = Profiles::from_config(&cfg(true));
        let p = profiles.select(field(Family::Terminal, true));
        assert_eq!(p.predictor, None);
        assert_eq!(p.strength, 0.0);
        assert!(!p.may_record);
    }

    #[test]
    fn password_is_off() {
        let profiles = Profiles::from_config(&cfg(true));
        let p = profiles.select(field(Family::Password, true));
        assert_eq!((p.predictor.as_deref(), p.strength), (None, 0.0));
        assert!(!p.may_record);
        assert_eq!(
            profiles.select(field(Family::Password, false)).strength,
            0.0
        );
    }

    #[test]
    fn email_and_url_are_half_strength() {
        let profiles = Profiles::from_config(&cfg(true));
        for family in [Family::Email, Family::Url] {
            let p = profiles.select(field(family, false));
            assert_eq!(p.predictor.as_deref(), Some("en"));
            assert_eq!(p.strength, 0.35);
        }
    }

    #[test]
    fn number_and_phone_are_off() {
        let profiles = Profiles::from_config(&cfg(true));
        for family in [Family::Number, Family::Phone] {
            let p = profiles.select(field(family, false));
            assert_eq!((p.predictor.as_deref(), p.strength), (None, 0.0));
            assert!(!p.may_record);
        }
    }

    #[test]
    fn disabled_config_loads_no_dictionary() {
        let prediction = Prediction::from_config(&cfg(false));
        assert!(prediction.predictors.is_empty());
        let (_, predictor) = prediction.active(field(Family::Text, false));
        assert_eq!(predictor.prior("th"), FLAT);
    }

    #[test]
    fn enabled_config_loads_both_dictionaries() {
        let prediction = Prediction::from_config(&cfg(true));
        assert_eq!(prediction.predictors.len(), 2);
        let (_, terminal) = prediction.active(field(Family::Terminal, false));
        assert_ne!(terminal.prior("sys"), FLAT);
    }

    #[test]
    fn missing_predictor_is_flat() {
        let mut prediction = Prediction::from_config(&cfg(true));
        prediction.predictors.clear();
        let (_, predictor) = prediction.active(field(Family::Text, false));
        assert_eq!(predictor.prior("th"), FLAT);
    }
}
