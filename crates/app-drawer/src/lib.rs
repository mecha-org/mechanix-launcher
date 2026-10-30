#[cfg_attr(not(test), allow(dead_code))]
mod entries;
#[cfg_attr(not(test), allow(dead_code))]
mod icons;
#[cfg_attr(not(test), allow(dead_code))]
mod rank;

use mecha_wayland::prelude::*;
use screen::{Screen, placeholder_page};

pub struct AppDrawer;

impl Screen for AppDrawer {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        placeholder_page(app, parent, "App Drawer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::{FontBook, Text};
    use status_bar::GEIST_MONO_BYTES;

    #[test]
    fn builds_one_label() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());
        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let root = app.root();
        let page = AppDrawer.build(&mut app, root);
        app.tick();

        let texts: Vec<_> = app
            .descendants(page)
            .filter(|n| app.widget::<Text>(*n).is_some())
            .collect();
        assert_eq!(texts.len(), 1);
    }
}
