use crate::card_drag::{CardDrag, Moved, Settle};
use crate::card_icon::{CardIconBuilder, UpdateIcon};
use crate::card_options::{self, CardOption, CardOptions, CardOptionsBuilder, CardOptionsPressed};
use crate::count_badge::{CountBadge, CountBadgeBuilder};
use crate::dbus_spec::{NOTIFICATIONS_EXPIRY, NOTIFIFICATION_DISMISSED};
use crate::group::OpenGroup;
use crate::millis::Millis;
use crate::notification_store::Entry;
use crate::notify_signal::NotificationSignal::{RemoveNotification, ShowOptions};
use crate::rich_text::RichTextBuilder;
use crate::rich_text::RichTextExt;
use crate::style::{size, space, text};
use mecha_wayland::mechanix_widgets::prelude::TextContextExt;
use mecha_wayland::mechanix_widgets::{self as mw};
use mecha_wayland::prelude::*;
use std::time::Duration;
use std::time::SystemTime;

pub struct CardBuilder {
    pub entry: Entry,
}

pub struct Card {
    pub notification_id: u32,
    drag: CardDrag,
    content: Handle<Div>,
    options: Handle<CardOptions>,
    pub badge: Handle<CountBadge>,
}

pub struct UpdateCard(pub Entry);
impl Event for UpdateCard {}

pub struct SnoozeCard(pub Millis);
impl Event for SnoozeCard {}

pub struct ToggleCard;
impl Event for ToggleCard {}

impl Build for CardBuilder {
    type Widget = Card;
}

impl Widget for Card {
    type Builder = CardBuilder;

    fn build(mut b: CardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() =
            LayoutStyle::default().relative().width(percent(100.0));
        s.component_mut::<Viewport>(me).unwrap().clip = true;

        let content = s.spawn(me, div().style(content_style(0.0)));

        let icon = s.spawn(
            content,
            CardIconBuilder {
                image: b.entry.notification.image.take(),
            },
        );
        let badge = s.spawn(icon, CountBadgeBuilder { count: 1 });
        let style = s.component::<LayoutStyle>(badge).unwrap().clone();
        *s.component_mut::<LayoutStyle>(badge).unwrap() = badge_position(style);

        let on_expire = move |ctx: &mut Context<'_, Card>, _: &Timeout| {
            ctx.signal(RemoveNotification(b.entry.id, NOTIFICATIONS_EXPIRY));
        };

        let expiry = match b.entry.notification.expire_timeout {
            Some(expiry_timeout) => s.set_timeout(expiry_timeout.duration(), on_expire),
            // we need to handle the timeout in order to trigger it
            // this is passive
            _ => s.add_timeout(Duration::MAX, on_expire),
        };

        let column = s.spawn(
            content,
            div().style(
                LayoutStyle::default()
                    .column()
                    .gap(px(space::TITLE_TO_BODY.dp()))
                    .grow(1.0)
                    .shrink(1.0)
                    .min_width(px(0.0)),
            ),
        );

        let summary = s.spawn(
            column,
            mw::text(b.entry.notification.summary)
                .variant(text::CARD_TITLE.variant)
                .color(text::CARD_TITLE.color)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis),
        );

        let rich_text = s.spawn(
            column,
            RichTextBuilder {
                runs: b.entry.notification.body.into_runs(),
            },
        );

        let time = s.spawn(
            content,
            mw::text(ago(b.entry.received_time))
                .variant(text::CARD_TIME.variant)
                .color(text::CARD_TIME.color),
        );

        let options = s.spawn(me, CardOptionsBuilder { card: me.id() });

        let id = b.entry.id;
        s.on::<CardOptionsPressed>(me, move |ctx, e| {
            if ctx.me().drag.ended_in_swipe() {
                return;
            }
            match (e.option, e.anchor) {
                (CardOption::Dismiss, _) => dismiss(ctx, id),
                (CardOption::Options, Some(anchor)) => {
                    ctx.signal(ShowOptions(id, anchor));
                }
                (CardOption::Options, None) => {}
            }
        });

        s.on::<Press>(me, |ctx, e| ctx.me().drag.press(e.contact, e.position))
            .on::<PressMoved>(me, |ctx, e| {
                let w = width(ctx);
                match ctx.me().drag.update(e.contact, e.position, w) {
                    Moved::Claim => {
                        if ctx.claim_contact(e.contact) {
                            ctx.me().drag.start_swipe();
                        }
                    }
                    Moved::Slide(offset) => set_offset(ctx, offset),
                    Moved::Ignore => {}
                }
            })
            .on::<Release>(me, move |ctx, e| {
                let w = width(ctx);
                match ctx.me().drag.release(e.contact, w) {
                    Settle::Rest(offset) => set_offset(ctx, offset),
                    Settle::Dismiss => dismiss(ctx, id),
                    Settle::Ignore => {}
                }
            })
            .on::<PressCancelled>(me, |ctx, _| {
                let offset = ctx.me().drag.cancel();
                set_offset(ctx, offset);
            })
            .on::<Clicked>(content.id(), |ctx, _| {
                if ctx.me().drag.ended_in_swipe() {
                    return;
                }
                if ctx.me().drag.offset() < 0.0 {
                    set_offset(ctx, 0.0);
                    return;
                }
                let me = ctx.handle();
                if let Some(cards) = ctx.tree().parent(me) {
                    ctx.emit(OpenGroup, cards);
                }
            });

        s.on::<UpdateCard>(me, move |ctx, e| {
            let entry = &e.0;
            ctx.at(summary)
                .unwrap()
                .set_text(entry.notification.summary.clone());

            ctx.at(rich_text)
                .unwrap()
                .set_run(entry.notification.body.clone().into_runs());
            // TODO: update the body's runs
            ctx.at(time).unwrap().set_text(ago(entry.received_time));

            ctx.emit(UpdateIcon(entry.notification.image.clone()), icon);

            // whenever a replacement occurs, the expiry timer would start again,
            match entry.notification.expire_timeout {
                Some(expiry_timeout) => ctx.start_timer_for(expiry, expiry_timeout.duration()),
                // the replacement never expires
                _ => ctx.cancel_timer(expiry),
            };
        });

        s.on::<ToggleCard>(me, |ctx, _| {
            let shown = ctx.style().display != Display::Hidden;
            ctx.set_display(if shown {
                Display::Hidden
            } else {
                Display::Flex
            });
        });

        let snooze = s.add_timeout(Duration::MAX, |ctx, _| {
            ctx.component_mut::<LayoutStyle>().unwrap().display = Display::Flex;
        });

        s.on::<SnoozeCard>(me, move |ctx, e| {
            ctx.component_mut::<LayoutStyle>().unwrap().display = Display::Hidden;
            ctx.start_timer_for(snooze, e.0.duration());
            // come back closed
            set_offset(ctx, 0.0);
        });
        Card {
            notification_id: b.entry.id,
            drag: CardDrag::default(),
            content,
            options,
            badge,
        }
    }
}

// TODO: It needs to update each minute
// currently until a new notification arrives,
// the card won't update the time
fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .unwrap_or_default()
        .as_secs();
    match secs {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..86_400 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

fn badge_position(style: LayoutStyle) -> LayoutStyle {
    let half = -size::COUNT_BADGE / 2.0;
    style
        .absolute()
        .inset(Insets::new(px(half), px(half), auto(), auto()))
}

fn content_style(offset: f32) -> LayoutStyle {
    LayoutStyle::default()
        .row()
        .relative()
        .inset(Insets::new(auto(), auto(), auto(), px(offset)))
        .align_items(Align::Center)
        .width(percent(100.0))
        .min_height(px(size::CARD_MIN_HEIGHT))
        .padding(Insets::all(px(space::CARD_PAD.dp())))
        .gap(px(space::ICON_TO_TEXT.dp()))
}

fn set_offset(ctx: &mut Context<'_, Card>, offset: f32) {
    ctx.me().drag.set_offset(offset);
    let (content, options) = (ctx.me().content, ctx.me().options);
    if let Some(mut n) = ctx.at(content) {
        restyle(&mut n, content_style(offset));
    }
    if let Some(mut n) = ctx.at(options) {
        restyle(&mut n, card_options::style(offset));
    }
}

fn restyle<W: Widget>(n: &mut Context<'_, W>, style: LayoutStyle) {
    if *n.component::<LayoutStyle>().unwrap() != style {
        *n.component_mut::<LayoutStyle>().unwrap() = style;
    }
}

fn width(ctx: &mut Context<'_, Card>) -> f32 {
    ctx.component::<ComputedLayout>()
        .map_or(0.0, |l| l.rect.width())
}

fn dismiss(ctx: &mut Context<Card>, id: u32) {
    ctx.signal(RemoveNotification(id, NOTIFIFICATION_DISMISSED));
}
