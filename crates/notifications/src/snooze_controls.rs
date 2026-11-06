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
}
