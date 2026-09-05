use gpui::{
    ClipboardItem, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, px, size,
};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
};

const TABLE: &str = "outside\n| Name | Description | Count |\n| :--- | :---: | ---: |\n| **alpha** | A longer description with many words that must wrap independently inside its own column without shifting other columns or losing its text. | 12 |\n| 世界 | tiny | 999999 |\n| missing | | |\n\nafter\n";

fn table_window(cx: &mut TestAppContext) -> (VisualTestContext, gpui::Entity<Workspace>) {
    let sandbox = tempfile::tempdir().unwrap();
    let directory = sandbox.path().to_path_buf();
    std::mem::forget(sandbox);
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled(TABLE),
            Explorer::open(&directory).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    (window, view)
}

/// (source_row, height) pairs for the editor pane's hit rows.
fn hit_rows(window: &mut VisualTestContext, view: &gpui::Entity<Workspace>) -> Vec<(usize, f32)> {
    window.update(|_, cx| {
        view.read(cx).layouts[Pane::Editor.index()]
            .rows
            .iter()
            .map(|row| (row.source_row, f32::from(row.height)))
            .collect()
    })
}

#[gpui::test]
fn cmd_v_pastes_clipboard_into_the_editor(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("pasted text".into()));
    window.simulate_keystrokes("cmd-v");
    window.run_until_parked();
    assert!(window.update(|_, cx| {
        view.read(cx)
            .documents
            .current()
            .buffer
            .text()
            .contains("pasted text")
    }));
    // Insert mode: the paste joins the current undo transaction at the caret.
    window.simulate_keystrokes("escape i");
    cx.write_to_clipboard(ClipboardItem::new_string(" twice".into()));
    window.simulate_keystrokes("cmd-v");
    window.run_until_parked();
    assert!(window.update(|_, cx| {
        view.read(cx)
            .documents
            .current()
            .buffer
            .text()
            .contains(" twice")
    }));
}

#[gpui::test]
fn explorer_yank_uses_clipboard_across_panes_and_reload(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            std::fs::write(app.explorer.directory.join("note.md"), "").unwrap();
            app.explorer.reload().unwrap();
            cx.notify();
        });
    });
    cx.write_to_clipboard(ClipboardItem::new_string("old clipboard".into()));
    window.simulate_keystrokes("ctrl-w l y y ctrl-w h p");
    window.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "note.md\n"
    );
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        TABLE.replacen("outside\n", "outside\nnote.md\n", 1)
    );
    // Refresh replaces the explorer buffer, but must not lose the yank.
    window.simulate_keystrokes("ctrl-w l : e enter p");
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).explorer.buffer.text()),
        "note.md\nnote.md"
    );
    window.simulate_keystrokes("u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).explorer.buffer.text()),
        "note.md"
    );
}

#[gpui::test]
fn vim_paste_uses_latest_clipboard_and_preserves_characterwise_yanks(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    window.simulate_keystrokes("v l y");
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "ou");
    window.simulate_keystrokes("ctrl-w l p");
    assert_eq!(
        window.update(|_, cx| view.read(cx).explorer.buffer.text()),
        "ou"
    );
    // A copy from another application takes precedence over the last Vim yank.
    cx.write_to_clipboard(ClipboardItem::new_string("NEW".into()));
    window.simulate_keystrokes("ctrl-w h shift-p");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        format!("NEW{TABLE}")
    );
    window.simulate_keystrokes("u");
    cx.write_to_clipboard(ClipboardItem::new_string("".into()));
    window.simulate_keystrokes("p");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        TABLE
    );
}

#[gpui::test]
fn table_rows_render_as_grid_and_click_selects_source_row(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    // One hit row per source line; every table row (including the delimiter)
    // participates in click mapping. The trailing newline yields a final
    // empty line.
    let rows = hit_rows(&mut window, &view);
    assert_eq!(
        rows.iter().map(|(row, _)| *row).collect::<Vec<_>>(),
        (0..9).collect::<Vec<_>>()
    );
    // Every table row (header, delimiter rule, and body) renders with at
    // least one line of height.
    assert!(rows[..7].iter().all(|(_, height)| *height >= 30.));

    // Clicking inside the first rendered body row activates its raw source.
    let (origin, height) = window.update(|_, cx| {
        let row = view.read(cx).layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 3)
            .unwrap();
        (row.origin, row.height)
    });
    window.simulate_mouse_down(
        Point::new(origin.x + px(30.), origin.y + height / 2.),
        MouseButton::Left,
        Modifiers::default(),
    );
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.row),
        3
    );

    // Editing keys operate on the now-raw active row; undo restores.
    window.simulate_keystrokes("i");
    window.simulate_input("X");
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert!(window.update(|_, cx| {
        view.read(cx).documents.current().buffer.lines[3]
            .text
            .starts_with("X| **alpha**")
    }));
    window.simulate_keystrokes("u");
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.lines[3]
            .text
            .clone()),
        "| **alpha** | A longer description with many words that must wrap independently inside its own column without shifting other columns or losing its text. | 12 |"
    );
}

#[gpui::test]
fn image_rows_reserve_fitted_space_and_never_overlap_following_text(cx: &mut TestAppContext) {
    // 4x2 red PNG: width-driven sizing gives height = width / 2.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x04\x00\x00\x00\x02\x08\x06\x00\x00\x00\x7f\xa8}c\x00\x00\x00\x12IDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x193\xa0\x0b\x00\x00\x0f!\x0f\xf1\x047\xc6\x9f\x00\x00\x00\x00IEND\xaeB`\x82";
    let sandbox = tempfile::tempdir().unwrap();
    let directory = sandbox.path().to_path_buf();
    std::fs::write(directory.join("pic.png"), PNG).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled("before\n![](pic.png)\nafter\n"),
            Explorer::open(&directory).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();

    let layout = |window: &mut VisualTestContext| {
        window.update(|_, cx| {
            view.read(cx).layouts[Pane::Editor.index()]
                .rows
                .iter()
                .map(|row| {
                    (
                        row.source_row,
                        f32::from(row.origin.y),
                        f32::from(row.height),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    let rows = layout(&mut window);
    let image_row = rows.iter().find(|(row, _, _)| *row == 1).unwrap();
    let after = rows.iter().find(|(row, _, _)| *row == 2).unwrap();
    // The row reserves the image's fitted height (width / aspect) beyond the
    // one text line it occupies in the buffer.
    assert!(image_row.2 > 2. * 30., "image row reserves space: {rows:?}");
    // The next row starts exactly at the image row's bottom edge.
    assert!((after.1 - (image_row.1 + image_row.2)).abs() < 0.5);

    // Width-driven: a narrower pane shrinks the reserved image height.
    window.simulate_resize(size(px(700.), px(900.)));
    window.run_until_parked();
    let narrow = layout(&mut window);
    let narrow_image = narrow.iter().find(|(row, _, _)| *row == 1).unwrap();
    assert!(narrow_image.2 < image_row.2, "narrow pane: {narrow:?}");
}

#[gpui::test]
fn narrow_viewport_wraps_cells_and_keeps_shared_column_widths(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    let wide_heights = hit_rows(&mut window, &view);

    window.simulate_resize(size(px(700.), px(900.)));
    window.run_until_parked();
    let narrow_heights = hit_rows(&mut window, &view);
    // Less width means more wrapping for the long description cell.
    assert!(narrow_heights[3].1 > wide_heights[3].1);

    // Scrolling past the hidden header keeps row heights stable because column
    // widths are measured once per table and cached across rows.
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.row = 5;
            app.follow_cursor = false;
        });
    });
    window.run_until_parked();
    let scrolled = hit_rows(&mut window, &view);
    assert_eq!(scrolled[3].1, narrow_heights[3].1);
}

#[gpui::test]
fn large_image_is_downscaled_and_renders_proportionally(cx: &mut TestAppContext) {
    let sandbox = tempfile::tempdir().unwrap();
    let directory = sandbox.path().to_path_buf();
    // 2400x1200 image (2:1 aspect ratio)
    let img = image::RgbImage::new(2400, 1200);
    img.save(directory.join("large.png")).unwrap();

    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled("before\n![](large.png)\nafter\n"),
            Explorer::open(&directory).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.run_until_parked();

    let rows = window.update(|_, cx| {
        view.read(cx).layouts[Pane::Editor.index()]
            .rows
            .iter()
            .map(|row| {
                (
                    row.source_row,
                    f32::from(row.origin.y),
                    f32::from(row.height),
                )
            })
            .collect::<Vec<_>>()
    });
    let image_row = rows.iter().find(|(row, _, _)| *row == 1).unwrap();
    let after = rows.iter().find(|(row, _, _)| *row == 2).unwrap();
    assert!(image_row.2 > 100.0);
    assert!((after.1 - (image_row.1 + image_row.2)).abs() < 0.5);
}
