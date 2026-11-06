use mecha_wayland::prelude::*;

pub struct ShowPopup {
    pub anchor: Rect,
    pub area: Rect,
    pub size: Size,
    pub gap: f32,
}

impl Event for ShowPopup {}

pub struct HidePopup;
impl Event for HidePopup {}

pub struct PopupBuilder;
pub struct Popup;

impl Build for PopupBuilder {
    type Widget = Popup;
}

impl Widget for Popup {
    type Builder = PopupBuilder;

    fn build(_: PopupBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = style(0.0, 0.0).hidden();

        s.on::<ShowPopup>(me, |ctx, e| {
            let (top, right) = place(e);
            ctx.set_style(style(top, right));
        })
        .on::<HidePopup>(me, |ctx, _| {
            ctx.set_display(Display::Hidden);
        })
        .on::<Press>(me, |ctx, e| {
            ctx.claim_contact(e.contact);
        })
        .on::<Clicked>(me, |ctx, _| {
            ctx.set_display(Display::Hidden);
        });

        Popup
    }
}

fn place(e: &ShowPopup) -> (f32, f32) {
    let (anchor, area, size) = (e.anchor, e.area, e.size);

    let below = anchor.bottom() + e.gap;
    let above = anchor.y() - e.gap - size.height;
    let fits_below = below + size.height <= area.bottom();
    let fits_above = above >= area.y();
    let y = if !fits_below && fits_above {
        above
    } else {
        below
    };
    let top = (y - area.y()).clamp(0.0, (area.height() - size.height).max(0.0));

    let right = (area.right() - anchor.right()).clamp(0.0, (area.width() - size.width).max(0.0));

    (top, right)
}

fn style(top: f32, right: f32) -> LayoutStyle {
    LayoutStyle::default()
        .absolute()
        .inset(Insets::all(px(0.0)))
        .column()
        .align_items(Align::End)
        .padding(Insets::new(px(top), px(right), px(0.0), px(0.0)))
}
