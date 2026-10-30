use crate::millis::Millis;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnoozeControls {
    Mins(u32),
    Hours(u32),
    Days(u32),
}

impl SnoozeControls {
    pub const fn millis(self) -> Millis {
        match self {
            SnoozeControls::Mins(n) => Millis::mins(n),
            SnoozeControls::Hours(n) => Millis::hours(n),
            SnoozeControls::Days(n) => Millis::hours(n * 24),
        }
    }

    pub fn label(self) -> String {
        match self {
            SnoozeControls::Mins(n) => format!("{n}m"),
            SnoozeControls::Hours(n) => format!("{n}h"),
            SnoozeControls::Days(n) => format!("{n}d"),
        }
    }
}

pub const SNOOZE_OPTIONS: [SnoozeControls; 3] = [
    SnoozeControls::Mins(15),
    SnoozeControls::Hours(1),
    SnoozeControls::Hours(2),
];
