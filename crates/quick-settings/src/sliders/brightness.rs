use mecha_wayland::mechanix_widgets::prelude::{
    Slider, SliderChange, SliderContextExt, SliderSize, slider,
};
use mecha_wayland::prelude::*;
use std::time::Duration;

use super::control_style;
use crate::util::load_icon;
use services::{BrightnessChanged, MAX_BRIGHTNESS, MIN_BRIGHTNESS, SetBrightness};

mod svg {
    pub const MEDIUM: &[u8] = include_bytes!("../../assets/brightness/brightness-medium.svg");
}

const ICON_SIZE: f32 = 24.0;
const SLIDER_LENGTH_PERCENT: f32 = 80.0;
const ICON_GAP_PERCENT: f32 = 5.0;
const THROTTLE: Duration = Duration::from_millis(50);

pub struct BrightnessSlider {
    pub slider: Handle<Slider>,
    throttle: TimerId,
    pending: Option<f32>,
    /// The daemon's lowest level; the slider stays at or above it.
    minimum: f32,
}

pub struct BrightnessSliderBuilder;

impl Build for BrightnessSliderBuilder {
    type Widget = BrightnessSlider;
}

impl Widget for BrightnessSlider {
    type Builder = BrightnessSliderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = control_style();

        let icon_sprite = load_icon(&mut s.resource_mut::<Atlas>(), svg::MEDIUM);

        let row = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .width(percent(100.0))
                    .height(percent(100.0))
                    .center()
                    .gap(percent(ICON_GAP_PERCENT)),
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
                .style(LayoutStyle::default().height(percent(100.0)))
                .size(SliderSize::Small)
                .tick_height(percent(80.0))
                .tick_gap(4.0)
                .show_value(false),
        );
        let throttle = s.add_timeout(THROTTLE, |ctx, event| {
            if let Some(value) = ctx.me().pending.take() {
                ctx.signal(SetBrightness(value));
                ctx.start_timer(event.id);
            }
        });
        s.on::<SliderChange>(slider_handle, |ctx, event| {
            let minimum = ctx.me().minimum;
            let value = event.value.max(minimum);
            if value != event.value {
                // Hold the thumb at the daemon's floor.
                let slider = ctx.me().slider;
                if let Some(mut slider) = ctx.at(slider) {
                    slider.set_value(value);
                }
            }
            let throttle = ctx.me().throttle;
            if ctx.timer_running(throttle) {
                ctx.me().pending = Some(value);
            } else {
                ctx.signal(SetBrightness(value));
                ctx.start_timer(throttle);
            }
        });

        s.on::<BrightnessChanged>(me, |ctx, event| {
            ctx.me().minimum = event.minimum;
            let slider = ctx.me().slider;
            if let Some(mut slider) = ctx.at(slider) {
                // Don't yank the thumb while the user is moving it.
                if slider.me().is_dragging() {
                    return;
                }
                slider.set_value(event.value);
            }
        });

        BrightnessSlider {
            slider: slider_handle,
            throttle,
            pending: None,
            minimum: MIN_BRIGHTNESS,
        }
    }
}
