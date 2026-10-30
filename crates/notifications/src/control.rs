#[derive(Default, Debug, Clone, PartialEq)]
pub enum Control {
    #[default]
    Dismiss,
    Snooze,
}

impl Control {
    pub const ALL: [Control; 2] = [Control::Dismiss, Control::Snooze];

    pub fn label(&self) -> &'static str {
        match self {
            Control::Dismiss => "Dismiss",
            Control::Snooze => "Snooze",
        }
    }
}
