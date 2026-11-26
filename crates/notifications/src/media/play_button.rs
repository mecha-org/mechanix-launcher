use mecha_wayland::mechanix_widgets::{self as mw, ButtonContextExt, button};
use mecha_wayland::prelude::*;
use services::mpris::{MediaCommand, PlaybackStatus, PlayerId};

use crate::style::{color, size};

const PLAY: &[u8] = include_bytes!("../../assets/play.svg");
const PAUSE: &[u8] = include_bytes!("../../assets/pause.svg");

pub struct PlayButton {
    button: Handle<mw::Button>,
    play: SpriteId,
    pause: SpriteId,
    player: PlayerId,
}

pub struct PlayButtonBuilder {
    pub id: PlayerId,
    pub status: PlaybackStatus,
}

impl Build for PlayButtonBuilder {
    type Widget = PlayButton;
}

impl Widget for PlayButton {
    type Builder = PlayButtonBuilder;

    fn build(b: PlayButtonBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().shrink(0.0);

        let icon_size = mw::ButtonSize::Large.icon_size() as u32;
        let mut atlas = s.resource_mut::<Atlas>();
        let play = atlas
            .insert_svg(PLAY, icon_size)
            .expect("embedded play glyph is a valid SVG");
        let pause = atlas
            .insert_svg(PAUSE, icon_size)
            .expect("embedded pause glyph is a valid SVG");
        drop(atlas);

        let side = px(size::MEDIA_BUTTON);
        let button = s.spawn(
            me,
            button()
                .variant(mw::ButtonVariant::Tonal)
                .size(mw::ButtonSize::Large)
                .shape(Shape::Full)
                .background(color::MEDIA_BUTTON)
                .border_color(color::MEDIA_BUTTON_BORDER)
                .border_widths(Insets::all(1.0))
                .text_color(color::MEDIA_BUTTON_ICON)
                .width(side)
                .height(side)
                .padding(Insets::all(px(0.0)))
                .icon(icon_for(b.status, play, pause)),
        );

        s.on::<Clicked>(button, move |ctx, _| {
            let id = ctx.me().player.clone();
            ctx.signal(MediaCommand::PlayPause(id));
        });

        PlayButton {
            button,
            play,
            pause,
            player: b.id,
        }
    }
}

pub trait PlayButtonExt {
    fn set_status(&mut self, status: PlaybackStatus);
}

impl PlayButtonExt for Context<'_, PlayButton> {
    fn set_status(&mut self, status: PlaybackStatus) {
        let me = self.me();
        let icon = icon_for(status, me.play, me.pause);
        let button = me.button;
        if let Some(mut n) = self.at(button) {
            n.set_icon(icon);
        }
    }
}

fn icon_for(status: PlaybackStatus, play: SpriteId, pause: SpriteId) -> SpriteId {
    match status {
        PlaybackStatus::Playing => pause,
        _ => play,
    }
}
