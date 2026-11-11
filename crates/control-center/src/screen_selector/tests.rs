use super::*;
use crate::control_center::{ControlCenter, ControlCenterBuilder};
use mecha_wayland::mechanix_widgets::{NativeText, prelude::FontBook};

struct Fixture {
    app: App,
    window: NodeId,
    center: Handle<ControlCenter>,
    row: NodeId,
    cells: [NodeId; 2],
}

impl Fixture {
    fn new(selected: AppScreen) -> Self {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let window = app.spawn(app.root(), window()).id();
        let center = app.spawn(
            window,
            ControlCenterBuilder {
                opened_screen: selected,
            },
        );
        let selector = app.children(center).unwrap()[0];
        let row = app.children(selector).unwrap()[1];
        let cells = app.children(row).unwrap().try_into().unwrap();
        let mut fixture = Self {
            app,
            window,
            center,
            row,
            cells,
        };
        fixture.resize(960.0);
        fixture
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.app.tick();
        }
    }

    fn resize(&mut self, width: f32) {
        *self.app.component_mut::<LayoutStyle>(self.window).unwrap() =
            LayoutStyle::default().size(px(width), px(347.0));
        self.settle();
    }

    fn rect(&self, node: NodeId) -> Rect {
        self.app.component::<ComputedLayout>(node).unwrap().rect
    }

    fn parts(&self, cell: NodeId) -> [NodeId; 4] {
        self.app.children(cell).unwrap().try_into().unwrap()
    }

    fn label_widths(&self) -> [f32; 2] {
        self.cells
            .map(|cell| self.rect(self.parts(cell)[1]).width())
    }

    fn assert_selection(&self, selected: AppScreen) {
        let containers = self
            .app
            .widget::<ControlCenter>(self.center)
            .unwrap()
            .containers;
        for (screen, cell) in AppScreen::ALL.into_iter().zip(self.cells) {
            let [_, label, _, indicator] = self.parts(cell);
            let on = screen == selected;
            let display = if on { Display::Flex } else { Display::Hidden };
            assert_eq!(
                self.app
                    .component::<LayoutStyle>(indicator)
                    .unwrap()
                    .display,
                display
            );
            let (_, container) = containers.iter().find(|(s, _)| *s == screen).unwrap();
            assert_eq!(
                self.app
                    .component::<LayoutStyle>(*container)
                    .unwrap()
                    .display,
                display
            );
            let label = self.app.widget::<Text>(label).unwrap();
            let native = self.app.widget::<NativeText>(label.native()).unwrap();
            let role = if on {
                ColorRole::OnSurface
            } else {
                ColorRole::OnSurfaceVariant
            };
            assert_eq!(native.color, self.app.color(role));
            if on {
                let cell = self.rect(cell);
                let indicator = self.rect(indicator);
                close(indicator.x(), cell.x());
                close(indicator.width(), cell.width());
                close(indicator.height(), 2.0);
                close(indicator.bottom(), cell.bottom());
            }
        }
    }
}

// Layout snaps to pixels, so adjacent rounded edges can differ by one unit.
fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn full_labels_fit_and_spacers_shrink_then_recover_on_resize() {
    let mut f = Fixture::new(AppScreen::NotificationsScreen);
    let natural_widths = f.label_widths();
    assert!(natural_widths[0] > natural_widths[1]);
    let native_props = f.cells.map(|cell| {
        let label = f.app.widget::<Text>(f.parts(cell)[1]).unwrap();
        f.app
            .widget::<NativeText>(label.native())
            .unwrap()
            .props
            .clone()
    });

    // Logical widths spanning scales 0.77, 1.0, 2.61 and 3.1, then grow again.
    for width in [1247.0, 960.0, 368.0, 308.0, 960.0] {
        f.resize(width);
        let row = f.rect(f.row);
        close(row.width(), width);
        let mut previous_right = row.x();
        for (index, cell) in f.cells.into_iter().enumerate() {
            let [left, label, right, _] = f.parts(cell);
            let cell = f.rect(cell);
            let label_rect = f.rect(label);
            close(cell.x(), previous_right);
            assert!(cell.right() <= row.right());
            previous_right = cell.right();
            close(cell.height(), 47.0);
            close(label_rect.width(), natural_widths[index]);
            let themed = f.app.widget::<Text>(label).unwrap();
            assert_eq!(themed.variant, TextVariant::HeadlineSmall);
            assert!(themed.emphasised);
            let native = f.app.widget::<NativeText>(themed.native()).unwrap();
            assert_eq!(native.string, ["NOTIFICATIONS", "SETTINGS"][index]);
            assert_eq!(native.props, native_props[index]);
            close(f.rect(themed.native().id()).width(), natural_widths[index]);
            assert!(label_rect.x() >= cell.x() + 2.0);
            assert!(label_rect.right() <= cell.right() - 2.0);
            close(label_rect.x() - cell.x(), cell.right() - label_rect.right());
            let left = f.rect(left);
            let right = f.rect(right);
            for spacer in [left, right] {
                assert!(
                    (2.0..=16.0).contains(&spacer.width()),
                    "{width}: {spacer:?}"
                );
                if width >= 960.0 {
                    close(spacer.width(), 16.0);
                } else if width == 308.0 {
                    assert!(spacer.width() < 16.0);
                }
            }
            close(left.width(), right.width());
            close(left.right(), label_rect.x());
            close(right.x(), label_rect.right());
            if width >= 960.0 {
                // Each tab receives the same spare width above its own text width.
                close(
                    cell.width() - natural_widths[index],
                    (width - natural_widths.iter().sum::<f32>()) / 2.0,
                );
            }
        }
        close(previous_right, row.right());
        assert!(f.rect(f.cells[0]).width() > f.rect(f.cells[1]).width());
        f.assert_selection(AppScreen::NotificationsScreen);
    }
}

#[test]
fn top_spacing_yields_to_content_and_recovers_without_squeezing_tabs() {
    let mut f = Fixture::new(AppScreen::NotificationsScreen);
    let selector = f.app.children(f.center).unwrap()[0];
    let spacer = f.app.children(selector).unwrap()[0];
    let divider = f.app.children(selector).unwrap()[2];
    let content = f.app.widget::<ControlCenter>(f.center).unwrap().containers[0].1;
    // Reserve a realistic content height to put pressure on the selector through
    // its parent, rather than imposing a smaller selector height directly.
    f.app
        .component_mut::<LayoutStyle>(content)
        .unwrap()
        .min_height = px(230.0);
    let label_heights = f.cells.map(|cell| f.rect(f.parts(cell)[1]).height());

    // 47-unit tabs + 1-unit divider + 230-unit content + 48-unit drag handle.
    for (height, gap) in [(400.0, 20.0), (336.0, 10.0), (326.0, 0.0), (400.0, 20.0)] {
        *f.app.component_mut::<LayoutStyle>(f.window).unwrap() =
            LayoutStyle::default().size(px(308.0), px(height));
        f.settle();
        let selector_rect = f.rect(selector);
        let spacer_rect = f.rect(spacer);
        let row = f.rect(f.row);
        let divider_rect = f.rect(divider);
        close(spacer_rect.height(), gap);
        close(spacer_rect.y(), selector_rect.y());
        close(row.y(), spacer_rect.bottom());
        close(row.height(), 47.0);
        close(divider_rect.y(), row.bottom());
        close(divider_rect.height(), 1.0);
        close(divider_rect.bottom(), selector_rect.bottom());
        close(f.rect(content.id()).y(), selector_rect.bottom());
        assert!(f.rect(content.id()).height() >= 230.0);
        let handle = *f.app.children(f.center).unwrap().last().unwrap();
        close(f.rect(handle).bottom(), height);
        for (cell, expected_height) in f.cells.into_iter().zip(label_heights) {
            close(f.rect(f.parts(cell)[1]).height(), expected_height);
        }
        f.assert_selection(AppScreen::NotificationsScreen);
    }
}

#[test]
fn clicks_and_programmatic_selection_keep_indicator_and_content_in_sync() {
    for initial in AppScreen::ALL {
        let mut f = Fixture::new(initial);
        f.resize(308.0);
        f.assert_selection(initial);
        let widths = f.label_widths();
        for index in [1, 0, 1] {
            let cell = f.cells[index];
            let rect = f.rect(cell);
            f.app.emit(
                Clicked {
                    contact: ContactId::Mouse,
                    position: Point::new(
                        rect.x() + rect.width() / 2.0,
                        rect.y() + rect.height() / 2.0,
                    ),
                },
                cell,
            );
            f.settle();
            f.assert_selection(AppScreen::ALL[index]);
            assert_eq!(f.label_widths(), widths);
        }
        for selected in AppScreen::ALL {
            crate::select_screen(&mut f.app, selected);
            f.settle();
            f.assert_selection(selected);
            f.resize(960.0);
            f.assert_selection(selected);
            f.resize(308.0);
            f.assert_selection(selected);
        }
    }
}
