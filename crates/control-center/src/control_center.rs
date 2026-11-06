use crate::app_screens::AppScreen;
use crate::drag_handle::DragHandleBuilder;
use crate::screen_selector::ScreenSelectorBuilder;
use mecha_wayland::prelude::*;

pub struct ControlCenterBuilder {
    pub opened_screen: AppScreen,
}

pub struct ControlCenter {
    pub containers: [(AppScreen, Handle<Div>); AppScreen::ALL.len()],
}

impl Build for ControlCenterBuilder {
    type Widget = ControlCenter;
}

fn shown(on: bool) -> Display {
    if on { Display::Flex } else { Display::Hidden }
}

impl Widget for ControlCenter {
    type Builder = ControlCenterBuilder;

    fn build(builder: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .height(percent(100.0));
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        s.spawn(
            me,
            ScreenSelectorBuilder {
                control_center: me.id(),
                selected: builder.opened_screen,
            },
        );

        let opened = builder.opened_screen;
        let container = |screen: AppScreen| {
            div().style(
                LayoutStyle::default()
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0))
                    .display(shown(opened == screen)),
            )
        };
        let containers = AppScreen::ALL.map(|screen| (screen, s.spawn(me, container(screen))));
        // One handle for every screen: the panel closes on an up swipe
        // whichever screen is showing.
        s.spawn(me, DragHandleBuilder);

        s.on::<AppScreen>(me, move |ctx, selected| {
            for (screen, node) in containers {
                if let Some(mut c) = ctx.at(node)
                    && let Some(mut style) = c.component_mut::<LayoutStyle>()
                {
                    style.display = shown(screen == *selected);
                }
            }
        });

        Self { containers }
    }
}
