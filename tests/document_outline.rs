use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, TestAppContext, VisualTestContext, point, px,
    size,
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
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("note.{extension}"));
    std::fs::write(&path, source).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::open(&path).unwrap(),
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

#[gpui::test]
fn unicode_ime_filter_and_jump_preserve_insert_mode_content_and_undo(cx: &mut TestAppContext) {
    let source = "intro\n# Café\nprose\n## 中文章节\nbody\n## Other";
    let (_dir, mut window, view) = editor(cx, source, "md");
    window.simulate_keystrokes("A");
    window.simulate_input("!");
    let edited = source.replacen("intro", "intro!", 1);
    window.simulate_keystrokes("cmd-shift-o");
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            assert!(app.selected_text_range(false, window, cx).is_some());
            app.replace_and_mark_text_in_range(None, "zhong", None, window, cx);
            assert_eq!(app.marked_text_range(window, cx), Some(0..5));
            app.replace_text_in_range(None, "中文", window, cx);
            assert_eq!(app.documents.current().buffer.text(), edited);
        })
    });
    window.run_until_parked();
    let panel = window.debug_bounds("outline-panel").unwrap();
    window.update(|window, cx| {
        view.update(cx, |app, cx| {
            let caret = app
                .bounds_for_range(
                    2..2,
                    Bounds::new(point(px(0.), px(0.)), size(px(900.), px(600.))),
                    window,
                    cx,
                )
                .unwrap();
            assert!(panel.contains(&caret.origin));
        })
    });
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let buffer = &app.documents.current().buffer;
        assert_eq!((buffer.row, buffer.col, buffer.mode), (3, 0, Mode::Insert));
        assert_eq!(buffer.text(), edited);
        assert!(
            app.layouts[Pane::Editor.index()]
                .rows
                .iter()
                .any(|row| row.source_row == 3)
        );
    });
    window.simulate_keystrokes("escape u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        source
    );
}

#[gpui::test]
fn keyboard_results_scroll_and_filtered_click_jumps_to_physical_row(cx: &mut TestAppContext) {
    let source = (0..45)
        .map(|index| format!("## 章节 {index}\nbody\n"))
        .collect::<String>();
    let (_dir, mut window, view) = editor(cx, &source, "md");
    let button = window.debug_bounds("outline").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    window.simulate_keystrokes(
        &std::iter::repeat_n("down", 40)
            .collect::<Vec<_>>()
            .join(" "),
    );
    window.run_until_parked();
    let panel = window.debug_bounds("outline-panel").unwrap();
    let selected = window.debug_bounds("outline-heading-40").unwrap();
    assert!(selected.top() >= panel.top() && selected.bottom() <= panel.bottom());
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.row, 80);
        assert!(
            app.layouts[Pane::Editor.index()]
                .rows
                .iter()
                .any(|row| row.source_row == 80)
        );
    });
    window.simulate_keystrokes(":");
    window.simulate_input("outline");
    window.simulate_keystrokes("enter");
    window.simulate_input("章节 3");
    window.run_until_parked();
    let target = window.debug_bounds("outline-heading-3").unwrap();
    window.simulate_click(target.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.mode), (6, Mode::Normal));
        assert_eq!(buffer.text(), source);
    });
}

#[gpui::test]
fn plain_text_save_as_edits_and_document_switch_refresh_outline(cx: &mut TestAppContext) {
    let source = "intro\n# First\n## 中文";
    let (dir, mut window, view) = editor(cx, source, "txt");
    window.simulate_keystrokes("ctrl-shift-o");
    window.run_until_parked();
    assert!(window.debug_bounds("outline-empty").is_some());
    window.simulate_input("中文");
    window.simulate_keystrokes("enter escape cmd-shift-s");
    window.run_until_parked();
    cx.simulate_new_path_selection(|_| Some(dir.path().join("converted.md")));
    window.run_until_parked();
    window.simulate_keystrokes("ctrl-shift-o");
    window.simulate_input("中文");
    window.simulate_keystrokes("enter A");
    window.simulate_input("更新");
    window.simulate_keystrokes("escape ctrl-shift-o");
    window.simulate_input("更新");
    window.simulate_keystrokes("enter");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.row),
        2
    );
    window.simulate_keystrokes("cmd-n i");
    window.simulate_input("intro");
    window.simulate_keystrokes("enter");
    window.simulate_input("# Different");
    window.simulate_keystrokes("enter");
    window.simulate_input("## 中文第二");
    window.simulate_keystrokes("escape ctrl-shift-o");
    window.simulate_input("中文");
    window.simulate_keystrokes("ctrl-pageup enter");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.lines[2].text, "## 中文更新");
    });
    window.simulate_keystrokes("ctrl-pagedown ctrl-shift-o");
    window.simulate_input("第二");
    window.simulate_keystrokes("enter");
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.row, 2);
        assert_eq!(buffer.lines[2].text, "## 中文第二");
    });
}

#[gpui::test]
fn modifier_click_resolves_local_unicode_fragment_without_task_or_external_side_effects(
    cx: &mut TestAppContext,
) {
    let source =
        "intro\n- [ ] [jump](#%E4%B8%AD%E6%96%87) [outside](https://example.com/#x)\nbody\n# 中文";
    let (_dir, mut window, view) = editor(cx, source, "md");
    window.simulate_keystrokes("i");
    let link = window.update(|_, cx| {
        let row = &view.read(cx).layouts[Pane::Editor.index()].rows[1];
        row.position_for_index(row.line.text.find("jump").unwrap()) + point(px(2.), px(5.))
    });
    window.simulate_click(
        link,
        Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col, buffer.mode), (3, 0, Mode::Insert));
        assert_eq!(buffer.text(), source);
        assert!(buffer.selected_range(1).is_none());
    });
    // Restore the first viewport so the external preview link is visible again.
    window.simulate_keystrokes("escape g g i");
    window.run_until_parked();
    let external = window.update(|_, cx| {
        let row = &view.read(cx).layouts[Pane::Editor.index()].rows[1];
        row.position_for_index(row.line.text.find("outside").unwrap()) + point(px(2.), px(5.))
    });
    window.simulate_click(
        external,
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col, buffer.mode), (0, 0, Mode::Insert));
        assert_eq!(buffer.text(), source);
        assert!(buffer.selected_range(1).is_none());
    });
}
