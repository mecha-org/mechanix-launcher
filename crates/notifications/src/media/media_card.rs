use mecha_wayland::prelude::*;
use services::mpris::{MprisPlayers, PlayerId};

use super::track_info::{TrackInfoBuilder, TrackInfoExt};
use crate::style::space;

pub struct ShowPlayer(pub PlayerId);
impl Event for ShowPlayer {}

pub struct MediaCard;

pub struct MediaCardBuilder;

impl Build for MediaCardBuilder {
    type Widget = MediaCard;
}

impl Widget for MediaCard {
    type Builder = MediaCardBuilder;

    fn build(_: MediaCardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let pad = px(space::MEDIA_PAD.dp());
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .width(percent(100.0))
            .shrink(0.0)
            .padding(Insets::new(pad, pad, pad, pad))
            .gap(px(space::MEDIA_GAP.dp()));
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceContainerLow)));

        let middle = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column()
                    .grow(1.0)
                    .shrink(1.0)
                    .min_width(px(0.0)),
            ),
        );

        let info = s.spawn(
            middle,
            TrackInfoBuilder {
                title: "Faisal (envelopes mix)".into(),
                artist: "Fred again..".into(),
            },
        );

        s.on::<ShowPlayer>(me, move |ctx, e| {
            let Some((title, artist)) = track_text(ctx.resource(), &e.0) else {
                return;
            };
            if let Some(mut n) = ctx.at(info) {
                n.set_track(title, artist);
            }
        });

        MediaCard
    }
}

fn track_text(players: &MprisPlayers, id: &PlayerId) -> Option<(String, String)> {
    let track = &players.get(id)?.track;
    let title = track.title.clone().unwrap_or_default();
    let artist = track.artists.join(", ");
    Some((title, artist))
}
