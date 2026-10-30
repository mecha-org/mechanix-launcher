use mecha_wayland::prelude::*;

pub struct Toggle;
impl Event for Toggle {}

pub fn toggle_display<W: Widget>(ctx: &mut Context<'_, W>, _: &Toggle) {
    let mut style = ctx.component_mut::<LayoutStyle>().unwrap();
    style.display = match style.display {
        Display::Hidden => Display::Flex,
        _ => Display::Hidden,
    };
}
