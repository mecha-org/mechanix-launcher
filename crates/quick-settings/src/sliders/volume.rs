use mecha_pulseaudio::*;
use mecha_wayland::mechanix_widgets::prelude::{SliderChange, SliderContextExt, slider};
use mecha_wayland::prelude::*;

use super::SLIDER_HEIGHT;
use crate::util::load_icon;

mod svg {
    pub const HIGH: &[u8] = include_bytes!("../../assets/speaker/speaker-high.svg");
    pub const LOW: &[u8] = include_bytes!("../../assets/speaker/speaker-low.svg");
    pub const MUTE: &[u8] = include_bytes!("../../assets/speaker/speaker-mute.svg");
}

const LEVEL_PERCENT: f32 = 70.0; // initial value

const ICON_SIZE: f32 = 24.0;
const SLIDER_LENGTH_PERCENT: f32 = 80.0;

const HIGH_LEVEL: u32 = 65; // after what value we use the high icon

pub struct VolumeSlider;

pub struct VolumeSliderBuilder;

impl Build for VolumeSliderBuilder {
    type Widget = VolumeSlider;
}

impl Widget for VolumeSlider {
    type Builder = VolumeSliderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .width(percent(100.0))
            .height(px(SLIDER_HEIGHT))
            .shrink(0.0);

        let high = load_icon(&mut s.resource_mut::<Atlas>(), svg::HIGH);
        let low = load_icon(&mut s.resource_mut::<Atlas>(), svg::LOW);
        let mute = load_icon(&mut s.resource_mut::<Atlas>(), svg::MUTE);

        let row = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .width(percent(100.0))
                    .center()
                    .gap(px(Spacing::Space300.dp())),
            ),
        );

        let icon_handle = s.spawn(
            row,
            icon(high) // default icon
                .style(LayoutStyle::default().shrink(0.0))
                .size(Size::new(ICON_SIZE, ICON_SIZE))
                .color(s.color(ColorRole::OnSurface)),
        );

        let slider_handle = s.spawn(
            row,
            slider()
                .value(LEVEL_PERCENT)
                .range(0.0, 100.0)
                .length(percent(SLIDER_LENGTH_PERCENT))
                .tick_height(SLIDER_HEIGHT)
                .tick_gap(4.0),
        );

        s.on::<Clicked>(icon_handle, |ctx, _| {
            if let VolumeState::Ready { muted, .. } = *ctx.resource::<VolumeState>() {
                ctx.signal(SetMuted(!muted))
            }
        });

        s.on::<SliderChange>(slider_handle, |ctx, e| {
            println!("[volume] slider changed: value = {:.2}", e.value);

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
                if let Some(mut icon) = ctx.at(icon_handle) {
                    let sprite = if muted {
                        mute
                    } else if volume_percent > HIGH_LEVEL {
                        high
                    } else {
                        low
                    };
                    icon.set_sprite(sprite);
                    // set_sprite resets to the sprite's natural size
                    icon.set_size(Size::new(ICON_SIZE, ICON_SIZE));
                }
            }
        });

        VolumeSlider
    }
}
