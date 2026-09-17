use gpui::{ClipboardItem, Entity, TestAppContext, VisualTestContext};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    vim::Mode,
};
use std::path::Path;

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
    source: &str,
    config: Config,
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            config,
            None,
            Document::untitled(source),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    window.run_until_parked();
    (dir, window, view)
}

fn select(
    window: &mut VisualTestContext,
    view: &Entity<Workspace>,
    anchor: (usize, usize),
    head: (usize, usize),
) {
    window.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.documents
                .current_mut()
                .buffer
                .select_with_mouse(anchor, head);
            cx.notify();
        });
    });
}

fn text(window: &mut VisualTestContext, view: &Entity<Workspace>) -> String {
    window.update(|_, cx| view.read(cx).documents.current().buffer.text())
}

#[gpui::test]
fn default_shortcuts_toggle_selected_text_without_touching_clipboard(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, "hello", Config::default());
    window.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("clipboard".into())));
    for (key, expected) in [
        ("b", "**hello**"),
        ("i", "*hello*"),
        ("u", "<u>hello</u>"),
        ("shift-x", "~~hello~~"),
        ("shift-c", "`hello`"),
    ] {
        select(&mut window, &view, (0, 4), (0, 0));
        window.simulate_keystrokes(&shortcut(key));
        assert_eq!(text(&mut window, &view), expected);
        window.update(|_, cx| {
            let buffer = &view.read(cx).documents.current().buffer;
            let range = buffer.selected_range(0).unwrap();
            assert_eq!(&buffer.lines[0].text[range.clone()], "hello");
            assert_eq!(buffer.col, range.start);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("clipboard")
            );
        });
        window.simulate_keystrokes(&shortcut(key));
        assert_eq!(text(&mut window, &view), "hello");
    }
}

#[gpui::test]
fn formatting_shortcuts_are_configurable(cx: &mut TestAppContext) {
    let config = Config::parse(
        Path::new("config.toml"),
        "[keys]\nf2 = 'bold'\nf3 = 'italic'\nf4 = 'underline'\nf5 = 'strikethrough'\nf6 = 'inline-code'",
    ).unwrap();
    config.validate().unwrap();
    let (_dir, mut window, view) = workspace(cx, "word", config);
    for (key, expected) in [
        ("f2", "**word**"),
        ("f3", "*word*"),
        ("f4", "<u>word</u>"),
        ("f5", "~~word~~"),
        ("f6", "`word`"),
    ] {
        select(&mut window, &view, (0, 0), (0, 3));
        window.simulate_keystrokes(key);
        assert_eq!(text(&mut window, &view), expected);
        window.simulate_keystrokes(key);
        assert_eq!(text(&mut window, &view), "word");
    }
}

#[gpui::test]
fn multiline_formatting_is_one_undoable_edit(cx: &mut TestAppContext) {
    let source = " first  \n\nsecond ";
    let (_dir, mut window, view) = workspace(cx, source, Config::default());
    window.simulate_keystrokes("V G");
    window.simulate_keystrokes(&shortcut("b"));
    let formatted = " **first**  \n\n**second** ";
    assert_eq!(text(&mut window, &view), formatted);
    window.simulate_keystrokes("escape u");
    assert_eq!(text(&mut window, &view), source);
    window.simulate_keystrokes("ctrl-r");
    assert_eq!(text(&mut window, &view), formatted);
}

#[gpui::test]
fn formatting_requires_selection_and_markdown_editor_context(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, "word", Config::default());
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "word");
    window.simulate_keystrokes("i");
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "word");
    window.simulate_keystrokes("escape");
    select(&mut window, &view, (0, 0), (0, 3));

    for context in ["plain", "command", "ime", "explorer", "terminal"] {
        let files = window.update(|_, cx| {
            view.update(cx, |app, _| {
                match context {
                    "plain" => app.documents.current_mut().path = Some("note.txt".into()),
                    "command" => app.command = Some(":write".into()),
                    "ime" => app.marked = Some(0..4),
                    "explorer" => {
                        app.pane = Pane::Explorer;
                        app.explorer.buffer.set_text("file.md");
                        app.explorer.buffer.select_with_mouse((0, 0), (0, 6));
                    }
                    "terminal" => app.pane = Pane::Terminal,
                    _ => unreachable!(),
                }
                app.refresh_projection();
                app.explorer.buffer.text()
            })
        });
        window.simulate_keystrokes(&shortcut("b"));
        assert_eq!(text(&mut window, &view), "word", "{context}");
        window.update(|_, cx| {
            view.update(cx, |app, _| {
                assert_eq!(app.explorer.buffer.text(), files);
                if context == "command" {
                    assert_eq!(app.command.as_deref(), Some(":write"));
                }
                if context == "ime" {
                    assert_eq!(app.marked, Some(0..4));
                }
                app.documents.current_mut().path = None;
                app.command = None;
                app.marked = None;
                app.pane = Pane::Editor;
                app.refresh_projection();
            });
        });
    }
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "**word**");
}

#[gpui::test]
fn formatting_does_not_edit_behind_overlays_or_native_dialogs(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, "word", Config::default());
    select(&mut window, &view, (0, 0), (0, 3));
    for open in [
        "f1".to_owned(),
        shortcut("shift-t"),
        shortcut("shift-o"),
        shortcut("shift-m"),
    ] {
        window.simulate_keystrokes(&open);
        window.simulate_keystrokes(&shortcut("b"));
        assert_eq!(text(&mut window, &view), "word", "{open}");
        window.simulate_keystrokes("escape");
    }
    window.simulate_keystrokes(&shortcut("s"));
    window.run_until_parked();
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "word");
    cx.simulate_new_path_selection(|_| None);
    window.run_until_parked();
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "**word**");
}

#[gpui::test]
fn formatting_rejects_selected_code_blocks_but_not_adjacent_prose(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, "", Config::default());
    for source in [
        "prose\n\n```rust\nlet x = 1;\n```\n\nafter",
        "prose\n\n    let x = 1;\n\nafter",
        "prose\n\n> ```\n> code\n> ```\n\nafter",
    ] {
        window.update(|_, cx| {
            view.update(cx, |app, _| {
                app.documents.current_mut().buffer.set_text(source);
                app.refresh_projection();
            });
        });
        window.simulate_keystrokes("g g V G");
        window.simulate_keystrokes(&shortcut("b"));
        assert_eq!(text(&mut window, &view), source);
        window.simulate_keystrokes("escape g g v e");
        window.simulate_keystrokes(&shortcut("b"));
        assert_eq!(
            text(&mut window, &view),
            source.replacen("prose", "**prose**", 1)
        );
        window.update(|_, cx| {
            assert_eq!(view.read(cx).documents.current().buffer.mode, Mode::Visual);
        });
    }
}

#[gpui::test]
fn writer_tabs_protect_the_selection_then_format_when_dismissed(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx, "word", Config::default());
    select(&mut window, &view, (0, 0), (0, 3));
    window.simulate_keystrokes(&shortcut("shift-m"));
    window.simulate_keystrokes("down enter");
    window.simulate_keystrokes(&shortcut("alt-t"));
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "word");
    window.simulate_keystrokes("escape");
    window.simulate_keystrokes(&shortcut("b"));
    assert_eq!(text(&mut window, &view), "**word**");
}
