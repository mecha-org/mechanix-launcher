use mecha_wayland::prelude::*;

mod brightness;
mod volume;

use brightness::BrightnessSliderBuilder;
use volume::VolumeSliderBuilder;

/// Extra space under the sliders. They stay centered in what's left, so
/// this moves them up by half its value.
const BOTTOM_PADDING: f32 = 90.0;

pub struct Sliders;

pub struct SlidersBuilder;

impl Build for SlidersBuilder {
    type Widget = Sliders;
}

impl Sliders {
    /// Portrait: take all the height left between the header and the
    /// bottom bar, and center the slider column inside it.
    /// Landscape: take half the width beside the tiles, centered with them.
    pub fn style(landscape: bool) -> LayoutStyle {
        let style = LayoutStyle::default()
            .grow(1.0)
            .min_height(px(0.0))
            .column()
            .justify(Justify::Center);
        if landscape {
            style.flex_basis(px(0.0)).min_width(px(0.0))
        } else {
            style.width(percent(100.0)).padding(Insets::new(
                px(0.0),
                px(0.0),
                px(BOTTOM_PADDING),
                px(0.0),
            ))
        }
    }
}

impl Widget for Sliders {
    type Builder = SlidersBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = Sliders::style(false);

        let col = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .center()
                    .gap(px(Spacing::Space500.dp())),
            ),
        );
        s.spawn(col, VolumeSliderBuilder);
        s.spawn(col, BrightnessSliderBuilder);

        Sliders
    }
}
