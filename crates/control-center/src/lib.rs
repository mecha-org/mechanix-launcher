use mecha_wayland::prelude::*;
use screen::Screen;

pub mod app_screens;
pub mod control_center;
pub mod screen_selector;

use crate::app_screens::AppScreen;
use crate::control_center::{ControlCenter, ControlCenterBuilder};

pub struct ControlCenterScreen;

impl Screen for ControlCenterScreen {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        let cc = app.spawn(
            parent,
            ControlCenterBuilder {
                opened_screen: AppScreen::default(),
            },
        );

        let containers = app
            .widget::<ControlCenter>(cc)
            .expect("control center just spawned")
            .containers;

        for (screen, node) in containers {
            screen.build(app, node.id());
        }
        cc.id()
    }
}
