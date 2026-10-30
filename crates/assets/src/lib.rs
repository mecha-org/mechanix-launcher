//! Fonts and icons shared by more than one launcher crate, embedded once so
//! every consumer draws from the same file.

pub mod fonts {
    pub const GEIST_MONO: &[u8] = include_bytes!("../fonts/GeistMono-Regular.ttf");
}

pub mod icons {
    pub const AIRPLANE: &[u8] = include_bytes!("../icons/airplane.svg");
    pub const APP: &[u8] = include_bytes!("../icons/app.svg");
    pub const SEARCH: &[u8] = include_bytes!("../icons/magnifying-glass.svg");
    pub const CLEAR: &[u8] = include_bytes!("../icons/x.svg");

    pub mod wifi {
        pub const HIGH: &[u8] = include_bytes!("../icons/wifi/wifi-high.svg");
        pub const MEDIUM: &[u8] = include_bytes!("../icons/wifi/wifi-medium.svg");
        pub const LOW: &[u8] = include_bytes!("../icons/wifi/wifi-low.svg");
        pub const NONE: &[u8] = include_bytes!("../icons/wifi/wifi-none.svg");
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
