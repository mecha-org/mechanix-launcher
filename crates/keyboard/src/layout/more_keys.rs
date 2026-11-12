//! Long-press data: the corner hint and the choices row for each key, per
//! named set in the bundled `more_keys.yaml`. Loaded once.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tracing::warn;
use yaml_serde::from_str;

use super::keymap::{KeyFace, View};
use serde::Deserialize;

static MORE_KEYS_YAML: &str = include_str!("../../resources/more_keys.yaml");

#[derive(Debug, Deserialize)]
struct FileSpec {
    sets: HashMap<String, SetSpec>,
}

#[derive(Debug, Default, Deserialize)]
struct SetSpec {
    extends: Option<String>,
    #[serde(default)]
    hints: HashMap<String, String>,
    #[serde(default)]
    choices: HashMap<String, String>,
    /// Tokens whose hint is display-only (not offered as a choice).
    #[serde(default)]
    hint_only: Vec<String>,
}

/// One resolved set: `extends` already merged in.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct MoreKeySet {
    hints: HashMap<String, String>,
    choices: HashMap<String, Vec<String>>,
    hint_only: HashSet<String>,
}

/// Every set from `more_keys.yaml`, by name.
#[derive(Debug, Default)]
pub(crate) struct MoreKeys {
    sets: HashMap<String, MoreKeySet>,
}

impl MoreKeys {
    pub(crate) fn parse(yaml: &str) -> Self {
        let file: FileSpec = match from_str(yaml) {
            Ok(f) => f,
            Err(e) => {
                warn!("more_keys.yaml does not parse ({e}); no long-press data");
                return Self::default();
            }
        };
        let sets = file
            .sets
            .keys()
            .map(|name| (name.clone(), resolve(&file, name, &mut Vec::new())))
            .collect();
        Self { sets }
    }

    /// The bundled data, parsed once.
    pub(crate) fn bundled() -> &'static MoreKeys {
        static CELL: OnceLock<MoreKeys> = OnceLock::new();
        CELL.get_or_init(|| Self::parse(MORE_KEYS_YAML))
    }

    pub(crate) fn set(&self, name: &str) -> Option<&MoreKeySet> {
        self.sets.get(name)
    }
}

/// Set `name` with its `extends` chain merged (the child wins per token).
fn resolve(file: &FileSpec, name: &str, visiting: &mut Vec<String>) -> MoreKeySet {
    let Some(spec) = file.sets.get(name) else {
        warn!("more_keys set {name:?} is not defined");
        return MoreKeySet::default();
    };
    let mut set = match &spec.extends {
        Some(parent) if visiting.iter().any(|v| v == parent) || parent == name => {
            warn!("more_keys set {name:?} extends {parent:?} through a cycle; ignoring");
            MoreKeySet::default()
        }
        Some(parent) => {
            visiting.push(name.to_string());
            let base = resolve(file, parent, visiting);
            visiting.pop();
            base
        }
        None => MoreKeySet::default(),
    };
    set.hints
        .extend(spec.hints.iter().map(|(k, v)| (k.clone(), v.clone())));
    for (token, list) in &spec.choices {
        set.choices.insert(
            token.clone(),
            list.split_whitespace().map(str::to_string).collect(),
        );
    }
    set.hint_only.extend(spec.hint_only.iter().cloned());
    set
}

/// Fill `hint` and `choices` on every text key of `view` from `set`.
pub(crate) fn attach(view: &mut View, set: &MoreKeySet) {
    for row in &mut view.rows {
        for key in row.keys_mut() {
            let token = key.token.as_str();
            let hint = set.hints.get(token);
            let mut choices = set.choices.get(token).cloned().unwrap_or_default();
            if let Some(h) = hint
                && !set.hint_only.contains(token)
                && choices.first() != Some(h)
            {
                choices.insert(0, h.clone());
            }
            if matches!(key.face, KeyFace::Text(_)) {
                key.hint = hint.cloned();
            }
            key.choices = choices;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::keymap::{Family, Page};

    fn set(name: &str) -> MoreKeySet {
        MoreKeys::bundled().set(name).expect("set exists").clone()
    }

    /// (hint, choices) of `token` as `attach` would give it.
    fn of(name: &str, token: &str) -> (Option<String>, Vec<String>) {
        let s = set(name);
        let mut view = View {
            name: "t".into(),
            family: Family::Text,
            page: Page::Letters,
            rows: vec![super::super::keymap::Row {
                cells: vec![super::super::keymap::Cell::Key(
                    super::super::keymap::Key::test(token),
                )],
                height: 1.0,
            }],
        };
        attach(&mut view, &s);
        let k = view.keys().next().unwrap();
        (k.hint.clone(), k.choices.clone())
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn top_row_digit_is_hint_and_first_choice() {
        assert_eq!(of("letters", "q"), (Some("1".into()), v(&["1"])));
        assert_eq!(
            of("letters", "e"),
            (Some("3".into()), v(&["3", "é", "è", "ê", "ë", "ē"]))
        );
    }

    #[test]
    fn period_has_sixteen_choices() {
        let (hint, choices) = of("letters", "period");
        assert_eq!(hint, None);
        assert_eq!(choices.len(), 16);
        assert_eq!(choices[0], ",");
        assert_eq!(choices[15], "&");
        assert!(choices.contains(&"\"".to_string()));
    }

    #[test]
    fn password_set_extends_letters_with_symbol_first() {
        let (hint, choices) = of("password", "a");
        assert_eq!(hint.as_deref(), Some("@"));
        assert_eq!(choices, v(&["@", "à", "á", "â", "ä", "æ", "ã", "å", "ā"]));
        assert_eq!(of("password", "q").1, v(&["1"]));
        assert_eq!(of("password", "at").1, v(&["#", "$", "%", "&"]));
    }

    #[test]
    fn terminal_letters_have_no_accents() {
        assert_eq!(of("terminal", "e"), (Some("3".into()), v(&["3"])));
        assert_eq!(of("terminal", "a"), (Some("|".into()), v(&["|"])));
        assert_eq!(of("terminal", "k").1, v(&["(", "[", "{"]));
        assert_eq!(of("terminal", "l").1, v(&[")", "]", "}"]));
        assert_eq!(of("terminal", "dot").1, v(&["~", "*"]));
    }

    #[test]
    fn phone_letter_groups_are_hint_only() {
        assert_eq!(of("phone", "2"), (Some("ABC".into()), v(&[])));
        assert_eq!(of("phone", "0"), (Some("+".into()), v(&["+"])));
        assert_eq!(of("phone", "minus").1, v(&["+"]));
    }

    #[test]
    fn symbols_and_dot_com_data() {
        assert_eq!(of("symbols", "$").1, v(&["¢", "£", "€", "¥", "₱"]));
        assert_eq!(of("symbols", "\"").1, v(&["„", "“", "”", "«", "»"]));
        assert_eq!(
            of("letters", "dot_com").1,
            v(&[".org", ".net", ".io", ".dev", ".co.uk"])
        );
    }

    #[test]
    fn unknown_set_or_cycle_warns_not_panics() {
        let mm = MoreKeys::parse(
            "sets:\n  a: { extends: nope, hints: { x: \"1\" } }\n  b: { extends: b }\n",
        );
        assert!(mm.set("a").is_some());
        assert!(mm.set("b").is_some());
        assert!(mm.set("zzz").is_none());
        assert!(MoreKeys::parse("not: [yaml").set("a").is_none());
    }

    #[test]
    fn every_token_named_in_a_set_exists_in_a_view() {
        let keymap = crate::layout::load_keymap();
        let tokens: HashSet<&str> = keymap.keys().map(|k| k.token.as_str()).collect();
        let mm = MoreKeys::bundled();
        for (name, set) in &mm.sets {
            let named = set.hints.keys().chain(set.choices.keys());
            for token in named {
                assert!(
                    tokens.contains(token.as_str()),
                    "set {name:?} names token {token:?}, which no view has"
                );
            }
        }
    }
}
