use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    vim::Mode,
};

fn editor(
    cx: &mut TestAppContext,
    text: &str,
) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let directory = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled(text),
            Explorer::open(directory.path()).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    window.simulate_resize(size(px(700.), px(900.)));
    window.run_until_parked();
    (directory, window, view)
}

#[gpui::test]
fn active_paragraph_wraps_and_clicks_use_visual_rows_without_leaving_insert(
    cx: &mut TestAppContext,
) {
    let text = "café hello 世界 more words to fill a paragraph comfortably ".repeat(12);
    let (_directory, mut window, view) = editor(cx, &text);
    window.simulate_keystrokes("i");
    window.run_until_parked();
    let (col, position) = window.update(|_, cx| {
        let app = view.read(cx);
        let row = &app.layouts[Pane::Editor.index()].rows[0];
        assert!(row.wrapped.is_some());
        assert!(row.height > row.line_height);
        let col = text.find("comfortably").unwrap();
        let position = row.position_for_index(col);
        assert!(position.y > row.origin.y);
        assert_eq!(row.index_for_position(position), col);
        (col, position + point(px(0.), px(5.)))
    });
    window.simulate_click(position, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.mode, Mode::Insert);
        assert_eq!(buffer.col, col);
    });
    window.simulate_keystrokes("down");
    window.run_until_parked();
    let down = window.update(|_, cx| view.read(cx).documents.current().buffer.col);
    assert!(down > col);
    window.simulate_keystrokes("up");
    window.run_until_parked();
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.col, col));
    window.simulate_input("X");
    window.update(|_, cx| {
        assert_eq!(
            view.read(cx).documents.current().buffer.lines[0].text,
            format!("{}X{}", &text[..col], &text[col..])
        )
    });
}

#[gpui::test]
fn preview_click_skips_hidden_markup_and_drag_selects_across_wraps(cx: &mut TestAppContext) {
    let paragraph = "hello world these are more words in a paragraph ".repeat(8);
    let text = format!("outside\n**café** and [世界](url)\n{paragraph}");
    let (_directory, mut window, view) = editor(cx, &text);
    let position = window.update(|_, cx| {
        let row = &view.read(cx).layouts[Pane::Editor.index()].rows[1];
        assert!(!row.raw);
        row.position_for_index("café and ".len()) + point(px(0.), px(5.))
    });
    window.simulate_click(position, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col), (1, "**café** and [".len()));
    });
    window.simulate_keystrokes("j");
    window.run_until_parked();
    let (start, end) = window.update(|_, cx| {
        let row = view.read(cx).layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 2)
            .unwrap();
        (
            row.position_for_index(0) + point(px(0.), px(5.)),
            row.position_for_index(60) + point(px(0.), px(5.)),
        )
    });
    window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(buffer.selected_range(2), Some(0..61));
    });
}

#[gpui::test]
fn caret_remains_visible_inside_a_paragraph_taller_than_the_viewport(cx: &mut TestAppContext) {
    let text = "many words filling one very long paragraph ".repeat(100);
    let (_directory, mut window, view) = editor(cx, &text);
    window.simulate_keystrokes("A");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let row = &app.layouts[Pane::Editor.index()].rows[0];
        let caret = row.position_for_index(app.documents.current().buffer.col);
        assert!(app.scroll_offsets[Pane::Editor.index()] > 0.);
        assert!(caret.y > px(0.) && caret.y < px(900.));
    });
    window.simulate_keystrokes("escape 0");
    window.run_until_parked();
    window.update(|_, cx| assert!(view.read(cx).scroll_offsets[Pane::Editor.index()] < 5.));
}

#[gpui::test]
fn insert_arrows_cross_paragraph_boundaries_using_editable_visual_rows(cx: &mut TestAppContext) {
    let paragraph = "words filling a long paragraph across several visual rows ".repeat(5);
    let text = format!("{paragraph}\nmiddle row\n# {paragraph}");
    let (_directory, mut window, view) = editor(cx, &text);
    window.simulate_keystrokes("j l l l i");
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col), (1, 3));
    });
    window.simulate_keystrokes("up");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let buffer = &app.documents.current().buffer;
        assert_eq!(buffer.row, 0);
        let row = app.layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 0)
            .unwrap();
        assert!(row.wrapped.is_some());
        let caret = row.position_for_index(buffer.col);
        assert_eq!(caret.y, row.origin.y + row.height - row.line_height);
        assert!(
            buffer.col > 3,
            "Up must enter the last visual row, not the first"
        );
    });
    window.simulate_keystrokes("down");
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(
            (buffer.row, buffer.col),
            (1, 3),
            "Crossing back preserves preferred screen x"
        );
    });
    window.simulate_keystrokes("down");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        let buffer = &app.documents.current().buffer;
        assert_eq!(
            (buffer.row, buffer.col),
            (2, 3),
            "Destination is shaped as source, including heading syntax"
        );
        let row = app.layouts[Pane::Editor.index()]
            .rows
            .iter()
            .find(|row| row.source_row == 2)
            .unwrap();
        assert!(row.raw && row.wrapped.is_some());
        assert_eq!(row.position_for_index(buffer.col).y, row.origin.y);
    });
    window.simulate_keystrokes("up");
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!((buffer.row, buffer.col), (1, 3));
    });
    window.simulate_keystrokes("escape k");
    window.run_until_parked();
    window.update(|_, cx| {
        let buffer = &view.read(cx).documents.current().buffer;
        assert_eq!(
            (buffer.mode, buffer.row),
            (Mode::Normal, 0),
            "Vim k still moves by source line"
        );
    });
}
