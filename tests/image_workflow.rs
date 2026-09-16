use gpui::{
    ClipboardItem, Entity, Image, ImageFormat, TestAppContext, VisualTestContext, px, size,
};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    image_assets::local_image_path,
    markdown,
    vim::Mode,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

const NATIVE_PASTE: &str = if cfg!(target_os = "macos") {
    "cmd-v"
} else {
    "ctrl-v"
};

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        120,
        80,
        image::Rgba([20, 100, 180, 255]),
    ))
    .write_to(&mut bytes, image::ImageFormat::Png)
    .unwrap();
    bytes.into_inner()
}

fn editor(
    cx: &mut TestAppContext,
    source: &str,
    extension: Option<&str>,
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let document = if let Some(extension) = extension {
        let path = dir.path().join(format!("note.{extension}"));
        std::fs::write(&path, source).unwrap();
        Document::open(&path).unwrap()
    } else {
        Document::untitled(source)
    };
    let (view, window) = cx.add_window_view(|window, cx| {
        let mut config = Config::default();
        // Exercise native paste directly; Ctrl-Shift-V still uses the Paste action.
        config.keys.remove(NATIVE_PASTE);
        Workspace::new(
            config,
            None,
            document,
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    window.simulate_resize(size(px(900.), px(600.)));
    window.run_until_parked();
    (dir, window, view)
}

fn put_image(window: &mut VisualTestContext) {
    window.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            ImageFormat::Png,
            png(),
        )));
    });
}

fn text(window: &mut VisualTestContext, view: &Entity<Workspace>) -> String {
    window.update(|_, cx| view.read(cx).documents.current().buffer.text())
}

fn assets(base: &Path, source: &str) -> Vec<PathBuf> {
    markdown::project(source, &Config::default().theme)
        .iter()
        .flat_map(|line| &line.images)
        .map(|image| {
            assert!(!Path::new(&image.url).is_absolute());
            local_image_path(base, &image.url).unwrap()
        })
        .collect()
}

#[gpui::test]
fn clipboard_action_imports_portable_preview_and_one_undo_redo(cx: &mut TestAppContext) {
    let original = "\n\nafter";
    let (dir, mut window, view) = editor(cx, original, Some("md"));
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    let inserted = text(&mut window, &view);
    let paths = assets(dir.path(), &inserted);
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), png());
    assert_eq!(paths[0].parent().unwrap(), dir.path().join("assets"));
    window.update(|_, cx| {
        assert!(
            view.read(cx)
                .explorer
                .buffer
                .lines
                .iter()
                .any(|line| line.text == "assets/"),
            "Files must show the new assets directory without a manual reload"
        );
    });
    window.simulate_keystrokes("j j");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let rows = &app.layouts[Pane::Editor.index()].rows;
        let image = rows.iter().find(|row| row.source_row == 0).unwrap();
        let following = rows.iter().find(|row| row.source_row == 1).unwrap();
        assert!(!image.raw);
        assert!(f32::from(image.height) > app.config.line_height * 2.);
        assert!(following.origin.y >= image.origin.y + image.height);
        assert_eq!(app.documents.current().buffer.mode, Mode::Normal);
    });
    window.simulate_keystrokes("cmd-s");
    window.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        inserted
    );
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), original);
    assert!(
        paths[0].exists(),
        "undo must retain assets potentially used elsewhere"
    );
    window.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&mut window, &view), inserted);
}

#[gpui::test]
fn direct_clipboard_shortcut_isolates_image_from_insert_typing(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "end", Some("md"));
    window.simulate_keystrokes("i");
    window.simulate_input("before ");
    put_image(&mut window);
    window.simulate_keystrokes(NATIVE_PASTE);
    window.run_until_parked();
    let inserted = text(&mut window, &view);
    assert!(inserted.starts_with("before !["));
    assert!(inserted.ends_with("end"));
    assert_eq!(assets(dir.path(), &inserted).len(), 1);
    window.update(|_, cx| assert_eq!(view.read(cx).buffer().mode, Mode::Insert));
    window.simulate_input("after ");
    window.simulate_keystrokes("escape u");
    assert_eq!(text(&mut window, &view), inserted);
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), "before end");
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), "end");
}

#[gpui::test]
fn untitled_cancel_then_save_imports_original_tab_without_moving_active_view(
    cx: &mut TestAppContext,
) {
    let (dir, mut window, view) = editor(cx, "original", None);
    let original_id = window.update(|_, cx| view.read(cx).documents.active_id());
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| None);
    window.run_until_parked();
    assert_eq!(text(&mut window, &view), "original");
    assert!(!dir.path().join("assets").exists());
    window.update(|_, cx| assert!(view.read(cx).documents.current().path.is_none()));

    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    let other = dir.path().join("other.md");
    std::fs::write(
        &other,
        (0..100).map(|i| format!("row {i}\n")).collect::<String>(),
    )
    .unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents.open(&other).unwrap();
            app.documents.current_mut().buffer.row = 40;
            app.tops[0] = 35;
            app.scroll_offsets[0] = 9.;
            app.follow_cursor = false;
            app.refresh_projection();
            cx.notify();
        });
    });
    window.run_until_parked();
    let before = window.update(|_, cx| {
        let app = view.read(cx);
        (
            app.documents.active_id(),
            app.buffer().text(),
            app.buffer().row,
            app.tops[0],
            app.scroll_offsets[0],
        )
    });
    let saved = dir.path().join("original.md");
    cx.simulate_new_path_selection(|_| Some(saved.clone()));
    window.run_until_parked();
    let imported = window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            (
                app.documents.active_id(),
                app.buffer().text(),
                app.buffer().row,
                app.tops[0],
                app.scroll_offsets[0]
            ),
            before
        );
        let document = app.documents.get(original_id).unwrap();
        assert!(document.buffer.dirty());
        document.buffer.text()
    });
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "original");
    assert!(imported.ends_with("original"));
    let paths = assets(dir.path(), &imported);
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), png());
    // Select through the command path, which refreshes the original tab's projection.
    window.simulate_keystrokes(":");
    window.simulate_input("bp");
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    window.update(|_, cx| assert_eq!(view.read(cx).documents.active_id(), original_id));
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), "original");
    window.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&mut window, &view), imported);
    assert!(paths[0].exists());
}

#[gpui::test]
fn common_drop_entrypoint_copies_collisions_and_rolls_back_invalid_batch(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "", Some("md"));
    let source = dir.path().join("画 面#%()[alt].png");
    let bytes = png();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let existing = dir.path().join("assets").join(source.file_name().unwrap());
    std::fs::write(&existing, b"existing asset must not be overwritten").unwrap();
    let drop_files = |window: &mut VisualTestContext, paths: &[PathBuf]| {
        window.update(|window, cx| {
            view.update(cx, |app, cx| app.import_image_files(paths, window, cx))
        });
        window.run_until_parked();
    };
    drop_files(&mut window, &[source.clone(), source.clone()]);
    let inserted = text(&mut window, &view);
    let paths = assets(dir.path(), &inserted);
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], paths[1]);
    assert_ne!(paths[0], existing);
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"existing asset must not be overwritten"
    );
    assert!(inserted.contains("%20") && inserted.contains("%23") && inserted.contains("%25"));
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    for path in &paths {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let invalid = dir.path().join("broken.png");
    std::fs::write(&invalid, b"not an image").unwrap();
    drop_files(&mut window, &[source.clone(), invalid]);
    assert_eq!(text(&mut window, &view), inserted);
    assert_eq!(
        std::fs::read_dir(dir.path().join("assets"))
            .unwrap()
            .count(),
        2
    );
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), "");
    window.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&mut window, &view), inserted);
    for path in paths {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[gpui::test]
fn image_import_refreshes_open_assets_directory(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "", Some("md"));
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir(&assets_dir).unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.explorer = Explorer::open(&assets_dir).unwrap();
        });
    });
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    let paths = assets(dir.path(), &text(&mut window, &view));
    assert_eq!(paths.len(), 1);
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.explorer.directory,
            std::fs::canonicalize(&assets_dir).unwrap()
        );
        assert!(
            app.explorer
                .buffer
                .lines
                .iter()
                .any(|line| { line.text == paths[0].file_name().unwrap().to_str().unwrap() })
        );
        assert!(!app.explorer.dirty());
    });
}

#[gpui::test]
fn image_import_preserves_concurrent_explorer_edits_and_undo(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "", Some("md"));
    let source = dir.path().join("source.png");
    std::fs::write(&source, png()).unwrap();
    let staged = window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.import_image_files(&[source], window, cx);
            app.explorer.buffer.insert_text("renamed-");
            assert!(app.explorer.dirty());
            app.explorer.buffer.text()
        })
    });
    window.run_until_parked();
    let paths = assets(dir.path(), &text(&mut window, &view));
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), png());
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            assert_eq!(app.explorer.buffer.text(), staged);
            assert!(app.explorer.dirty());
            app.explorer.buffer.key("u");
            assert_eq!(app.explorer.buffer.text(), "note.md");
            assert!(!app.explorer.dirty());
            app.explorer.reload().unwrap();
            assert!(
                app.explorer
                    .buffer
                    .lines
                    .iter()
                    .any(|line| line.text == "assets/")
            );
        });
    });
    assert!(dir.path().join("note.md").exists());
    assert!(!dir.path().join("renamed-note.md").exists());
}

#[gpui::test]
fn rejected_contexts_and_non_markdown_save_never_create_assets(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "literal", Some("txt"));
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    assert_eq!(text(&mut window, &view), "literal");
    assert!(!dir.path().join("assets").exists());
    window.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("![x](literal.png)".into()))
    });
    window.simulate_keystrokes("ctrl-shift-v");
    assert_eq!(text(&mut window, &view), "![x](literal.png)literal");
    assert!(!dir.path().join("assets").exists());

    let (dir, mut window, view) = editor(cx, "untitled", None);
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    let path = dir.path().join("wrong.txt");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    window.run_until_parked();
    assert!(!path.exists());
    assert!(!dir.path().join("assets").exists());
    assert_eq!(text(&mut window, &view), "untitled");
    window.update(|_, cx| assert!(view.read(cx).documents.current().path.is_none()));
    window.simulate_keystrokes("cmd-shift-o");
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    assert_eq!(text(&mut window, &view), "untitled");
    assert!(!dir.path().join("assets").exists());
}

#[gpui::test]
fn pending_import_rejects_changed_document_and_other_buffer_owned_path(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "original", None);
    put_image(&mut window);
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("changed ")
        });
    });
    let changed_path = dir.path().join("changed.md");
    cx.simulate_new_path_selection(|_| Some(changed_path.clone()));
    window.run_until_parked();
    assert!(!changed_path.exists());
    assert!(!dir.path().join("assets").exists());
    assert_eq!(text(&mut window, &view), "changed original");

    let owned = dir.path().join("owned.md");
    std::fs::write(&owned, "do not overwrite").unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            let id = app.documents.active_id();
            app.documents.open(&owned).unwrap();
            app.documents.select(id).unwrap();
        });
    });
    window.simulate_keystrokes("ctrl-shift-v");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(owned.clone()));
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(&owned).unwrap(), "do not overwrite");
    assert!(!dir.path().join("assets").exists());
    assert_eq!(text(&mut window, &view), "changed original");
}

#[gpui::test]
fn logically_named_markdown_imports_without_prompt_or_initial_save(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "", None);
    let path = dir.path().join("not-yet-saved.md");
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.documents.open(&path).unwrap();
            let source = dir.path().join("source.png");
            std::fs::write(&source, png()).unwrap();
            app.import_image_files(&[source], window, cx);
        });
    });
    window.run_until_parked();
    let inserted = text(&mut window, &view);
    let paths = assets(dir.path(), &inserted);
    assert_eq!(paths.len(), 1);
    assert_eq!(std::fs::read(&paths[0]).unwrap(), png());
    assert!(
        !path.exists(),
        "an already named buffer does not need a Save As dialog"
    );
}

#[gpui::test]
fn direct_paste_keeps_outline_text_literal_and_rejects_images(cx: &mut TestAppContext) {
    let source = "intro\n## 中文目的地";
    let (dir, mut window, view) = editor(cx, source, Some("md"));
    window.simulate_keystrokes("cmd-shift-o");
    put_image(&mut window);
    window.simulate_keystrokes(NATIVE_PASTE);
    window.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("中文".into())));
    window.simulate_keystrokes(NATIVE_PASTE);
    window.simulate_keystrokes("enter");
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.row, 1);
        assert_eq!(buffer.text(), source);
        assert!(!buffer.dirty());
    });
    assert!(!dir.path().join("assets").exists());
}

#[gpui::test]
fn import_completion_uses_original_document_and_caret(cx: &mut TestAppContext) {
    let (dir, mut window, view) = editor(cx, "before\nafter", Some("md"));
    let source = dir.path().join("source.png");
    std::fs::write(&source, png()).unwrap();
    let original_id = window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.documents.current_mut().buffer.row = 1;
            let id = app.documents.active_id();
            app.import_image_files(&[source], window, cx);
            assert_eq!(app.buffer().text(), "before\nafter");
            app.documents.current_mut().buffer.row = 0;
            app.documents.new_document();
            app.documents
                .current_mut()
                .buffer
                .insert_text("other edits");
            app.refresh_projection();
            id
        })
    });
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            assert_eq!(app.buffer().text(), "other edits");
            let current_caret = (app.buffer().row, app.buffer().col);
            let original = &app.documents.get(original_id).unwrap().buffer;
            assert!(original.text().starts_with("before\n!["));
            assert!(original.text().ends_with("after"));
            let active = app.documents.active_id();
            app.documents.select(original_id).unwrap();
            app.documents.current_mut().buffer.key("u");
            assert_eq!(app.buffer().text(), "before\nafter");
            app.documents.select(active).unwrap();
            assert_eq!((app.buffer().row, app.buffer().col), current_caret);
        });
    });
}

#[gpui::test]
fn import_completion_rejects_edits_close_reload_and_save_as(cx: &mut TestAppContext) {
    for change in ["edit", "close", "reload", "save-as"] {
        let (dir, mut window, view) = editor(cx, "original", Some("md"));
        let source = dir.path().join("source.png");
        std::fs::write(&source, png()).unwrap();
        window.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.import_image_files(&[source], window, cx);
                assert_eq!(app.buffer().text(), "original");
                match change {
                    "edit" => app
                        .documents
                        .current_mut()
                        .buffer
                        .insert_text("concurrent "),
                    "close" => app.documents.delete(true).unwrap(),
                    "reload" => app.documents.reload(true).unwrap(),
                    "save-as" => app
                        .documents
                        .save(Some(&dir.path().join("moved.md")), false)
                        .unwrap(),
                    _ => unreachable!(),
                }
                app.refresh_projection();
            });
        });
        let expected = text(&mut window, &view);
        window.run_until_parked();
        window.update(|_, cx| {
            let app = view.read(cx);
            assert_eq!(app.buffer().text(), expected, "{change}");
            assert!(
                app.explorer
                    .buffer
                    .lines
                    .iter()
                    .any(|line| line.text == "assets/"),
                "Files must refresh copied assets even when insertion is rejected: {change}"
            );
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
            "original"
        );
        assert_eq!(
            std::fs::read(dir.path().join("assets/source.png")).unwrap(),
            png()
        );
    }
}
