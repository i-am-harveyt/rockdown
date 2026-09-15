use gpui::{TestAppContext, px, size};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
};

#[gpui::test]
fn adding_back_a_missing_encoded_local_image_repairs_its_preview(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.md");
    let source = "# Document\n\n![Picture](assets/%E5%9C%96%20one.png \"120px\")\nAfter image";
    std::fs::write(&path, source).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::open(&path).unwrap(),
            Explorer::open(root.path()).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.simulate_resize(size(px(1000.), px(800.)));
    window.run_until_parked();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([40, 120, 200, 255]))
        .save(root.path().join("assets/圖 one.png"))
        .unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.refresh_projection();
            cx.notify();
        })
    });
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let row = app.layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 2)
            .unwrap();
        assert_eq!(row.height, row.line_height + px(120.));
        assert_eq!(app.documents.current().buffer.text(), source);
        assert!(!app.documents.current().buffer.dirty());
    });
}
