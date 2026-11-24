use mecha_wayland::mechanix_widgets as mw;
use mecha_wayland::mechanix_widgets::prelude::TextContextExt;
use mecha_wayland::prelude::*;

use crate::style::{space, text};

pub struct TrackInfo {
    title: Handle<mw::Text>,
    artist: Handle<mw::Text>,
}

pub struct TrackInfoBuilder {
    pub title: String,
    pub artist: String,
}

impl Build for TrackInfoBuilder {
    type Widget = TrackInfo;
}

impl Widget for TrackInfo {
    type Builder = TrackInfoBuilder;

    fn build(b: TrackInfoBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .min_width(px(0.0))
            .gap(px(space::MEDIA_TITLE_TO_ARTIST.dp()));

        let title = s.spawn(me, line(b.title, text::MEDIA_TITLE));
        let artist = s.spawn(me, line(b.artist, text::MEDIA_ARTIST));

        TrackInfo { title, artist }
    }
}

pub trait TrackInfoExt {
    fn set_track(&mut self, title: String, artist: String);
}

impl TrackInfoExt for Context<'_, TrackInfo> {
    fn set_track(&mut self, title: String, artist: String) {
        let (title_node, artist_node) = (self.me().title, self.me().artist);
        if let Some(mut n) = self.at(title_node) {
            n.set_text(title);
        }
        if let Some(mut n) = self.at(artist_node) {
            n.set_text(artist);
        }
    }
}

fn line(content: String, style: crate::style::TextStyle) -> mw::TextBuilder {
    mw::text(content)
        .variant(style.variant)
        .color(style.color)
        .max_lines(1)
        .overflow(TextOverflow::Ellipsis)
        .style(LayoutStyle::default().max_width(percent(100.0)))
}
