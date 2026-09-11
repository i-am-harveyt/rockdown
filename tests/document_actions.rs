use gpui::{TestAppContext, VisualTestContext};
use rockdown::{app::Workspace, config::Config, document::Document, explorer::Explorer};

fn workspace(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    VisualTestContext,
    gpui::Entity<Workspace>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled(""),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    (dir, window.clone(), view)
}

#[gpui::test]
fn save_dialog_targets_original_buffer_and_cancel_preserves_edits(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("original")
        })
    });
    window.simulate_keystrokes("cmd-s");
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.new_document();
            app.documents.current_mut().buffer.insert_text("second");
        })
    });
    let path = dir.path().join("original.md");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), "second");
        assert!(app.documents.current().path.is_none());
    });
    window.simulate_keystrokes("cmd-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| None);
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(app.documents.current().buffer.dirty());
        assert_eq!(app.message, "Save cancelled");
    });
}

#[gpui::test]
fn close_save_cancel_keeps_tab_and_close_window_checks_inactive_buffers(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("keep")
        })
    });
    window.simulate_keystrokes("cmd-w");
    window.run_until_parked();
    cx.simulate_prompt_answer("Save");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| None);
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            assert_eq!(app.documents.current().buffer.text(), "keep");
            app.documents.new_document();
        })
    });
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            assert!(!app.request_window_close(window, cx));
        })
    });
    window.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.entries().len(), 2);
        assert!(app.documents.dirty());
        assert_eq!(app.message, "Close cancelled");
    });
}

#[gpui::test]
fn window_close_protects_staged_explorer_changes(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.explorer.buffer.insert_text("new.md");
            assert!(app.explorer.dirty());
            app.request_window_close(window, cx);
        })
    });
    window.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    window.update(|_, cx| assert!(view.read(cx).explorer.dirty()));
}

#[gpui::test]
fn closing_save_failure_keeps_original_buffer_and_other_file(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    let owned = dir.path().join("owned.md");
    std::fs::write(&owned, "disk").unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("unsaved");
            let id = app.documents.active_id();
            app.documents.open(&owned).unwrap();
            app.documents.select(id).unwrap();
        })
    });
    window.simulate_keystrokes("cmd-w");
    window.run_until_parked();
    cx.simulate_prompt_answer("Save");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(owned.clone()));
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(owned).unwrap(), "disk");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.entries().len(), 2);
        assert_eq!(app.documents.current().buffer.text(), "unsaved");
        assert!(app.message.contains("another buffer"));
    });
}

#[gpui::test]
fn cancelling_later_close_prompt_preserves_previously_discarded_buffers(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.documents.current_mut().buffer.insert_text("first");
            app.documents.new_document();
            app.documents.current_mut().buffer.insert_text("second");
            app.request_window_close(window, cx);
        })
    });
    window.run_until_parked();
    cx.simulate_prompt_answer("Discard");
    window.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.entries().len(), 2);
        assert!(
            app.documents
                .entries()
                .iter()
                .all(|entry| entry.document.buffer.dirty())
        );
    });
}

#[gpui::test]
fn discard_rechecks_edits_made_while_prompt_is_pending(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("first")
        })
    });
    window.simulate_keystrokes("cmd-w");
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.key("A");
            app.documents.current_mut().buffer.insert_text(" later");
        })
    });
    cx.simulate_prompt_answer("Discard");
    window.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    window.update(|_, cx| {
        assert_eq!(
            view.read(cx).documents.current().buffer.text(),
            "first later"
        );
    });
}

#[gpui::test]
fn save_as_refreshes_clean_files_and_preserves_staging(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    window.simulate_keystrokes("cmd-shift-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(dir.path().join("created.md")));
    window.run_until_parked();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            assert!(app.explorer.buffer.text().contains("created.md"));
            assert!(!app.explorer.dirty());
            app.explorer.buffer.insert_text("staged-");
        })
    });
    let staged = window.update(|_, cx| view.read(cx).explorer.buffer.text());
    window.simulate_keystrokes("cmd-shift-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(dir.path().join("copy.md")));
    window.run_until_parked();
    assert!(dir.path().join("copy.md").exists());
    window.update(|_, cx| {
        assert_eq!(view.read(cx).explorer.buffer.text(), staged);
        assert!(view.read(cx).explorer.dirty());
    });
}

#[gpui::test]
fn files_refresh_failure_does_not_turn_successful_save_into_failure(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    let destination = tempfile::tempdir().unwrap();
    window.update(|_, cx| {
        view.update(cx, |app, _| {
            app.documents.current_mut().buffer.insert_text("# Saved")
        })
    });
    // Saving elsewhere still succeeds after the Files directory disappears.
    std::fs::remove_dir(dir.path()).unwrap();
    window.simulate_keystrokes("cmd-s");
    window.run_until_parked();
    let path = destination.path().join("note.md");
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    window.run_until_parked();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Saved");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.documents.current().buffer.dirty());
        assert!(!app.projection.is_empty());
        assert!(app.message.starts_with("Saved "));
        assert!(app.message.contains("Files refresh failed"));
    });
}
