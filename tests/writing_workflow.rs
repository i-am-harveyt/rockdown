use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, MouseButton, TestAppContext, VisualTestContext,
    point, px, size,
};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    vim::Mode,
};

fn editor(
    cx: &mut TestAppContext,
    source: &str,
    extension: &str,
    font_size: f32,
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("note.{extension}"));
    std::fs::write(&path, source).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config {
                writing_width: 320.,
                font_size,
                line_height: font_size + 15.,
                ..Default::default()
            },
            None,
            Document::open(&path).unwrap(),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    window.simulate_resize(size(px(1000.), px(900.)));
    window.run_until_parked();
    (dir, window, view)
}

#[gpui::test]
fn centered_wrapped_tasks_continue_undo_save_and_close_together(cx: &mut TestAppContext) {
    let source = format!("intro\n- [ ] {}", "café 世界 writing words ".repeat(10));
    let (dir, mut window, view) = editor(cx, &source, "md", 15.);
    window.simulate_keystrokes("i");
    let marker = window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.explorer_visible);
        let row = &app.layouts[Pane::Editor.index()].rows[1];
        assert!(row.origin.x > px(300.));
        assert!(row.wrapped.is_some());
        row.position_for_index(row.line.text.find("[ ]").unwrap() + 1) + point(px(1.), px(5.))
    });
    window.simulate_click(marker, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col, buffer.mode), (0, 0, Mode::Insert));
        assert!(buffer.lines[1].text.starts_with("- [x]"));
    });
    let (col, click) = window.update(|_, cx| {
        let app = view.read(cx);
        let row = &app.layouts[Pane::Editor.index()].rows[1];
        let display = row.line.text.rfind("café").unwrap();
        (
            app.projection[1].source_column(display),
            row.position_for_index(display) + point(px(1.), px(5.)),
        )
    });
    window.simulate_click(click, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(
            (buffer.row, buffer.col, buffer.mode),
            (1, col, Mode::Insert)
        );
        assert!(buffer.lines[1].text.starts_with("- [x]"));
    });
    window.simulate_keystrokes("escape A enter");
    window.simulate_input("next task");
    window.simulate_keystrokes("escape");
    let edited = window.update(|_, cx| view.read(cx).documents.current().buffer.text());
    assert!(edited.ends_with("\n- [ ] next task"));
    window.simulate_keystrokes("u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.lines.len()),
        2
    );
    window.simulate_keystrokes("ctrl-r cmd-w");
    window.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        edited
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        source
    );
    window.simulate_keystrokes("cmd-s");
    window.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        edited
    );
    window.simulate_keystrokes("cmd-w");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(app.documents.current().path.is_none());
        assert!(!app.documents.dirty());
    });
}

#[gpui::test]
fn wrapped_source_checkbox_uses_its_visual_row_and_does_not_start_selection(
    cx: &mut TestAppContext,
) {
    let source = format!("{}- [ ] {}", "> ".repeat(12), "task words ".repeat(4));
    let (_dir, mut window, view) = editor(cx, &source, "md", 24.);
    window.simulate_keystrokes("i");
    let (marker, later) = window.update(|_, cx| {
        let app = view.read(cx);
        let row = &app.layouts[Pane::Editor.index()].rows[0];
        let marker = app.projection[0].task_marker.unwrap();
        let position = row.position_for_index(marker + 1);
        assert!(position.y > row.origin.y);
        (
            position + point(px(1.), px(5.)),
            row.position_for_index(source.len() - 3) + point(px(1.), px(5.)),
        )
    });
    window.simulate_mouse_down(marker, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(later, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(later, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.text(), source.replacen("[ ]", "[x]", 1));
        assert_eq!(buffer.mode, Mode::Insert);
        assert_eq!(buffer.col, 0);
        assert!(buffer.selected_range(0).is_none());
    });
    window.simulate_keystrokes("escape u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        source
    );
}

#[gpui::test]
fn native_save_as_reflows_between_plain_and_markdown_and_keeps_clicks_correct(
    cx: &mut TestAppContext,
) {
    let source = "café 世界 words for a longer line ".repeat(12);
    let (dir, mut window, view) = editor(cx, &source, "txt", 15.);
    window.simulate_keystrokes("i cmd-shift-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(dir.path().join("converted.md")));
    window.run_until_parked();
    let click = window.update(|_, cx| {
        let row = &view.read(cx).layouts[Pane::Editor.index()].rows[0];
        assert!(row.wrapped.is_some());
        assert!(row.origin.x > px(300.));
        row.position_for_index(50) + point(px(0.), px(5.))
    });
    window.simulate_click(click, Modifiers::default());
    window.simulate_input("X");
    let expected = format!("{}X{}", &source[..50], &source[50..]);
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        expected
    );
    window.simulate_keystrokes("cmd-shift-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(dir.path().join("back.txt")));
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(app.projection.is_empty());
        assert!(app.layouts[Pane::Editor.index()].rows[0].wrapped.is_none());
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("back.txt")).unwrap(),
        expected
    );
}

#[gpui::test]
fn ime_composition_and_candidate_bounds_follow_wrapping_after_resize(cx: &mut TestAppContext) {
    let source = format!("{}👩‍💻 世界", "some words to wrap ".repeat(8));
    let (_dir, mut window, view) = editor(cx, &source, "md", 15.);
    window.simulate_keystrokes("A");
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "に", None, window, cx);
        })
    });
    window.run_until_parked();
    window.simulate_resize(size(px(740.), px(800.)));
    window.run_until_parked();
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            let start = source.encode_utf16().count();
            assert_eq!(app.marked_text_range(window, cx), Some(start..start + 1));
            let row = &app.layouts[Pane::Editor.index()].rows[0];
            let expected = row.position_for_index(source.len());
            assert!(expected.y > row.origin.y);
            let bounds = app
                .bounds_for_range(
                    start..start + 1,
                    Bounds::new(point(px(0.), px(0.)), size(px(740.), px(800.))),
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(bounds.origin, expected);
            app.replace_text_in_range(None, "日本語", window, cx);
        })
    });
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.documents.current().buffer.text(),
            format!("{source}日本語")
        );
        assert!(app.marked.is_none());
    });
    window.simulate_keystrokes("escape u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        source
    );
}
