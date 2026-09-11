use gpui::{Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, px};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
};

fn editor(
    cx: &mut TestAppContext,
    text: &str,
    plain: bool,
) -> (
    tempfile::TempDir,
    VisualTestContext,
    gpui::Entity<Workspace>,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut document = Document::untitled(text);
    if plain {
        document.path = Some(directory.path().join("plain.txt"));
    }
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            document,
            Explorer::open(directory.path()).unwrap(),
            window,
            cx,
        )
    });
    (directory, window.clone(), view)
}

#[gpui::test]
fn return_continues_markdown_but_shift_return_and_code_stay_literal(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = editor(cx, "- [x] done", false);
    window.simulate_keystrokes("A enter");
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        "- [x] done\n- [ ] "
    );
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        "- [x] done\n"
    );
    window.simulate_keystrokes("escape u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        "- [x] done"
    );
    window.simulate_keystrokes("A shift-enter");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        "- [x] done\n"
    );

    let (_dir, mut code_window, code_view) = editor(cx, "```md\n- literal\n```", false);
    code_window.simulate_keystrokes("j A enter");
    assert_eq!(
        code_window.update(|_, cx| code_view.read(cx).documents.current().buffer.text()),
        "```md\n- literal\n\n```"
    );
    let (_dir, mut plain_window, plain_view) = editor(cx, "- literal", true);
    plain_window.simulate_keystrokes("A enter");
    assert_eq!(
        plain_window.update(|_, cx| plain_view.read(cx).documents.current().buffer.text()),
        "- literal\n"
    );
}

#[gpui::test]
fn clicking_a_preview_task_toggles_only_marker_without_moving_caret(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = editor(cx, "writing\n- [ ] task", false);
    window.simulate_keystrokes("A");
    let point = window.update(|_, cx| {
        let row = view.read(cx).layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 1)
            .unwrap();
        let marker = row.line.text.find("[ ]").unwrap();
        Point::new(
            row.origin.x + row.line.x_for_index(marker + 1),
            row.origin.y + px(10.),
        )
    });
    window.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.text(), "writing\n- [x] task");
        assert_eq!(buffer.row, 0);
        assert_eq!(buffer.col, 7);
        assert_eq!(buffer.mode, rockdown::vim::Mode::Insert);
    });
    window.simulate_keystrokes("escape u");
    assert_eq!(
        window.update(|_, cx| view.read(cx).documents.current().buffer.text()),
        "writing\n- [ ] task"
    );
}
