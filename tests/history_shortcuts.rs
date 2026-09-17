use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    vim::Mode,
};

fn shortcut(key: &str) -> String {
    format!(
        "{}-{key}",
        if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        }
    )
}

fn workspace(
    cx: &mut TestAppContext,
    config: Config,
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            config,
            None,
            Document::untitled(""),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    (dir, window.clone(), view)
}

fn text(window: &mut VisualTestContext, view: &Entity<Workspace>) -> String {
    window.update(|_, cx| view.read(cx).documents.current().buffer.text())
}

#[gpui::test]
fn insert_history_preserves_caret_and_separates_new_typing(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, Config::default());
    window.simulate_keystrokes("i");
    window.simulate_input("café👩‍💻");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.mode, Mode::Insert));
    window.simulate_keystrokes(&shortcut("shift-z"));
    window.simulate_input("!");
    assert_eq!(text(&mut window, &view), "café👩‍💻!");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "café👩‍💻");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "");
    window.simulate_keystrokes(&shortcut("shift-z"));
    window.simulate_input("?");
    window.simulate_keystrokes(&shortcut("shift-z"));
    assert_eq!(text(&mut window, &view), "café👩‍💻?");
    window.simulate_keystrokes("escape u ctrl-r");
    assert_eq!(text(&mut window, &view), "café👩‍💻?");
    window.simulate_keystrokes("u");
    assert_eq!(text(&mut window, &view), "café👩‍💻");
}

#[gpui::test]
fn history_refreshes_preview_and_clears_visual_selection(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, Config::default());
    window.simulate_keystrokes("i");
    window.simulate_input("**word**");
    window.simulate_keystrokes("enter");
    window.simulate_input("next");
    window.simulate_keystrokes("escape g g v");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.mode, Mode::Normal);
        assert!(app.documents.current().buffer.selected_range(0).is_none());
        assert!(app.projection[0].spans.is_empty());
    });
    window.simulate_keystrokes(&shortcut("shift-z"));
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), "**word**\nnext");
        assert!(
            app.projection[0]
                .spans
                .iter()
                .any(|span| span.bold && span.text == "word")
        );
    });
}

#[gpui::test]
fn history_is_scoped_to_active_document_or_staged_files(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx, Config::default());
    window.simulate_keystrokes("i");
    window.simulate_input("first");
    window.simulate_keystrokes("escape");
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents.new_document();
            app.documents.current_mut().path = Some(dir.path().join("second.txt"));
            app.refresh_projection();
            cx.notify();
        })
    });
    window.simulate_keystrokes("i");
    window.simulate_input("second");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "");
    window.simulate_keystrokes(&shortcut("shift-z"));
    assert_eq!(text(&mut window, &view), "second");
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents.previous();
            app.pane = Pane::Explorer;
            app.explorer_visible = true;
            cx.notify();
        })
    });
    let original = window.update(|_, cx| view.read(cx).explorer.buffer.text());
    window.simulate_keystrokes("i");
    window.simulate_input("draft.md");
    let edited = window.update(|_, cx| view.read(cx).explorer.buffer.text());
    window.simulate_keystrokes(&shortcut("z"));
    window.update(|_, cx| assert_eq!(view.read(cx).explorer.buffer.text(), original));
    assert_eq!(text(&mut window, &view), "first");
    window.simulate_keystrokes(&shortcut("shift-z"));
    window.update(|_, cx| assert_eq!(view.read(cx).explorer.buffer.text(), edited));
    assert!(!dir.path().join("draft.md").exists());
}

#[gpui::test]
fn history_does_not_edit_behind_input_contexts_and_dialogs(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, Config::default());
    window.simulate_keystrokes("i");
    window.simulate_input("draft");
    window.simulate_keystrokes("escape");
    for open in [
        "f1".to_owned(),
        shortcut("shift-t"),
        shortcut("shift-o"),
        shortcut("shift-m"),
        ":".to_owned(),
        "/".to_owned(),
    ] {
        window.simulate_keystrokes(&open);
        window.simulate_keystrokes(&shortcut("z"));
        window.simulate_keystrokes(&shortcut("shift-z"));
        assert_eq!(text(&mut window, &view), "draft", "{open}");
        window.simulate_keystrokes("escape");
    }
    window.simulate_keystrokes("A");
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "中文", None, window, cx);
        })
    });
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "draft中文");
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.replace_text_in_range(None, "中文", window, cx);
        })
    });
    window.simulate_keystrokes("escape");
    window.simulate_keystrokes(&shortcut("s"));
    window.run_until_parked();
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "draft中文");
    cx.simulate_new_path_selection(|_| None);
    window.run_until_parked();
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "draft");
}

#[gpui::test]
fn custom_history_bindings_share_vim_history(cx: &mut TestAppContext) {
    let config = Config {
        keys: [
            ("alt-z".into(), "undo".into()),
            ("alt-shift-z".into(), "redo".into()),
        ]
        .into(),
        ..Config::default()
    };
    config.validate().unwrap();
    let (_dir, mut window, view) = workspace(cx, config);
    window.simulate_keystrokes("i");
    window.simulate_input("word");
    window.simulate_keystrokes("escape alt-z");
    assert_eq!(text(&mut window, &view), "");
    window.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&mut window, &view), "word");
    window.simulate_keystrokes("u alt-shift-z");
    assert_eq!(text(&mut window, &view), "word");
}

#[gpui::test]
fn empty_history_keeps_insert_caret_and_platform_redo_works(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, Config::default());
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents.current_mut().buffer.set_text("界");
            app.refresh_projection();
            cx.notify();
        })
    });
    window.simulate_keystrokes("A");
    window.simulate_keystrokes(&shortcut("z"));
    window.simulate_keystrokes(&shortcut("shift-z"));
    window.simulate_input("!");
    assert_eq!(text(&mut window, &view), "界!");
    window.simulate_keystrokes(&shortcut("z"));
    assert_eq!(text(&mut window, &view), "界");
    window.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    });
    assert_eq!(text(&mut window, &view), "界!");
}

#[gpui::test]
fn history_keys_preserve_ime_composition_on_both_platform_keymaps(cx: &mut TestAppContext) {
    let mut config = Config::default();
    for (key, action) in [
        ("ctrl-z", "undo"),
        ("ctrl-shift-z", "redo"),
        ("ctrl-y", "redo"),
        ("cmd-z", "undo"),
        ("cmd-shift-z", "redo"),
    ] {
        config.keys.insert(key.into(), action.into());
    }
    let (_dir, mut window, view) = workspace(cx, config);
    window.simulate_keystrokes("i");
    window.simulate_input("draft");
    for key in ["ctrl-z", "ctrl-shift-z", "ctrl-y", "cmd-z", "cmd-shift-z"] {
        window.simulate_keystrokes("escape A");
        window.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.replace_and_mark_text_in_range(None, "中文", None, window, cx);
            })
        });
        window.simulate_keystrokes(key);
        window.update(|window, cx| {
            view.update(cx, |app, cx| {
                assert_eq!(app.marked_text_range(window, cx), Some(5..7), "{key}");
                app.replace_text_in_range(None, "中文", window, cx);
            })
        });
        assert_eq!(text(&mut window, &view), "draft中文", "{key}");
        window.simulate_keystrokes(&shortcut("z"));
        assert_eq!(text(&mut window, &view), "draft", "{key}");
    }
}
