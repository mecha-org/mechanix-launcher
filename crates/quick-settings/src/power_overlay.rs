//! Full-panel confirmation for shutdown and restart.
use mecha_wayland::mechanix_widgets::prelude::{
    Button, ButtonContextExt, ButtonSize, ButtonVariant, Text, TextContextExt, button, text,
};
use mecha_wayland::prelude::*;

use crate::quick_settings::QuickSettings;
use crate::util::load_icon_sized;
use services::{PowerOff, Restart};

pub const POWER_ICON: &[u8] = include_bytes!("../assets/power.svg");
const RESTART_ICON: &[u8] = include_bytes!("../assets/rotate.svg");

pub struct OpenPowerOverlay;
impl Event for OpenPowerOverlay {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    PowerOff,
    Restart,
}

pub struct PowerOverlay {
    content: Handle<QuickSettings>,
    confirm: Handle<Button>,
    label: Handle<Text>,
    power_icon: SpriteId,
    restart_icon: SpriteId,
    action: Action,
    open: bool,
}

pub struct PowerOverlayBuilder {
    pub root: NodeId,
    pub content: Handle<QuickSettings>,
}

impl Build for PowerOverlayBuilder {
    type Widget = PowerOverlay;
}

fn set_open(ctx: &mut Context<'_, PowerOverlay>, open: bool) {
    let content = ctx.me().content;
    ctx.me().open = open;
    ctx.set_display(if open { Display::Flex } else { Display::Hidden });
    ctx.at(content)
        .unwrap()
        .set_display(if open { Display::Hidden } else { Display::Flex });
}

impl Widget for PowerOverlay {
    type Builder = PowerOverlayBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .hidden()
            .absolute()
            .inset(Insets::all(px(0.0)))
            .size(percent(100.0), percent(100.0))
            .column()
            .padding(Insets::all(px(24.0)));
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceDim)));

        let top = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .justify(Justify::SpaceBetween)
                    .align_items(Align::Center)
                    .width(percent(100.0)),
            ),
        );
        let restart = s.spawn(
            top,
            button()
                .variant(ButtonVariant::Text)
                .text_color(ColorRole::OnSurface)
                .label("RESTART")
                .hover_background(Color::TRANSPARENT)
                .pressed_background(Color::TRANSPARENT)
                .height(px(40.0)),
        );
        let close = s.spawn(
            top,
            button()
                .variant(ButtonVariant::Text)
                .text_color(ColorRole::OnSurface)
                .label("CLOSE")
                .hover_background(Color::TRANSPARENT)
                .pressed_background(Color::TRANSPARENT)
                .height(px(40.0)),
        );

        let center = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .grow(1.0)
                    .width(percent(100.0))
                    .column()
                    .center()
                    .gap(px(20.0)),
            ),
        );
        let icon_size = ButtonSize::XLarge.icon_size() as u32;
        let power_icon = load_icon_sized(&mut s.resource_mut::<Atlas>(), POWER_ICON, icon_size);
        let restart_icon = load_icon_sized(&mut s.resource_mut::<Atlas>(), RESTART_ICON, icon_size);
        let confirm = s.spawn(
            center,
            button()
                .variant(ButtonVariant::Outlined)
                .size(ButtonSize::XLarge)
                .icon(power_icon)
                .radius(Corners::all(64.))
                .width(px(128.0))
                .height(px(128.0))
                .padding(Insets::all(px(0.))),
        );
        let label = s.spawn(
            center,
            text("Power off")
                .variant(TextVariant::TitleLarge)
                .color(ColorRole::OnSurface),
        );

        s.on::<OpenPowerOverlay>(b.root, |ctx, _| {
            ctx.me().action = Action::PowerOff;
            let label = ctx.me().label;
            ctx.at(label).unwrap().set_text("Power off");
            let (confirm, icon) = (ctx.me().confirm, ctx.me().power_icon);
            ctx.at(confirm).unwrap().set_icon(icon);
            set_open(ctx, true);
        });
        s.on::<Clicked>(restart, |ctx, _| {
            if ctx.me().open {
                ctx.me().action = Action::Restart;
                let label = ctx.me().label;
                ctx.at(label).unwrap().set_text("Restart");
                let (confirm, icon) = (ctx.me().confirm, ctx.me().restart_icon);
                ctx.at(confirm).unwrap().set_icon(icon);
            }
        });
        s.on::<Clicked>(close, |ctx, _| {
            set_open(ctx, false);
        });
        s.on::<Clicked>(confirm, |ctx, _| {
            if !ctx.me().open {
                return;
            }
            let action = ctx.me().action;
            set_open(ctx, false);
            match action {
                Action::PowerOff => ctx.signal(PowerOff),
                Action::Restart => ctx.signal(Restart),
            }
        });

        PowerOverlay {
            content: b.content,
            confirm,
            label,
            power_icon,
            restart_icon,
            action: Action::PowerOff,
            open: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottom_bar::BottomBarBuilder;
    use crate::header::HeaderBuilder;
    use crate::quick_settings::QuickSettingsBuilder;
    use crate::sliders::SlidersBuilder;
    use mecha_wayland::mechanix_widgets::NativeText;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    struct Requests {
        shutdowns: usize,
        restarts: usize,
    }
    impl Resource for Requests {}

    #[test]
    fn overlay_requires_confirmation_and_can_restart_or_close() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule);
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        app.insert_resource(Requests {
            shutdowns: 0,
            restarts: 0,
        });
        app.system(|app: &mut App, _: &PowerOff| {
            app.resource_mut::<Requests>().shutdowns += 1;
        });
        app.system(|app: &mut App, _: &Restart| {
            app.resource_mut::<Requests>().restarts += 1;
        });

        let root = app.root();
        let content = app.spawn(
            root,
            QuickSettingsBuilder {
                header: HeaderBuilder,
                bottom_bar: BottomBarBuilder,
                sliders: SlidersBuilder,
            },
        );
        let overlay = app.spawn(root, PowerOverlayBuilder { root, content });
        let top = app.children(overlay).unwrap()[0];
        let restart = app.children(top).unwrap()[0];
        let close = app.children(top).unwrap()[1];
        let center = app.children(overlay).unwrap()[1];
        let confirm = app.children(center).unwrap()[0];
        let header = app.widgets::<crate::header::Header>().next().unwrap().0;
        let power = app.children(header).unwrap()[1];
        let click = Clicked {
            contact: ContactId::Mouse,
            position: Point::ZERO,
        };

        app.emit(click, confirm);
        app.flush();
        assert_eq!(app.resource::<Requests>().shutdowns, 0);
        assert_eq!(
            app.component::<LayoutStyle>(overlay).unwrap().display,
            Display::Hidden
        );

        app.emit(click, power);
        app.flush();
        assert_eq!(
            app.component::<LayoutStyle>(overlay).unwrap().display,
            Display::Flex
        );
        assert_eq!(
            app.component::<LayoutStyle>(content).unwrap().display,
            Display::Hidden
        );
        assert_eq!(app.resource::<Requests>().shutdowns, 0);

        app.emit(click, restart);
        app.flush();
        assert_eq!(
            app.widget::<PowerOverlay>(overlay).unwrap().action,
            Action::Restart
        );
        let label = app.widget::<PowerOverlay>(overlay).unwrap().label;
        let native = app.widget::<Text>(label).unwrap().native();
        assert_eq!(app.widget::<NativeText>(native).unwrap().string, "Restart");
        app.emit(click, close);
        app.flush();
        assert_eq!(
            app.component::<LayoutStyle>(content).unwrap().display,
            Display::Flex
        );
        assert_eq!(app.resource::<Requests>().restarts, 0);

        app.emit(click, power);
        app.flush();
        assert_eq!(
            app.widget::<PowerOverlay>(overlay).unwrap().action,
            Action::PowerOff
        );
        assert_eq!(
            app.widget::<NativeText>(native).unwrap().string,
            "Power off"
        );
        app.emit(click, confirm);
        app.flush();
        assert_eq!(app.resource::<Requests>().shutdowns, 1);
        assert_eq!(
            app.component::<LayoutStyle>(overlay).unwrap().display,
            Display::Hidden
        );

        app.emit(click, power);
        app.flush();
        app.emit(click, restart);
        app.flush();
        app.emit(click, confirm);
        app.flush();
        assert_eq!(app.resource::<Requests>().restarts, 1);
        assert_eq!(app.resource::<Requests>().shutdowns, 1);
    }
}
