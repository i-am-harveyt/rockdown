use gpui::{TestAppContext, px, size};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
};

#[gpui::test]
fn restored_offset_uses_measured_wrapped_height_and_rejects_stale_offsets(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let source = format!("{}\nshort\nend", "wrapped words ".repeat(60));
    let (view, window) = cx.add_window_view(|window, cx| {
        let mut app = Workspace::new(
            Config {
                writing_width: 320.,
                ..Default::default()
            },
            None,
            Document::untitled(&source),
            Explorer::open(root.path()).unwrap(),
            window,
            cx,
        );
        app.documents.viewport_mut().offset = 60.;
        app.initialize_recovery(None, None, cx);
        app
    });
    let mut window = window.clone();
    window.simulate_resize(size(px(900.), px(700.)));
    window.run_until_parked();
    window.update(|_, cx| {
        assert_eq!(view.read(cx).scroll_offsets[Pane::Editor.index()], 60.);
        view.update(cx, |app, cx| {
            app.documents.viewport_mut().top = 1;
            app.documents.viewport_mut().offset = 10000.;
            app.initialize_recovery(None, None, cx);
            cx.notify();
        });
    });
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.tops[Pane::Editor.index()], 1);
        assert_eq!(app.scroll_offsets[Pane::Editor.index()], 0.);
        assert_eq!(app.layouts[Pane::Editor.index()].rows[0].source_row, 1);
    });
}
