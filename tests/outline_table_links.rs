use gpui::{Modifiers, TestAppContext, point, px, size};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
};

#[gpui::test]
fn modifier_click_on_wrapped_centered_table_link_jumps_without_editing(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let source = format!(
        "# Start\n\n| Section |\n| :---: |\n| [{}](#%E7%9B%AE%E7%9A%84%E5%9C%B0) |\n\n## 目的地\n",
        "中文 section link ".repeat(10)
    );
    let path = root.path().join("note.md");
    std::fs::write(&path, &source).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config {
                writing_width: 320.,
                ..Default::default()
            },
            None,
            Document::open(&path).unwrap(),
            Explorer::open(root.path()).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.simulate_resize(size(px(900.), px(800.)));
    window.run_until_parked();
    let click = window.update(|_, cx| {
        let app = view.read(cx);
        let link = app.layouts[Pane::Editor.index()]
            .links
            .last()
            .expect("visible table link");
        link.bounds.origin + point(link.bounds.size.width / 2., link.bounds.size.height / 2.)
    });
    window.simulate_click(
        click,
        Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.row, 6);
        assert_eq!(app.documents.current().buffer.text(), source);
        assert!(!app.documents.current().buffer.dirty());
    });
}
