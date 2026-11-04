use mecha_pulseaudio::*;
use mecha_wayland::mechanix_widgets::prelude::{SliderChange, SliderContextExt, slider};
use mecha_wayland::prelude::*;

use crate::util::load_icon;

mod svg {
    pub const HIGH: &[u8] = include_bytes!("../../assets/speaker/speaker-high.svg");
    pub const LOW: &[u8] = include_bytes!("../../assets/speaker/speaker-low.svg");
    pub const MUTE: &[u8] = include_bytes!("../../assets/speaker/speaker-mute.svg");
}

const LEVEL_PERCENT: f32 = 70.0; // initial value
const PADDING_VERTICAL: f32 = 12.0;
const PADDING_HORIZONTAL: f32 = 32.0;

const HIGH_LEVEL: u32 = 65; // after what value we use the high icon

pub struct VolumeSlider;

pub struct VolumeSliderBuilder;

impl Build for VolumeSliderBuilder {
    type Widget = VolumeSlider;
}

impl Widget for VolumeSlider {
    type Builder = VolumeSliderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let high = load_icon(&mut s.resource_mut::<Atlas>(), svg::HIGH);
        let low = load_icon(&mut s.resource_mut::<Atlas>(), svg::LOW);
        let mute = load_icon(&mut s.resource_mut::<Atlas>(), svg::MUTE);

        let row = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .center()
                    .gap(px(Spacing::Space200.dp()))
                    .padding(Insets::symmetric(
                        px(PADDING_HORIZONTAL),
                        px(PADDING_VERTICAL),
                    )),
            ),
        );

        let icon_handle = s.spawn(
            row,
            icon(high) // default icon
                .style(LayoutStyle::default().shrink(0.0))
                .color(s.color(ColorRole::OnSurface)),
        );

        let slider_handle = s.spawn(
            row,
            slider()
                .value(LEVEL_PERCENT)
                .range(0.0, 100.0)
                .steps(68)
                .tick_height(40.0)
                .tick_gap(4.0),
        );

        s.on::<SliderChange>(slider_handle, |ctx, e| {
            println!(
                "[volume] slider changed: value = {:.2}, step_index = {}",
                e.value, e.step_index
            );

            ctx.signal(SetVolume(if e.value > 100.0 {
                100
            } else if e.value < 0.0 {
                0
            } else {
                e.value as u32
            }));
        });

        s.on::<VolumeStateEvent>(slider_handle, move |ctx, e| {
            if let VolumeState::Ready {
                volume_percent,
                muted,
            } = e.0
            {
                println!(
                    "[volume] state changed to: {} muted: {}",
                    volume_percent, muted
                );

                if let Some(mut slider) = ctx.at(slider_handle) {
                    slider.set_value(volume_percent.min(100) as f32);
                };

                // TODO: check if we're already muted and if our icon is
                // already set to high or low, saves resetting it
                if let Some(mut icon_handler) = ctx.at(icon_handle) {
                    if muted {
                        icon_handler.set_sprite(mute);
                    } else if volume_percent > HIGH_LEVEL {
                        icon_handler.set_sprite(high);
                    } else {
                        icon_handler.set_sprite(low);
                    }
                }
            }
        });

        VolumeSlider
    }
}
