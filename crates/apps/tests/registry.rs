use std::path::PathBuf;

use apps::{AppKey, Registry, RegistryChanged, RegistryModule};
use mecha_wayland::prelude::*;
use screen::Shown;
use wayland::fake::Fake;

const MANAGER: u32 = 5;
const A: u32 = 0xff00_0000;
const B: u32 = 0xff00_0001;

const EV_TITLE: u16 = 0;
const EV_APP_ID: u16 = 1;
const EV_DONE: u16 = 5;

#[derive(Default)]
struct Changes(u32);
impl Resource for Changes {}

fn count(app: &mut App, _: &RegistryChanged) {
    app.resource_mut::<Changes>().0 += 1;
}

fn settle(f: &mut Fake) {
    f.turn();
    f.app.tick();
}

fn data_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("apps-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("applications")).unwrap();
    root
}

fn fake(dir: &PathBuf) -> Fake {
    let mut f = Fake::new(
        &[("wl_seat", 9), ("zwlr_foreign_toplevel_manager_v1", 3)],
        |m| m.bind::<WlSeat>(),
    );
    f.app.add_module(ToplevelsModule);
    f.app.add_module(RegistryModule {
        data_dirs: vec![dir.clone()],
    });
    f.app.init_resource::<Changes>().system(count);
    f.turn();
    f.requests();
    f
}

fn open(f: &mut Fake, id: u32, app_id: &str, title: &str) {
    f.send(MANAGER, 0, |w| w.new_id(ObjectId(id)));
    f.send(id, EV_APP_ID, |w| w.string(app_id));
    f.send(id, EV_TITLE, |w| w.string(title));
    f.send(id, EV_DONE, |_| {});
}

fn changes(f: &Fake) -> u32 {
    f.app.resource::<Changes>().0
}

fn shown(f: &mut Fake) {
    let root = f.app.root();
    f.app.emit(Shown, root);
    f.app.flush();
}

#[test]
fn registry_follows_the_window_list() {
    let dir = data_dir("windows");
    let mut f = fake(&dir);
    assert!(f.app.resource::<Registry>().apps().is_empty());

    open(&mut f, A, "Org.Notes", "one");
    open(&mut f, B, "org.notes", "two");
    settle(&mut f);
    {
        let r = f.app.resource::<Registry>();
        assert_eq!(r.apps().len(), 1);
        assert_eq!(r.apps()[0].key, AppKey::AppId("org.notes".into()));
        assert_eq!(r.apps()[0].windows.len(), 2);
    }
    assert_eq!(changes(&f), 1, "one signal for the batch");

    // A title-only change regroups to the same apps: no signal.
    f.send(A, EV_TITLE, |w| w.string("renamed"));
    f.send(A, EV_DONE, |_| {});
    settle(&mut f);
    assert_eq!(changes(&f), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shown_rescans_and_signals_only_on_change() {
    let dir = data_dir("shown");
    let mut f = fake(&dir);
    shown(&mut f);
    assert_eq!(changes(&f), 0);

    std::fs::write(
        dir.join("applications/notes.desktop"),
        "[Desktop Entry]\nType=Application\nName=Notes\nExec=notes\n",
    )
    .unwrap();
    shown(&mut f);
    assert_eq!(changes(&f), 1);
    assert_eq!(f.app.resource::<Registry>().entries().len(), 1);

    shown(&mut f);
    assert_eq!(changes(&f), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
