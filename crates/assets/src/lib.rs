//! Fonts and icons shared by more than one launcher crate, embedded once so
//! every consumer draws from the same file.

pub mod fonts {
    pub const GEIST_MONO: &[u8] = include_bytes!("../fonts/GeistMono-Regular.ttf");
    pub const GEIST_MONO_LIGHT: &[u8] = include_bytes!("../fonts/GeistMono-Light.ttf");
}

pub mod icons {
    pub const AIRPLANE: &[u8] = include_bytes!("../icons/airplane.svg");
    pub const APP: &[u8] = include_bytes!("../icons/app.svg");
    /// Go on: send what was typed.
    pub const ARROW_RIGHT: &[u8] = include_bytes!("../icons/arrow-right.svg");
    pub const SEARCH: &[u8] = include_bytes!("../icons/magnifying-glass.svg");
    pub const CLEAR: &[u8] = include_bytes!("../icons/x.svg");
    pub const BACKSPACE: &[u8] = include_bytes!("../icons/backspace.svg");
    pub const ENTER: &[u8] = include_bytes!("../icons/enter.svg");
    /// Show a password as typed, and hide it again.
    pub const EYE: &[u8] = include_bytes!("../icons/eye.svg");
    pub const EYE_SLASH: &[u8] = include_bytes!("../icons/eye-slash.svg");
    pub const POWER: &[u8] = include_bytes!("../icons/power.svg");
    /// Turning arrows: rotation, and restart.
    pub const ROTATE: &[u8] = include_bytes!("../icons/rotate.svg");
    /// The Comet wheel, traced from the boot splash.
    pub const COMET_LOGO: &[u8] = include_bytes!("../icons/comet-logo.svg");

    pub mod wifi {
        pub const HIGH: &[u8] = include_bytes!("../icons/wifi/wifi-high.svg");
        pub const MEDIUM: &[u8] = include_bytes!("../icons/wifi/wifi-medium.svg");
        pub const LOW: &[u8] = include_bytes!("../icons/wifi/wifi-low.svg");
        pub const NONE: &[u8] = include_bytes!("../icons/wifi/wifi-none.svg");
        pub const CONNECTING: &[u8] = include_bytes!("../icons/wifi/wifi-connecting.svg");
        pub const SLASH: &[u8] = include_bytes!("../icons/wifi/wifi-slash.svg");
        pub const X: &[u8] = include_bytes!("../icons/wifi/wifi-x.svg");
    }

    pub mod bluetooth {
        pub const DEFAULT: &[u8] = include_bytes!("../icons/bluetooth/bluetooth.svg");
        pub const CONNECTED: &[u8] = include_bytes!("../icons/bluetooth/bluetooth-connected.svg");
        pub const SLASH: &[u8] = include_bytes!("../icons/bluetooth/bluetooth-slash.svg");
        pub const X: &[u8] = include_bytes!("../icons/bluetooth/bluetooth-x.svg");
    }

    pub mod cellular {
        pub const FULL: &[u8] = include_bytes!("../icons/cellular/cell-signal-full.svg");
        pub const HIGH: &[u8] = include_bytes!("../icons/cellular/cell-signal-high.svg");
        pub const MEDIUM: &[u8] = include_bytes!("../icons/cellular/cell-signal-medium.svg");
        pub const LOW: &[u8] = include_bytes!("../icons/cellular/cell-signal-low.svg");
        pub const NONE: &[u8] = include_bytes!("../icons/cellular/cell-signal-none.svg");
        pub const SLASH: &[u8] = include_bytes!("../icons/cellular/cell-signal-slash.svg");
        pub const X: &[u8] = include_bytes!("../icons/cellular/cell-signal-x.svg");
    }
}
