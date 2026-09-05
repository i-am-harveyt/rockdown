use gpui::{Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, px, size};
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
