use mecha_wayland::prelude::*;

use crate::group::{AddCard, CardGroup, CardGroupBuilder};
use crate::notification_store::Entry;
use crate::style::space;

pub struct CardListBuilder;

pub struct CardList;

pub struct AddEntry(pub Entry);
impl Event for AddEntry {}

pub struct ClearGroup(pub String);
impl Event for ClearGroup {}

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

        s.on::<AddEntry>(me, |ctx, e| add_entry(ctx, e.0.clone()));
        s.on::<ClearGroup>(me, |ctx, e| clear_group(ctx, &e.0));

        CardList
    }
}

fn add_entry(ctx: &mut Context<'_, CardList>, entry: Entry) {
    let me = ctx.handle();
    match find_group(ctx, &entry.notification.app_name) {
        Some(group) => ctx.emit(AddCard(entry), group),
        None => {
            ctx.spawn(
                me,
                CardGroupBuilder {
                    app_name: entry.notification.app_name.clone(),
                    entry,
                },
            );
        }
    }
}

fn clear_group(ctx: &mut Context<'_, CardList>, app_name: &str) {
    if let Some(group) = find_group(ctx, app_name) {
        ctx.remove(group);
    }
}

fn find_group(ctx: &Context<'_, CardList>, app_name: &str) -> Option<NodeId> {
    let groups = ctx.tree().children(ctx.handle())?;
    groups.iter().copied().find(|&g| {
        ctx.widget::<CardGroup>(g)
            .is_some_and(|g| g.app_name == app_name)
    })
}
