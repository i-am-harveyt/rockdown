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

const FILE_TYPE_TEXT: &str = "value = '''\n# Heading\n**bold** and [link](target)\n| Name | Value |\n| --- | --- |\n| a | b |\n---\n'''";

fn file_type_window(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    VisualTestContext,
    gpui::Entity<Workspace>,
) {
    let sandbox = tempfile::tempdir().unwrap();
    let path = sandbox.path().join("note.toml");
    std::fs::write(&path, FILE_TYPE_TEXT).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::open(&path).unwrap(),
            Explorer::open(sandbox.path()).unwrap(),
            window,
            cx,
        )
    });
    (sandbox, window.clone(), view)
}

fn file_command(window: &mut VisualTestContext, command: &str) {
    window.simulate_keystrokes(":");
    window.simulate_input(command);
    window.simulate_keystrokes("enter");
    window.run_until_parked();
}

fn assert_file_preview(
    window: &mut VisualTestContext,
    view: &gpui::Entity<Workspace>,
    markdown: bool,
) {
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), FILE_TYPE_TEXT);
        assert_eq!(
            app.layouts[Pane::Editor.index()]
                .rows
                .iter()
                .map(|row| (row.source_row, row.raw))
                .collect::<Vec<_>>(),
            (0..FILE_TYPE_TEXT.lines().count())
                .map(|row| (row, !markdown || row == 0))
                .collect::<Vec<_>>()
        );
    });
}

#[gpui::test]
fn named_plain_files_stay_literal_across_save_as_and_buffer_switches(cx: &mut TestAppContext) {
    let (sandbox, mut window, view) = file_type_window(cx);
    assert_file_preview(&mut window, &view, false);
    file_command(&mut window, "w note.md");
    assert_file_preview(&mut window, &view, true);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("note.md")).unwrap(),
        FILE_TYPE_TEXT
    );
    file_command(&mut window, "e note.toml");
    assert_file_preview(&mut window, &view, false);
    file_command(&mut window, "bp");
    assert_file_preview(&mut window, &view, true);
    file_command(&mut window, "w other.toml");
    assert_file_preview(&mut window, &view, false);
    file_command(&mut window, "w uppercase.MD");
    assert_file_preview(&mut window, &view, true);
    file_command(&mut window, "w extensionless");
    assert_file_preview(&mut window, &view, false);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("extensionless")).unwrap(),
        FILE_TYPE_TEXT
    );
}

#[gpui::test]
fn explorer_renames_and_deletion_update_preview_type(cx: &mut TestAppContext) {
    let (sandbox, mut window, view) = file_type_window(cx);
    for (name, markdown) in [("note.md", true), ("note.toml", false)] {
        window.simulate_keystrokes("ctrl-w l c c");
        window.simulate_input(name);
        window.simulate_keystrokes("escape : w enter ctrl-w h");
        window.run_until_parked();
        assert_file_preview(&mut window, &view, markdown);
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join(name)).unwrap(),
            FILE_TYPE_TEXT
        );
    }
    window.simulate_keystrokes("ctrl-w l d d : w enter ctrl-w h");
    window.run_until_parked();
    assert_file_preview(&mut window, &view, true);
    assert!(window.update(|_, cx| view.read(cx).documents.current().path.is_none()));
    assert!(!sandbox.path().join("note.toml").exists());
}

#[gpui::test]
fn save_as_refreshes_preview_even_when_explorer_reload_fails(cx: &mut TestAppContext) {
    let (sandbox, mut window, view) = file_type_window(cx);
    file_command(&mut window, "w note.md");
    assert_file_preview(&mut window, &view, true);
    let removed = sandbox.path().join("removed");
    std::fs::create_dir(&removed).unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.explorer = Explorer::open(&removed).unwrap();
        });
    });
    std::fs::remove_dir(&removed).unwrap();
    let target = sandbox.path().join("saved.toml");
    file_command(&mut window, &format!("w {}", target.display()));
    assert_file_preview(&mut window, &view, false);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), FILE_TYPE_TEXT);
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.documents.current().path.as_deref(),
            Some(std::fs::canonicalize(target).unwrap().as_path())
        );
    });
}

#[gpui::test]
fn large_underlined_headings_reserve_wrapped_space(cx: &mut TestAppContext) {
    let (mut window, view) = table_window(cx);
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents
                .current_mut()
                .buffer
                .set_text("before\n# A large heading that wraps across several words\nafter");
            app.config.markdown.h1.font_size = Some(64.);
            app.config.markdown.h1.underline = true;
            app.config.markdown.divider.thickness = 4.;
            app.refresh_projection();
            cx.notify();
        });
    });
    window.simulate_resize(size(px(900.), px(1200.)));
    window.run_until_parked();
    window.update(|_, cx| {
        let rows = &view.read(cx).layouts[Pane::Editor.index()].rows;
        let heading = rows.iter().find(|row| row.source_row == 1).unwrap();
        let after = rows.iter().find(|row| row.source_row == 2).unwrap();
        assert!(heading.height >= px(2. * 68. + 8.));
        assert!((f32::from(after.origin.y - heading.origin.y - heading.height)).abs() < 0.5);
        assert_eq!(after.line.text.as_ref(), "after");
    });
}

#[gpui::test]
fn viewport_alignment_centers_wrapped_rows_and_top_aligns_without_moving_cursor(
    cx: &mut TestAppContext,
) {
    let (mut window, view) = table_window(cx);
    let set_document = |window: &mut VisualTestContext, wrapped: bool| {
        window.update(|_, cx| {
            view.update(cx, |app, cx| {
                let text = (0..70)
                    .map(|row| {
                        if wrapped && row != 39 {
                            "wrapped words ".repeat(40)
                        } else {
                            format!("line {row}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                app.documents.current_mut().buffer.set_text(&text);
                app.refresh_projection();
                cx.notify();
            })
        });
        window.simulate_keystrokes("4 0 shift-g z z");
        window.run_until_parked();
    };
    let cursor_y = |window: &mut VisualTestContext| {
        window.update(|_, cx| {
            let app = view.read(cx);
            assert_eq!(app.documents.current().buffer.row, 39);
            f32::from(
                app.layouts[Pane::Editor.index()]
                    .rows
                    .iter()
                    .find(|row| row.source_row == 39)
                    .unwrap()
                    .origin
                    .y,
            )
        })
    };
    set_document(&mut window, false);
    let centered = cursor_y(&mut window);
    set_document(&mut window, true);
    assert!((cursor_y(&mut window) - centered).abs() < 1.);
    window.simulate_keystrokes("z t");
    window.run_until_parked();
    assert!(cursor_y(&mut window) < centered);
    assert_eq!(hit_rows(&mut window, &view)[0].0, 39);
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
