use mecha_wayland::prelude::*;

use crate::style::space;

pub struct CardListBuilder;

pub struct CardList;

impl Build for CardListBuilder {
    type Widget = CardList;
}

impl Widget for CardList {
    type Builder = CardListBuilder;

    fn build(_: CardListBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .padding(Insets::new(
                px(0.0),
                px(space::LIST_X.dp()),
                px(0.0),
                px(space::LIST_X.dp()),
            ))
            .gap(px(space::CARD_GAP.dp()));
        CardList
    }
}
