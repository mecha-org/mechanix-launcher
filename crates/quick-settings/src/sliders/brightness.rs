use mecha_wayland::mechanix_widgets::prelude::{Slider, SliderChange, SliderContextExt, slider};
use mecha_wayland::prelude::*;

use crate::util::load_icon;
use services::{BrightnessChanged, MAX_BRIGHTNESS, MIN_BRIGHTNESS, SetBrightness};

mod svg {
    pub const MEDIUM: &[u8] = include_bytes!("../../assets/brightness/brightness-medium.svg");
}

const ICON_SIZE: f32 = 24.0;
const SLIDER_LENGTH_PERCENT: f32 = 80.0;

pub struct BrightnessSlider {
    pub slider: Handle<Slider>,
}

pub struct BrightnessSliderBuilder;

impl Build for BrightnessSliderBuilder {
    type Widget = BrightnessSlider;
}

impl Widget for BrightnessSlider {
    type Builder = BrightnessSliderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().width(percent(100.0));

        let icon_sprite = load_icon(&mut s.resource_mut::<Atlas>(), svg::MEDIUM);

        let row = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .width(percent(100.0))
                    .center()
                    .gap(px(Spacing::Space200.dp())),
            ),
        );

        s.spawn(
            row,
            icon(icon_sprite)
                .style(LayoutStyle::default().shrink(0.0))
                .size(Size::new(ICON_SIZE, ICON_SIZE))
                .color(s.color(ColorRole::OnSurface)),
        );

        let slider_handle = s.spawn(
            row,
            slider()
                .value(MIN_BRIGHTNESS)
                .range(MIN_BRIGHTNESS, MAX_BRIGHTNESS)
                .length(percent(SLIDER_LENGTH_PERCENT))
                .tick_height(40.0)
                .tick_gap(4.0),
        );

        s.on::<SliderChange>(slider_handle, |ctx, event| {
            ctx.signal(SetBrightness(event.value));
        });

        s.on::<BrightnessChanged>(me, |ctx, event| {
            let slider = ctx.me().slider;
            if let Some(mut slider) = ctx.at(slider) {
                slider.set_value(event.value);
            }
        });

        BrightnessSlider {
            slider: slider_handle,
        }
    }
}
