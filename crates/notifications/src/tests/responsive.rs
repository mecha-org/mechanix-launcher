use mecha_wayland::mechanix_widgets::prelude::FontBook;
use mecha_wayland::prelude::*;
use screen::Screen;
use services::KeepAwake;

use crate::Notifications;
use crate::notification_store::NotificationStore;
use crate::notifications::on_store_changed;
use crate::notify::{Notification, Notify, append_notifications};
use crate::notify_signal::notify_signal;

const HEIGHT: f32 = 620.0;
const LONG: &str = "with a summary long enough to overflow the card on any screen width";

fn app() -> App {
    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(MechanixTheme::dark())
        .add_module(RingModule::default())
        .add_module(TimerModule);
    let mut atlas = Atlas::new();
    let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
    app.insert_resource(atlas);
    app.insert_resource(FontBook::new(font));
    app.init_resource::<KeepAwake>()
        .init_resource::<NotificationStore>()
        .system(notify_signal)
        .system(on_store_changed);
    crate::ongoing::install(&mut app);
    app
}

fn notify(app: &mut App, app_name: &str, summary: String) {
    let notification = Notification {
        app_name: app_name.into(),
        summary,
        ..Default::default()
    };
    append_notifications(
        app,
        &Notify {
            replaces_id: None,
            notification,
        },
    );
}

fn descendants(app: &App, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(children) = app.children(id) {
            stack.extend(children.iter().copied());
        }
        out.push(id);
    }
    out
}

#[test]
fn the_panel_fits_every_width() {
    let mut app = app();
    let size = |width| LayoutStyle::default().size(px(width), px(HEIGHT));
    let window = app.spawn(app.root(), window().layout(size(540.0))).id();
    let panel = Notifications.build(&mut app, window);

    for (i, name) in ["Mail", "Messages", "Mail", "Calendar"]
        .into_iter()
        .enumerate()
    {
        notify(&mut app, name, format!("{name} #{i} {LONG}"));
    }

    for width in [320.0, 540.0, 1280.0] {
        *app.component_mut::<LayoutStyle>(window).unwrap() = size(width);
        app.tick();
        app.tick();
        assert!(
            app.widgets::<crate::card::Card>().next().is_some(),
            "no cards"
        );

        let mut checked = 0;
        for id in descendants(&app, panel) {
            let Some(layout) = app.component::<ComputedLayout>(id) else {
                continue;
            };
            let rect = layout.rect;
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                continue; // hidden
            }
            assert!(
                rect.origin.x >= -0.5 && rect.origin.x + rect.width() <= width + 0.5,
                "at {width}: {id:?} spans {}..{}",
                rect.origin.x,
                rect.origin.x + rect.width(),
            );
            checked += 1;
        }
        assert!(checked > 20, "only {checked} nodes laid out at {width}");
    }
}
