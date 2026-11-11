//! The power menu: power off or restart, each asked for once more before
//! it goes to logind.
//!
//! [`PowerMenuModule`] gives it a window of its own, a full-screen overlay
//! layer above everything else, shown by [`ShowPowerMenu`] and hidden by
//! [`hide_power_menu`]. The menu is also a plain widget,
//! [`PowerMenuBuilder`], for a host that shows it in a window of its own:
//! the lock screen, which nothing else draws over while locked. Such a
//! host opens it with [`OpenPowerMenu`] and closes it with
//! [`ClosePowerMenu`].

use mecha_wayland::mechanix_widgets::prelude::{
    Button, ButtonBuilder, ButtonContextExt, ButtonSize, ButtonVariant, Text, TextContextExt,
    button, text,
};
use mecha_wayland::prelude::*;
use services::{PowerModule, PowerOff, Restart};

/// Show the power menu's own window.
pub struct ShowPowerMenu;
impl Signal for ShowPowerMenu {}

/// Hide the power menu's own window, as if closed.
pub fn hide_power_menu(app: &mut App) {
    if let Some(overlay) = app
        .has_resource::<Overlay>()
        .then(|| app.resource::<Overlay>().menu)
    {
        app.emit(ClosePowerMenu, overlay);
    }
}

/// The [`PowerMenu`] it is emitted at closed: by its close button, or
/// after asking logind.
pub struct PowerMenuClosed;
impl Event for PowerMenuClosed {}

/// Open the [`PowerMenu`] it is emitted at, on power off.
pub struct OpenPowerMenu;
impl Event for OpenPowerMenu {}

/// Close the [`PowerMenu`] it is emitted at.
pub struct ClosePowerMenu;
impl Event for ClosePowerMenu {}

/// The namespace prefix stacks it above everything, fullscreen apps and
/// the shell's bars included.
const NAMESPACE: &str = "mx-prompt-power";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

/// Installs the power menu's window, hidden, and logind's power-off and
/// reboot. Expects the host to have installed `DBusModule::new().system()`,
/// `RingModule` and `WindowModule`, and to have inserted the [`Atlas`] and
/// `FontBook`.
pub struct PowerMenuModule;

/// The power menu's own window and the menu in it.
struct Overlay {
    window: NodeId,
    menu: Handle<PowerMenu>,
}
impl Resource for Overlay {}

impl Module for PowerMenuModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<Overlay>() {
            return;
        }
        app.add_module(PowerModule);
        let root = app.root();
        let role = Role::Layer(LayerRole {
            layer: Layer::Overlay,
            anchor: ALL_SIDES,
            // Over the bars' zones too.
            exclusive_zone: -1,
            namespace: NAMESPACE.into(),
            keyboard_interactivity: KeyboardInteractivity::None,
        });
        let window = app
            .spawn_with(
                root,
                window()
                    .title(NAMESPACE)
                    .layout(LayoutStyle::default())
                    .clear(Color::TRANSPARENT),
                (role,),
            )
            .id();
        let menu = app.spawn(window, PowerMenuBuilder);
        app.insert_resource(Overlay { window, menu });
        set_mapped(app, false);
        app.system(|app: &mut App, _: &ShowPowerMenu| {
            set_mapped(app, true);
            let menu = app.resource::<Overlay>().menu;
            app.emit(OpenPowerMenu, menu);
        })
        .system(|app: &mut App, e: &Emitted<PowerMenuClosed>| {
            if e.targets.contains(&app.resource::<Overlay>().menu.id()) {
                set_mapped(app, false);
            }
        });
    }
}

/// Map or unmap the menu's own window.
fn set_mapped(app: &mut App, mapped: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let window = app.resource::<Overlay>().window;
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_mapped(window, mapped, &mut wl);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    PowerOff,
    Restart,
}

impl Action {
    fn other(self) -> Self {
        match self {
            Action::PowerOff => Action::Restart,
            Action::Restart => Action::PowerOff,
        }
    }
}

/// Power off, or restart, behind one more tap; the top-left button
/// switches to the other. Fills its parent, hidden until
/// [`OpenPowerMenu`].
pub struct PowerMenu {
    /// Offers the action not chosen.
    switch: Handle<Button>,
    confirm: Handle<Button>,
    label: Handle<Text>,
    power_icon: SpriteId,
    restart_icon: SpriteId,
    action: Action,
}

pub struct PowerMenuBuilder;

impl Build for PowerMenuBuilder {
    type Widget = PowerMenu;
}

fn choose(ctx: &mut Context<'_, PowerMenu>, action: Action) {
    let (label, confirm, switch) = (ctx.me().label, ctx.me().confirm, ctx.me().switch);
    let (name, icon) = match action {
        Action::PowerOff => ("Power off", ctx.me().power_icon),
        Action::Restart => ("Restart", ctx.me().restart_icon),
    };
    ctx.me().action = action;
    ctx.at(label).unwrap().set_text(name);
    ctx.at(confirm).unwrap().set_icon(icon);
    ctx.at(switch)
        .unwrap()
        .set_label(switch_label(action.other()));
}

fn switch_label(action: Action) -> &'static str {
    match action {
        Action::PowerOff => "POWER OFF",
        Action::Restart => "RESTART",
    }
}

/// The menu shows.
fn is_open(ctx: &Context<'_, PowerMenu>) -> bool {
    ctx.style().display != Display::Hidden
}

fn set_open(ctx: &mut Context<'_, PowerMenu>, open: bool) {
    let was = is_open(ctx);
    ctx.set_display(if open { Display::Flex } else { Display::Hidden });
    if was && !open {
        let me = ctx.handle();
        ctx.emit(PowerMenuClosed, me);
    }
}

fn text_button(label: &str) -> ButtonBuilder {
    button()
        .variant(ButtonVariant::Text)
        .text_color(ColorRole::OnSurface)
        .label(label)
        .hover_background(Color::TRANSPARENT)
        .pressed_background(Color::TRANSPARENT)
        .height(px(40.0))
}

impl Widget for PowerMenu {
    type Builder = PowerMenuBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
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
        let switch = s.spawn(top, text_button(switch_label(Action::Restart)));
        let close = s.spawn(top, text_button("CLOSE"));

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
        let (power_icon, restart_icon) = {
            let mut atlas = s.resource_mut::<Atlas>();
            let mut icon = |svg| {
                atlas
                    .insert_svg(svg, icon_size)
                    .expect("embedded icon SVG should render and fit in the atlas")
            };
            (icon(assets::icons::POWER), icon(assets::icons::ROTATE))
        };
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

        s.on::<OpenPowerMenu>(me, |ctx, _| {
            choose(ctx, Action::PowerOff);
            set_open(ctx, true);
        });
        s.on::<ClosePowerMenu>(me, |ctx, _| set_open(ctx, false));
        s.on::<Clicked>(switch, |ctx, _| {
            let other = ctx.me().action.other();
            choose(ctx, other);
        });
        s.on::<Clicked>(close, |ctx, _| set_open(ctx, false));
        s.on::<Clicked>(confirm, |ctx, _| {
            if !is_open(ctx) {
                return;
            }
            let action = ctx.me().action;
            set_open(ctx, false);
            match action {
                Action::PowerOff => ctx.signal(PowerOff),
                Action::Restart => ctx.signal(Restart),
            }
        });

        PowerMenu {
            switch,
            confirm,
            label,
            power_icon,
            restart_icon,
            action: Action::PowerOff,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[derive(Default)]
    struct Seen {
        shutdowns: usize,
        restarts: usize,
        closed: usize,
    }
    impl Resource for Seen {}

    fn app() -> App {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(WindowModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        app.init_resource::<Seen>();
        app.system(|app: &mut App, _: &PowerOff| app.resource_mut::<Seen>().shutdowns += 1)
            .system(|app: &mut App, _: &Restart| app.resource_mut::<Seen>().restarts += 1)
            .system(|app: &mut App, _: &Emitted<PowerMenuClosed>| {
                app.resource_mut::<Seen>().closed += 1;
            });
        app
    }

    const CLICK: Clicked = Clicked {
        contact: ContactId::Mouse,
        position: Point::ZERO,
    };

    struct Parts {
        menu: NodeId,
        switch: NodeId,
        close: NodeId,
        confirm: NodeId,
    }

    fn parts(app: &App, menu: NodeId) -> Parts {
        let top = app.children(menu).unwrap()[0];
        let center = app.children(menu).unwrap()[1];
        Parts {
            menu,
            switch: app.children(top).unwrap()[0],
            close: app.children(top).unwrap()[1],
            confirm: app.children(center).unwrap()[0],
        }
    }

    fn shown(app: &App, node: NodeId) -> bool {
        app.component::<LayoutStyle>(node).unwrap().display == Display::Flex
    }

    #[test]
    fn asks_once_more_and_can_restart_or_close() {
        let mut app = app();
        let root = app.root();
        let menu = app.spawn(root, PowerMenuBuilder).id();
        let p = parts(&app, menu);

        // Hidden, it does nothing.
        app.emit(CLICK, p.confirm);
        app.flush();
        assert_eq!(app.resource::<Seen>().shutdowns, 0);
        assert!(!shown(&app, p.menu));

        app.emit(OpenPowerMenu, menu);
        app.flush();
        assert!(shown(&app, p.menu));
        app.emit(CLICK, p.confirm);
        app.flush();
        assert_eq!(app.resource::<Seen>().shutdowns, 1);
        assert_eq!(app.resource::<Seen>().closed, 1);
        assert!(!shown(&app, p.menu));

        app.emit(OpenPowerMenu, menu);
        app.emit(CLICK, p.switch);
        app.emit(CLICK, p.confirm);
        app.flush();
        assert_eq!(app.resource::<Seen>().restarts, 1);

        // The switch goes back to power off.
        app.emit(OpenPowerMenu, menu);
        app.emit(CLICK, p.switch);
        app.flush();
        assert_eq!(switch_text(&app, p.switch), "POWER OFF");
        app.emit(CLICK, p.switch);
        app.flush();
        assert_eq!(switch_text(&app, p.switch), "RESTART");
        app.emit(CLICK, p.confirm);
        app.flush();
        assert_eq!(app.resource::<Seen>().shutdowns, 2);

        // Opening again starts over at power off.
        app.emit(OpenPowerMenu, menu);
        app.emit(CLICK, p.switch);
        app.emit(OpenPowerMenu, menu);
        app.flush();
        assert_eq!(switch_text(&app, p.switch), "RESTART");
        app.emit(CLICK, p.close);
        app.flush();
        assert_eq!(app.resource::<Seen>().closed, 4);
        assert_eq!(app.resource::<Seen>().shutdowns, 2);
        assert_eq!(app.resource::<Seen>().restarts, 1);
    }

    fn switch_text(app: &App, switch: NodeId) -> String {
        let label = app.widget::<Button>(switch).unwrap().label().unwrap();
        let native = app.widget::<Text>(label).unwrap().native();
        app.widget::<mecha_wayland::prelude::Text>(native)
            .unwrap()
            .string
            .clone()
    }

    #[test]
    fn the_module_shows_and_hides_its_window() {
        let mut app = app();
        // PresentationModule registers Role; it needs a live compositor.
        app.register_component::<Role>();
        app.add_module(RingModule::default())
            .add_module(DBusModule::new().system())
            .add_module(PowerMenuModule);
        app.flush();
        let menu = app.resource::<Overlay>().menu.id();
        let window = app.resource::<Overlay>().window;
        assert_eq!(app.parent(menu), Some(window));
        assert!(!shown(&app, menu));

        app.signal(ShowPowerMenu);
        app.flush();
        assert!(shown(&app, menu));

        hide_power_menu(&mut app);
        app.flush();
        assert!(!shown(&app, menu));
        assert_eq!(app.resource::<Seen>().closed, 1);
    }
}
