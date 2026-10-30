use mecha_wayland::mechanix_widgets::prelude::{TextVariant, text};
use mecha_wayland::prelude::*;

/// A full-size piece of launcher content. A screen builds itself under any
/// parent node and owns no window, so the nav can place it in whichever window
/// it likes. Screens live in their own crates and are held as
/// `Box<dyn Screen>`, so this trait must stay object-safe.
pub trait Screen: 'static {
    /// Builds this screen under `parent` and returns its root node.
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId;
}

/// Emitted by the nav at the node a [`Screen::build`] returned when the
/// screen's container is shown.
pub struct Shown;
impl Event for Shown {}

/// Emitted by the nav at the node a [`Screen::build`] returned when the
/// screen's container is hidden.
pub struct Hidden;
impl Event for Hidden {}

/// Signalled by a screen after it starts an app.
pub struct AppLaunched;
impl Signal for AppLaunched {}

/// Spawns a themed full-size page showing `label` centred. Used by screens
/// that have no real page yet.
pub fn placeholder_page(app: &mut App, parent: NodeId, label: &str) -> NodeId {
    app.spawn(
        parent,
        PlaceholderPage {
            label: label.to_string(),
        },
    )
    .id()
}

struct PlaceholderPage {
    label: String,
}

impl Build for PlaceholderPage {
    type Widget = PlaceholderPage;
}

impl Widget for PlaceholderPage {
    type Builder = PlaceholderPage;

    fn build(b: Self, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .center()
            .width(percent(100.0))
            .height(percent(100.0));

        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        s.spawn(
            me,
            text(b.label.clone()).variant(TextVariant::HeadlineMedium),
        );
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::{FontBook, Text};
    use status_bar::GEIST_MONO_BYTES;

    #[test]
    fn placeholder_page_spawns_label() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());
        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let root = app.root();
        let page = placeholder_page(&mut app, root, "Search");
        app.tick();

        let texts: Vec<_> = app
            .descendants(page)
            .filter(|n| app.widget::<Text>(*n).is_some())
            .collect();
        assert_eq!(texts.len(), 1);
    }

    #[test]
    fn screen_is_object_safe() {
        fn _takes(_: Vec<Box<dyn Screen>>) {}
    }
}
