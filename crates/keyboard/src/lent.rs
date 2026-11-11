//! The keys in another window. A host may move the keyboard's keys node,
//! with [`App::move_to`], into a window of its own and back, as the lock
//! screen does: the same keys, typing as ever, shown elsewhere. They show
//! wherever they are while the keyboard is visible, and hide while it
//! isn't. While they are away, the keyboard's own surface stays unmapped.

use mecha_wayland::prelude::*;

use crate::visibility::KeyboardVisibilityChanged;

/// The keys node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keys {
    pub(crate) node: NodeId,
}
impl Resource for Keys {}

/// The keys node, to move: as wide as its parent, as tall as the keys, and
/// hidden while the keyboard is. `None` without the keyboard.
pub fn keys(app: &App) -> Option<NodeId> {
    app.has_resource::<Keys>()
        .then(|| app.resource::<Keys>().node)
}

pub(crate) fn install(app: &mut App, keys: Keys) {
    app.insert_resource(keys);
    app.system(on_moved);
}

/// The keys moved: the surface maps or unmaps to match.
fn on_moved(app: &mut App, m: &Moved) {
    if m.id == app.resource::<Keys>().node {
        app.signal(KeyboardVisibilityChanged);
    }
}
