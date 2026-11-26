use mecha_wayland::prelude::*;
use services::mpris::{MediaPlayer, MprisPlayers, PlaybackStatus, PlayerId, Track};

use super::album_art::{AlbumArtBuilder, AlbumArtExt};
use super::play_button::{PlayButtonBuilder, PlayButtonExt};
use super::track_info::{TrackInfoBuilder, TrackInfoExt};
use crate::style::space;

pub struct PlayerUpdated(pub PlayerId);
impl Event for PlayerUpdated {}

pub struct MediaCard;

pub struct MediaCardBuilder {
    pub id: PlayerId,
    pub title: String,
    pub artist: String,
    pub art_url: Option<String>,
    pub status: PlaybackStatus,
}

impl From<&MediaPlayer> for MediaCardBuilder {
    fn from(player: &MediaPlayer) -> Self {
        let (title, artist) = track_text(&player.track);
        Self {
            id: player.id.clone(),
            title,
            artist,
            art_url: player.track.art_url.clone(),
            status: player.status,
        }
    }
}

impl Build for MediaCardBuilder {
    type Widget = MediaCard;
}

impl Widget for MediaCard {
    type Builder = MediaCardBuilder;

    fn build(b: MediaCardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let (x, y) = (px(space::MEDIA_PAD_X.dp()), px(space::MEDIA_PAD_Y.dp()));
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .width(percent(100.0))
            .shrink(0.0)
            .padding(Insets::new(y, x, y, x))
            .gap(px(space::MEDIA_GAP.dp()));
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceContainerLow)));

        let art = s.spawn(me, AlbumArtBuilder { url: b.art_url });

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
                title: b.title,
                artist: b.artist,
            },
        );

        let play = s.spawn(
            me,
            PlayButtonBuilder {
                id: b.id,
                status: b.status,
            },
        );

        s.on::<PlayerUpdated>(me, move |ctx, e| {
            let Some(player) = ctx.resource::<MprisPlayers>().get(&e.0) else {
                return;
            };
            let (title, artist) = track_text(&player.track);
            let art_url = player.track.art_url.clone();
            let status = player.status;

            if let Some(mut n) = ctx.at(art) {
                n.set_art(art_url);
            }
            if let Some(mut n) = ctx.at(info) {
                n.set_track(title, artist);
            }
            if let Some(mut n) = ctx.at(play) {
                n.set_status(status);
            }
        });

        MediaCard
    }
}

fn track_text(track: &Track) -> (String, String) {
    let title = track.title.clone().unwrap_or_default();
    let artist = track.artists.join(", ");
    (title, artist)
}
