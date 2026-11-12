use std::collections::HashMap;

use mecha_wayland::dbus::Value::{self, Str, U8};

/* NOTE: Images are handled separately.
 The freedesktop specs mentions image data in hints,
 as they involve priorities between app_icon and
 hint's image data, it stays as a separate module
*/
#[derive(Debug, Default, Clone)]
pub struct Hints {
    pub urgency: Urgency,
    pub category: Option<String>,
    pub desktop_entry: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub enum Urgency {
    LOW,
    #[default]
    NORMAL,
    CRITICAL,
}

impl Hints {
    pub fn parse(hints: HashMap<String, Value>) -> Self {
        let mut hints_struct = Hints::default();
        for (hint, value) in hints {
            match (&hint[..], value) {
                ("urgency", U8(level)) => {
                    hints_struct.urgency = match level {
                        0 => Urgency::LOW,
                        2 => Urgency::CRITICAL,
                        _ => Urgency::NORMAL,
                    };
                }
                ("category", Str(s)) => hints_struct.category = Some(s),
                ("desktop-entry", Str(s)) => hints_struct.desktop_entry = Some(s),
                // unknown hints, not supported
                _ => {}
            }
        }
        hints_struct
    }
}
