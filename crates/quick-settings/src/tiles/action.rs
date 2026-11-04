use mecha_wayland::prelude::*;

use super::{GAP, frame};
use crate::util::load_image;

/// Full-colour SVGs for the [`ActionTile`]s.
pub mod svg {
    pub const CAMERA: &[u8] = include_bytes!("../../assets/camera.svg");
    pub const DISPLAY: &[u8] = include_bytes!("../../assets/display.svg");
    pub const MICROPHONE: &[u8] = include_bytes!("../../assets/microphone.svg");
    pub const RECORD: &[u8] = include_bytes!("../../assets/record.svg");
    pub const ROTATE: &[u8] = include_bytes!("../../assets/rotate.svg");
    pub const SETTINGS: &[u8] = include_bytes!("../../assets/settings.svg");
    pub const TERMINAL: &[u8] = include_bytes!("../../assets/terminal.svg");
}

/// A stateless tile that only shows an icon (camera, terminal, settings, ...).
pub struct ActionTile;

pub struct ActionTileBuilder {
    /// Full-colour SVG, usually one of [`svg`]. Its own colours are kept and
    /// do not follow the theme.
    pub icon: &'static [u8],
}

/// Builds an [`ActionTile`] showing `icon`, e.g. `action_tile(svg::CAMERA)`.
pub fn action_tile(icon: &'static [u8]) -> ActionTileBuilder {
    ActionTileBuilder { icon }
}

impl Build for ActionTileBuilder {
    type Widget = ActionTile;
}

impl Widget for ActionTile {
    type Builder = ActionTileBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        frame(me, s, GAP);
        let sprite = load_image(&mut s.resource_mut::<Atlas>(), b.icon);
        s.spawn(me, image(sprite).style(LayoutStyle::default().shrink(0.0)));
        ActionTile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::WifiBuilder;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[test]
    fn action_tile_keeps_svg_colours_and_icon_tile_is_tinted() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let mic = app.spawn(app.root(), action_tile(svg::MICROPHONE));
        let wifi = app.spawn(
            app.root(),
            WifiBuilder {
                name: String::new(),
            },
        );
        app.tick();

        let paint_of = |app: &App, tile: NodeId| {
            let child = app.children(tile).unwrap()[0];
            app.component::<Paint>(child).unwrap().clone()
        };
        assert!(matches!(paint_of(&app, mic.id()), Paint::Polychrome(_)));
        assert!(matches!(paint_of(&app, wifi.id()), Paint::Monochrome(_)));
    }
}
