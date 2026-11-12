use mecha_wayland::prelude::*;

use super::{Borders, frame};
use crate::util::load_icon;

/// One-colour SVGs for the [`ActionTile`]s.
pub mod svg {
    pub const CAMERA: &[u8] = include_bytes!("../../assets/camera.svg");
    pub const MICROPHONE: &[u8] = include_bytes!("../../assets/microphone.svg");
    pub const RECORD: &[u8] = include_bytes!("../../assets/record.svg");
    pub const ROTATE: &[u8] = assets::icons::ROTATE;
    pub const SETTINGS: &[u8] = include_bytes!("../../assets/settings.svg");
    pub const TERMINAL: &[u8] = include_bytes!("../../assets/terminal.svg");
}

/// A stateless tile that only shows an icon (camera, terminal, settings, ...).
pub struct ActionTile;

pub struct ActionTileBuilder {
    /// SVG, usually one of [`svg`]. Its colours are dropped and it is tinted
    /// with `OnSecondaryFixedVariant`.
    pub icon: &'static [u8],
    /// Which sides get a border, e.g. [`borders::ALL`](super::borders::ALL).
    pub borders: Borders,
}

/// Builds an [`ActionTile`] showing `icon` with the given border sides,
/// e.g. `action_tile(svg::CAMERA, borders::ALL)`.
pub fn action_tile(icon: &'static [u8], borders: Borders) -> ActionTileBuilder {
    ActionTileBuilder { icon, borders }
}

impl Build for ActionTileBuilder {
    type Widget = ActionTile;
}

impl Widget for ActionTile {
    type Builder = ActionTileBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        frame(me, s, b.borders);
        let sprite = load_icon(&mut s.resource_mut::<Atlas>(), b.icon);
        s.spawn(
            me,
            icon(sprite)
                .style(LayoutStyle::default().shrink(0.0))
                .color(s.color(ColorRole::OnSecondaryFixedVariant)),
        );
        ActionTile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{WifiBuilder, borders};
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[test]
    fn action_tile_icon_is_tinted_with_on_secondary_fixed_variant() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let mic = app.spawn(app.root(), action_tile(svg::MICROPHONE, borders::ALL));
        let wifi = app.spawn(
            app.root(),
            WifiBuilder {
                borders: borders::ALL,
            },
        );
        app.tick();

        let paint_of = |app: &App, tile: NodeId| {
            let child = app.children(tile).unwrap()[0];
            app.component::<Paint>(child).unwrap().clone()
        };
        assert!(matches!(paint_of(&app, mic.id()), Paint::Monochrome(_)));
        assert!(matches!(paint_of(&app, wifi.id()), Paint::Monochrome(_)));
    }
}
