//! The focused text field as the keyboard sees it: which layout family suits
//! its content purpose, and whether what is typed there is sensitive.

use mecha_wayland::prelude::*;

use crate::layout::Family;
use crate::protocol::input_method::InputMethodState;
use crate::widget::keyboard::Keyboard;

/// What the keyboard needs to know about the focused field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Field {
    /// The layout family to show.
    pub(crate) family: Family,
    /// Whether the text typed here must not be learned or echoed.
    pub(crate) sensitive: bool,
}

impl Field {
    /// Derive the field from the client's content purpose and hints.
    pub(crate) fn from_purpose(
        purpose: ZwpTextInputV3ContentPurpose,
        hint: ZwpTextInputV3ContentHint,
    ) -> Field {
        use ZwpTextInputV3ContentPurpose as P;
        let family = match purpose {
            P::Email => Family::Email,
            P::Url => Family::Url,
            P::Password => Family::Password,
            P::Pin | P::Number | P::Digits => Family::Number,
            P::Phone => Family::Phone,
            P::Terminal => Family::Terminal,
            _ => Family::Text,
        };
        let sensitive = matches!(purpose, P::Password | P::Pin)
            || hint.intersects(
                ZwpTextInputV3ContentHint::SENSITIVEDATA | ZwpTextInputV3ContentHint::HIDDENTEXT,
            );
        let family = if sensitive && family == Family::Text {
            Family::Password
        } else {
            family
        };
        Field { family, sensitive }
    }

    /// The currently focused field, from the input-method state.
    pub(crate) fn current(ctx: &Context<'_, Keyboard>) -> Field {
        let field = &ctx.resource::<InputMethodState>().current;
        Field::from_purpose(field.purpose, field.hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ZwpTextInputV3ContentPurpose as P;

    const NONE: ZwpTextInputV3ContentHint = ZwpTextInputV3ContentHint::NONE;

    #[test]
    fn email_purpose_is_email_family() {
        let f = Field::from_purpose(P::Email, NONE);
        assert_eq!(
            f,
            Field {
                family: Family::Email,
                sensitive: false
            }
        );
    }

    #[test]
    fn pin_is_number_family_and_sensitive() {
        let f = Field::from_purpose(P::Pin, NONE);
        assert_eq!(
            f,
            Field {
                family: Family::Number,
                sensitive: true
            }
        );
    }

    #[test]
    fn terminal_with_hidden_text_stays_terminal_but_is_sensitive() {
        let f = Field::from_purpose(P::Terminal, ZwpTextInputV3ContentHint::HIDDENTEXT);
        assert_eq!(
            f,
            Field {
                family: Family::Terminal,
                sensitive: true
            }
        );
    }

    #[test]
    fn normal_with_sensitive_hint_becomes_password() {
        let f = Field::from_purpose(P::Normal, ZwpTextInputV3ContentHint::SENSITIVEDATA);
        assert_eq!(
            f,
            Field {
                family: Family::Password,
                sensitive: true
            }
        );
        assert_eq!(
            Field::from_purpose(P::Password, NONE).family,
            Family::Password
        );
    }

    #[test]
    fn name_and_dates_are_text() {
        for p in [P::Normal, P::Alpha, P::Name, P::Date, P::Time, P::Datetime] {
            let f = Field::from_purpose(p, NONE);
            assert_eq!(
                f,
                Field {
                    family: Family::Text,
                    sensitive: false
                },
                "{p:?}"
            );
        }
    }

    #[test]
    fn digits_is_number_not_sensitive() {
        for p in [P::Digits, P::Number] {
            assert_eq!(
                Field::from_purpose(p, NONE),
                Field {
                    family: Family::Number,
                    sensitive: false
                }
            );
        }
        assert_eq!(Field::from_purpose(P::Phone, NONE).family, Family::Phone);
        assert_eq!(Field::from_purpose(P::Url, NONE).family, Family::Url);
    }
}
