use std::time::Duration;

use mecha_wayland::mechanix_widgets::prelude::TextContextExt;
use mecha_wayland::mechanix_widgets::{self as mw, SliderContextExt, SliderSize, slider};
use mecha_wayland::prelude::*;
use services::mpris::{MprisPlayers, PlayerId};

use crate::style::{space, text};

pub struct Progress {
    player: PlayerId,
    timer: TimerId,
    slider: Handle<mw::Slider>,
    time: Handle<mw::Text>,
    length: Option<Duration>,
    label: String,
}

pub struct ProgressBuilder {
    pub id: PlayerId,
}

impl Build for ProgressBuilder {
    type Widget = Progress;
}

impl Widget for Progress {
    type Builder = ProgressBuilder;

    fn build(b: ProgressBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .min_width(px(0.0))
            .gap(px(space::MEDIA_TITLE_TO_ARTIST.dp()));

        let (pos, length, wait) = match s.resource::<MprisPlayers>().get(&b.id) {
            Some(player) => (
                player.position_now(),
                player.track.track_length,
                player.time_to_next_second(),
            ),
            None => (Duration::ZERO, None, None),
        };
        let label = label(pos, length);

        let slider = s.spawn(
            me,
            slider()
                .size(SliderSize::XSmall)
                .range(0.0, max(length))
                .value(pos.as_secs_f32())
                .length(percent(100.0))
                .show_value(false)
                .interactive(false),
        );
        let time = s.spawn(
            me,
            mw::text(label.clone())
                .variant(text::MEDIA_ARTIST.variant)
                .color(text::MEDIA_ARTIST.color)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis),
        );

        let on_tick = |ctx: &mut Context<'_, Progress>, _: &Timeout| ctx.refresh();

        let timer = match wait {
            Some(wait) => s.set_timeout(wait, on_tick),
            None => s.add_timeout(Duration::MAX, on_tick),
        };

        Progress {
            player: b.id,
            timer,
            slider,
            time,
            length,
            label,
        }
    }
}

pub trait ProgressExt {
    fn refresh(&mut self);
}

impl ProgressExt for Context<'_, Progress> {
    fn refresh(&mut self) {
        let id = self.me().player.clone();
        let Some(player) = self.resource::<MprisPlayers>().get(&id) else {
            return;
        };
        let pos = player.position_now();
        let length = player.track.track_length;
        let wait = player.time_to_next_second();

        show(self, pos, length);

        let timer = self.me().timer;
        match wait {
            Some(wait) => self.start_timer_for(timer, wait),
            None => self.cancel_timer(timer),
        };
    }
}

fn show(ctx: &mut Context<'_, Progress>, pos: Duration, length: Option<Duration>) {
    let slider = ctx.me().slider;
    let new_length = ctx.me().length != length;
    ctx.me().length = length;
    if let Some(mut n) = ctx.at(slider) {
        if new_length {
            n.set_range(0.0, max(length));
        }
        n.set_value(pos.as_secs_f32());
    }

    let label = label(pos, length);
    if ctx.me().label == label {
        return;
    }
    let time = ctx.me().time;
    if let Some(mut n) = ctx.at(time) {
        n.set_text(label.clone());
    }
    ctx.me().label = label;
}

fn max(length: Option<Duration>) -> f32 {
    length.map_or(1.0, |length| length.as_secs_f32())
}

fn label(pos: Duration, length: Option<Duration>) -> String {
    let Some(length) = length else {
        return mmss(pos);
    };
    format!("{} / {}", mmss(pos), mmss(length))
}

fn mmss(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}
