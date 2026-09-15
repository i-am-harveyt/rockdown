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
    // A completed replacement changes aspect ratio and therefore following-row
    // positions without another user action after the refresh that discovers it.
    image::RgbaImage::from_pixel(8, 16, image::Rgba([200, 40, 120, 255]))
        .save(root.path().join("assets/圖 one.png"))
        .unwrap();
    window.update(|_, cx| view.update(cx, |_, cx| cx.notify()));
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let rows = &app.layouts[Pane::Editor.index()].rows;
        let image = rows.iter().find(|row| row.source_row == 2).unwrap();
        let following = rows.iter().find(|row| row.source_row == 3).unwrap();
        assert_eq!(image.height, image.line_height + px(240.));
        assert_eq!(following.origin.y, image.origin.y + image.height);
        assert_eq!(app.documents.current().buffer.text(), source);
        assert!(!app.documents.current().buffer.dirty());
    });
}

#[gpui::test]
fn queued_offscreen_images_do_not_evict_visible_previews_forever(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("many.md");
    let mut source = String::from("# Images\n");
    for index in 0..40 {
        image::RgbaImage::from_pixel(40, 40, image::Rgba([index, 80, 120, 255]))
            .save(root.path().join(format!("{index}.png")))
            .unwrap();
        source.push_str(&format!("![Image]({index}.png \"120px\")\n"));
    }
    std::fs::write(&path, &source).unwrap();
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
    // Loading placeholders initially expose more than one cache's worth of
    // rows; loaded images then push most of those requests below the viewport.
    window.simulate_resize(size(px(1000.), px(2400.)));
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let rows = &app.layouts[Pane::Editor.index()].rows;
        assert!(rows.last().unwrap().source_row < 40);
        for row in rows.iter().filter(|row| row.source_row > 0) {
            assert_eq!(row.height, row.line_height + px(120.));
        }
    });
    // More simultaneously visible bitmaps than the idle-cache budget must
    // settle too, rather than continuously evicting and decoding each other.
    window.simulate_resize(size(px(1000.), px(8000.)));
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let rows = &app.layouts[Pane::Editor.index()].rows;
        for index in 1..=40 {
            let row = rows.iter().find(|row| row.source_row == index).unwrap();
            assert_eq!(row.height, row.line_height + px(120.));
        }
    });
}
