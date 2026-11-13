use crate::count_badge::{CountBadge, SetCount};
use crate::notify_signal::NotificationSignal::CloseGroupNotifications;
use crate::{
    card::{Card, CardBuilder, ToggleCard},
    notification_store::Entry,
};
use mecha_wayland::mechanix_widgets::{self as mw, button};
use mecha_wayland::prelude::*;

use crate::style::{color, shape, size, space, text};

const CARET_UP: &[u8] = include_bytes!("../assets/caret-up.svg");

pub struct ToggleGroup;
impl Event for ToggleGroup {}

pub struct OpenGroup;
impl Event for OpenGroup {}

pub struct AddCard(pub Entry);
impl Event for AddCard {}

pub struct RemoveCard(pub NodeId);
impl Event for RemoveCard {}

pub struct CardGroupBuilder {
    pub app_name: String,
    pub entry: Entry,
}

pub struct CardGroup {
    pub app_name: String,
    pub cards: Handle<Div>,
    pub expanded: bool,
}

impl Build for CardGroupBuilder {
    type Widget = CardGroup;
}

impl Widget for CardGroup {
    type Builder = CardGroupBuilder;

    fn build(b: CardGroupBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .gap(px(space::CARD_GAP.dp()));

        let header = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .gap(px(space::ICON_TO_TEXT.dp()))
                    .padding(Insets::all(px(space::CARD_PAD.dp())))
                    .display(Display::Hidden),
            ),
        );
        let up = s
            .resource_mut::<Atlas>()
            .insert_svg(CARET_UP, size::CHEVRON as u32)
            .expect("embedded caret-up glyph is a valid SVG");

        let color = s.color(text::GROUP_HEADER.color);
        s.spawn(
            header,
            icon(up)
                .color(color)
                .style(LayoutStyle::default().shrink(0.0)),
        );
        s.spawn(
            header,
            mw::text(b.app_name.clone())
                .variant(text::GROUP_HEADER.variant)
                .color(text::GROUP_HEADER.color)
                .max_lines(1)
                .style(
                    LayoutStyle::default()
                        .grow(1.0)
                        .shrink(1.0)
                        .min_width(px(0.0)),
                ),
        );
        let clear_btn = s.spawn(
            header,
            button()
                .variant(mw::ButtonVariant::Tonal)
                .size(mw::ButtonSize::Small)
                .background(color::CHIP)
                .text_color(text::CHIP.color)
                .shape(shape::PILL)
                .height(px(size::CHIP_HEIGHT))
                .padding(Insets::symmetric(px(space::CHIP_X.dp()), px(0.0)))
                .label("CLEAR"),
        );

        let cards = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column_reverse()
                    .width(percent(100.0))
                    .gap(px(space::CARD_GAP.dp())),
            ),
        );
        s.spawn(cards, CardBuilder { entry: b.entry });

        s.on::<ToggleGroup>(me, move |ctx, _| toggle_group(ctx, header));

        s.on::<AddCard>(me, |ctx, e| add_card(ctx, e.0.clone()));
        s.on::<RemoveCard>(me, |ctx, e| remove_card(ctx, e.0));

        s.on::<OpenGroup>(cards, move |ctx, _| {
            let count = ctx.tree().children(cards).map_or(0, <[NodeId]>::len);
            if !ctx.me().expanded && count > 1 {
                ctx.emit(ToggleGroup, me);
            }
        });

        let app_name = b.app_name.clone();
        s.on::<Clicked>(header, move |ctx, _| ctx.emit(ToggleGroup, me));
        s.on::<Clicked>(clear_btn, move |ctx, _| {
            ctx.signal(CloseGroupNotifications(app_name.clone()))
        });

        CardGroup {
            app_name: b.app_name,
            cards,
            expanded: false,
        }
    }
}

fn add_card(ctx: &mut Context<'_, CardGroup>, entry: Entry) {
    let (cards, expanded) = (ctx.me().cards, ctx.me().expanded);
    let previous = ctx.tree().children(cards).and_then(|c| c.last().copied());
    if let (false, Some(previous)) = (expanded, previous) {
        ctx.emit(ToggleCard, previous);
    }
    let card = ctx.spawn(cards, CardBuilder { entry });

    if let Some(badge) = previous.and_then(|p| badge_of(ctx, p)) {
        ctx.emit(SetCount(1), badge);
    }
    if !expanded {
        let count = ctx.tree().children(cards).map_or(0, <[NodeId]>::len);
        if let Some(badge) = badge_of(ctx, card.id()) {
            ctx.emit(SetCount(count), badge);
        }
    }
}

fn toggle_group(ctx: &mut Context<'_, CardGroup>, header: Handle<Div>) {
    let cards = ctx.me().cards;
    let all = ctx
        .tree()
        .children(cards)
        .map(<[NodeId]>::to_vec)
        .unwrap_or_default();
    let Some((&newest, older)) = all.split_last() else {
        return;
    };
    for &card in older {
        ctx.emit(ToggleCard, card);
    }

    let expanded = !ctx.me().expanded;
    ctx.me().expanded = expanded;

    if let Some(mut header) = ctx.at(header) {
        header.set_display(if expanded {
            Display::Flex
        } else {
            Display::Hidden
        });
    }
    let count = if expanded { 1 } else { all.len() };
    if let Some(badge) = badge_of(ctx, newest) {
        ctx.emit(SetCount(count), badge);
    }
}

fn remove_card(ctx: &mut Context<'_, CardGroup>, card: NodeId) {
    let (cards, expanded) = (ctx.me().cards, ctx.me().expanded);
    let was_shown = !expanded && newest_card(ctx, cards) == Some(card);

    ctx.remove(card);

    let count = card_count(ctx, cards);
    let group = ctx.handle();

    if count == 0 {
        ctx.remove(group);
        return;
    }

    if expanded {
        if count == 1 {
            ctx.emit(ToggleGroup, group);
        }
        return;
    }

    let Some(newest) = newest_card(ctx, cards) else {
        return;
    };
    if was_shown {
        ctx.emit(ToggleCard, newest);
    }
    if let Some(badge) = badge_of(ctx, newest) {
        ctx.emit(SetCount(count), badge);
    }
}

fn newest_card(ctx: &Context<'_, CardGroup>, cards: Handle<Div>) -> Option<NodeId> {
    ctx.tree().children(cards).and_then(|c| c.last().copied())
}

fn card_count(ctx: &Context<'_, CardGroup>, cards: Handle<Div>) -> usize {
    ctx.tree().children(cards).map_or(0, <[NodeId]>::len)
}

fn badge_of(ctx: &Context<'_, CardGroup>, card: NodeId) -> Option<Handle<CountBadge>> {
    ctx.widget::<Card>(card).map(|c| c.badge)
}
