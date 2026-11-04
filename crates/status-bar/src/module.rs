use mecha_wayland::prelude::*;
use screen::Screen;

use crate::{BluetoothModule, NetworkManagerModule, UpowerModule, status_bar};

/// Installs the status bar's services (Wi-Fi, Bluetooth and UPower), all from
/// `services` and shared with other screens.
/// It spawns no window; place the bar with [`StatusBarScreen`].
///
/// Expects the host to have installed D-Bus, `RingModule` and `TimerModule`
/// (for the clock), and to have inserted the
/// [`Atlas`], `FontBook` and [`StatusIcons`](crate::StatusIcons).
///
/// Build the status bar before adding this module (for example, add the
/// `NavModule` whose container holds a [`StatusBarScreen`] first): the
/// initial UPower reply is emitted to the widgets that exist at that point,
/// so a bar built later misses it.
pub struct StatusBarModule;

impl Module for StatusBarModule {
    fn install(self, app: &mut App) {
        app.add_module(NetworkManagerModule)
            .add_module(BluetoothModule)
            .add_module(UpowerModule);
    }
}

/// The status bar as content: builds a [`StatusBar`](crate::StatusBar) under
/// any parent node.
pub struct StatusBarScreen;

impl Screen for StatusBarScreen {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        app.spawn(parent, status_bar()).id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_ICON_SIZE, GEIST_MONO_BYTES, StatusBar, StatusIcons};
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[test]
    fn status_bar_screen_builds_status_bar() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(WindowModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule)
            .insert_resource(Atlas::new());
        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));
        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("icons load");
        app.insert_resource(icons);

        let root = app.root();
        let bar = StatusBarScreen.build(&mut app, root);
        app.tick();

        assert!(app.widget::<StatusBar>(bar).is_some());
        let bars = app
            .descendants(root)
            .filter(|n| app.widget::<StatusBar>(*n).is_some())
            .count();
        assert_eq!(bars, 1);
        assert_eq!(app.resource::<Windows>().iter().count(), 0);
    }
}
