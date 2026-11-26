//! One media card per player, keyed by its id so updates touch only that card.
use std::collections::HashMap;

use mecha_wayland::prelude::*;
use services::mpris::{MprisPlayers, PlayerId};

use super::media_card::{MediaCard, MediaCardBuilder, PlayerUpdated};
use crate::style::space;

pub struct ShowPlayer(pub PlayerId);
impl Event for ShowPlayer {}

pub struct RemovePlayer(pub PlayerId);
impl Event for RemovePlayer {}

pub struct MediaList {
    cards: HashMap<PlayerId, Handle<MediaCard>>,
}

pub struct MediaListBuilder;

impl Build for MediaListBuilder {
    type Widget = MediaList;
}

impl Widget for MediaList {
    type Builder = MediaListBuilder;

    fn build(_: MediaListBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .shrink(0.0)
            .gap(px(space::MEDIA_LIST_GAP.dp()));

        s.on::<ShowPlayer>(me, move |ctx, e| {
            if ctx.me().cards.contains_key(&e.0) {
                return;
            }
            let Some(builder) = ctx
                .resource::<MprisPlayers>()
                .get(&e.0)
                .map(MediaCardBuilder::from)
            else {
                return;
            };
            let card = ctx.spawn(me, builder);
            ctx.me().cards.insert(e.0.clone(), card);
        });

        s.on::<PlayerUpdated>(me, move |ctx, e| {
            let Some(&card) = ctx.me().cards.get(&e.0) else {
                return;
            };
            ctx.emit(PlayerUpdated(e.0.clone()), card);
        });

        s.on::<RemovePlayer>(me, move |ctx, e| {
            let Some(card) = ctx.me().cards.remove(&e.0) else {
                return;
            };
            ctx.remove(card);
        });

        MediaList {
            cards: HashMap::new(),
        }
    }
}
