use crate::notify_signal::NotificationSignal::SnoozeNotifiation;
use crate::popup::{HidePopup, PopupBuilder, ShowPopup};
use crate::snooze_controls::SnoozeControls;
use crate::style::{color, size, space, text};
use mecha_wayland::mechanix_widgets as mw;
use mecha_wayland::prelude::*;

const MENU_ITEMS: [(&str, SnoozeControls); 2] = [
    ("Remind in 15min", SnoozeControls::Mins(15)),
    ("Mute for 1 hr", SnoozeControls::Hours(1)),
];

const MENU_SIZE: Size = Size {
    width: size::MENU_WIDTH,
    height: 2.0 * space::MENU_Y.dp()
        + MENU_ITEMS.len() as f32 * (size::MENU_ROW_HEIGHT + 2.0 * space::MENU_Y.dp()),
};

pub struct ShowOptionsMenu {
    pub notification_id: u32,
    pub anchor: Rect,
}
impl Event for ShowOptionsMenu {}

pub struct OptionsMenuBuilder;

pub struct OptionsMenu {
    popup: NodeId,
    notification_id: Option<u32>,
}

impl Build for OptionsMenuBuilder {
    type Widget = OptionsMenu;
}

impl Widget for OptionsMenu {
    type Builder = OptionsMenuBuilder;

    fn build(_: OptionsMenuBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .absolute()
            .inset(Insets::all(px(0.0)));

        let popup = s.spawn(me, PopupBuilder).id();
        let list = s.spawn(
            popup,
            div().background(s.color(color::MENU)).style(
                LayoutStyle::default()
                    .width(px(size::MENU_WIDTH))
                    .column()
                    .padding(Insets::new(
                        px(space::MENU_Y.dp()),
                        px(0.0),
                        px(space::MENU_Y.dp()),
                        px(0.0),
                    )),
            ),
        );

        for (label, choice) in MENU_ITEMS {
            let row = spawn_row(s, list, label);
            s.on::<Press>(row, |ctx, e| {
                ctx.claim_contact(e.contact);
            })
            .on::<Clicked>(row, move |ctx, _| {
                if let Some(id) = ctx.me().notification_id {
                    ctx.signal(SnoozeNotifiation(id, choice.millis().0));
                }
                let popup = ctx.me().popup;
                ctx.emit(HidePopup, popup);
            });
        }

        s.on::<ShowOptionsMenu>(me, |ctx, e| {
            let Some(area) = ctx.component::<ComputedLayout>().map(|l| l.rect) else {
                return;
            };
            let show = ShowPopup {
                anchor: e.anchor,
                area,
                size: MENU_SIZE,
                gap: size::MENU_BELOW_ANCHOR,
            };
            ctx.me().notification_id = Some(e.notification_id);
            let popup = ctx.me().popup;
            ctx.emit(show, popup);
        });

        OptionsMenu {
            popup,
            notification_id: None,
        }
    }
}

fn spawn_row(s: &mut Spawner<'_, OptionsMenu>, list: Handle<Div>, label: &str) -> Handle<Div> {
    let item = s.spawn(
        list,
        div().style(
            LayoutStyle::default()
                .width(percent(100.0))
                .padding(Insets::new(
                    px(space::MENU_Y.dp()),
                    px(space::MENU_ITEM_X.dp()),
                    px(space::MENU_Y.dp()),
                    px(space::MENU_ITEM_X.dp()),
                )),
        ),
    );
    let row = s.spawn(
        item,
        div().style(
            LayoutStyle::default()
                .row()
                .align_items(Align::Center)
                .width(percent(100.0))
                .height(px(size::MENU_ROW_HEIGHT))
                .padding(Insets::all(px(space::MENU_ROW_PAD.dp()))),
        ),
    );
    s.spawn(
        row,
        mw::text(label)
            .variant(text::MENU_ITEM.variant)
            .color(text::MENU_ITEM.color)
            .max_lines(1)
            .overflow(TextOverflow::Ellipsis),
    );
    item
}
