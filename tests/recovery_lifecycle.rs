use gpui::TestAppContext;
use rockdown::{
    app::Workspace, config::Config, document::Document, explorer::Explorer, recovery::RecoveryStore,
};
use std::time::Duration;

#[gpui::test]
fn periodic_recovery_keeps_unsaved_typing_without_saving_the_document(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let storage = tempfile::tempdir().unwrap();
    let path = root.path().join("draft.md");
    std::fs::write(&path, "original").unwrap();
    let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        let mut app = Workspace::new(
            Config::default(),
            None,
            Document::open(&path).unwrap(),
            Explorer::open(root.path()).unwrap(),
            window,
            cx,
        );
        app.initialize_recovery(Some(store), None, cx);
        app
    });
    let mut window = window.clone();
    window.simulate_keystrokes("A");
    window.simulate_input(" unsaved 中文");
    window.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(3));
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    // Release recovery without finalization: model a process losing its window.
    window.update(|_, cx| {
        view.update(cx, |app, cx| app.initialize_recovery(None, None, cx));
    });
    // Remove the surface without the application's close confirmation: model a crash.
    window.update(|window, _| window.remove_window());
    drop(view);
    window.run_until_parked();
    let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
    let mut restored = store.load().unwrap().unwrap();
    assert_eq!(restored.current().buffer.text(), "original unsaved 中文");
    assert!(restored.current().buffer.dirty());
    std::fs::write(&path, "external edit").unwrap();
    assert!(restored.save(None, false).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "external edit");
}

#[gpui::test]
fn force_quit_does_not_resurrect_discarded_drafts(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let storage = tempfile::tempdir().unwrap();
    let path = root.path().join("draft.md");
    std::fs::write(&path, "saved").unwrap();
    let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        let mut app = Workspace::new(
            Config::default(),
            None,
            Document::open(&path).unwrap(),
            Explorer::open(root.path()).unwrap(),
            window,
            cx,
        );
        app.initialize_recovery(Some(store), None, cx);
        app
    });
    let mut window = window.clone();
    window.simulate_keystrokes("A");
    window.simulate_input(" discarded");
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(3));
    window.simulate_keystrokes(": q ! enter");
    drop(view);
    window.run_until_parked();
    let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
    let restored = store.load().unwrap().unwrap();
    assert_eq!(restored.current().buffer.text(), "saved");
    assert!(!restored.current().buffer.dirty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "saved");
}
