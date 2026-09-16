use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, px, size};
use rockdown::{
    app::{Pane, Workspace},
    config::Config,
    document::Document,
    explorer::Explorer,
    vim::Mode,
};

const SOURCE: &str = "intro\n# First\nparagraph\n## Second\nmore prose";

fn workspace(cx: &mut TestAppContext) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(&path, SOURCE).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config {
                tab_bar_visible: true,
                status_bar_visible: true,
                ..Config::default()
            },
            Some(dir.path().join("config.toml")),
            Document::open(&path).unwrap(),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let window = window.clone();
    window.simulate_resize(size(px(720.), px(480.)));
    window.run_until_parked();
    (dir, window, view)
}

fn click(window: &mut VisualTestContext, selector: &'static str) {
    window.run_until_parked();
    let bounds = window.debug_bounds(selector).unwrap();
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
}

fn select_mode(window: &mut VisualTestContext, selector: &'static str) {
    click(window, "ui-mode-control");
    assert!(window.debug_bounds("ui-mode-selector").is_some());
    click(window, selector);
}

fn assert_writer_chrome(window: &mut VisualTestContext) {
    window.run_until_parked();
    // GPUI 0.2.2 retains old debug bounds after removal; use these only
    // for current geometry, and verify dismissal through focus and input.
    assert!(window.debug_bounds("themes").is_none());
    for selector in [
        "ui-mode-control",
        "writer-mode-indicator",
        "writer-tools",
        "outline",
        "explorer",
        "help",
    ] {
        let bounds = window.debug_bounds(selector).unwrap();
        assert!(bounds.left() >= px(0.) && bounds.right() <= px(720.));
        assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(480.));
    }
}

#[gpui::test]
fn mode_picker_blocks_insert_and_document_actions_and_writer_keeps_theme_shortcut(
    cx: &mut TestAppContext,
) {
    let (dir, mut window, view) = workspace(cx);
    // Explicitly hide a previously opened Dev explorer before the round trip.
    window.simulate_keystrokes("cmd-e cmd-e A");
    window.simulate_input(" unsaved");
    let edited = SOURCE.replacen("intro", "intro unsaved", 1);
    click(&mut window, "ui-mode-control");
    assert!(window.debug_bounds("ui-mode-selector").is_some());
    window.simulate_input("must not enter the document");
    window.simulate_keystrokes("cmd-s ctrl-s cmd-n ctrl-n");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), edited);
        assert_eq!(app.documents.entries().len(), 1);
        assert!(app.documents.dirty());
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        SOURCE
    );
    assert!(window.debug_bounds("ui-mode-selector").is_some());
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    window.update(|window, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.mode, Mode::Insert);
        assert!(app.focus.is_focused(window));
    });

    select_mode(&mut window, "ui-mode-writer");
    assert_writer_chrome(&mut window);
    let theme = window.update(|_, cx| view.read(cx).config.theme.clone());
    window.simulate_keystrokes("cmd-shift-t end");
    window.run_until_parked();
    assert!(window.debug_bounds("theme-selector").is_some());
    assert!(window.debug_bounds("themes").is_none());
    window.update(|_, cx| assert_ne!(view.read(cx).config.theme, theme));
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme, theme));
    window.simulate_input(" resumed");
    let resumed = SOURCE.replacen("intro", "intro unsaved resumed", 1);
    window.update(|_, cx| {
        assert_eq!(view.read(cx).documents.current().buffer.text(), resumed);
    });
    assert_writer_chrome(&mut window);

    // Both shortcut spellings must reach the same modal selector in Writer.
    window.simulate_keystrokes("ctrl-shift-m");
    window.run_until_parked();
    assert!(window.debug_bounds("ui-mode-selector").is_some());
    window.simulate_keystrokes("escape cmd-shift-m");
    window.run_until_parked();
    assert!(window.debug_bounds("ui-mode-selector").is_some());
    click(&mut window, "ui-mode-dev");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.explorer_visible);
        assert_eq!(app.documents.current().buffer.text(), resumed);
    });
}

#[gpui::test]
fn writer_tools_switch_and_close_without_leaking_input_or_restoring_fixed_chrome(
    cx: &mut TestAppContext,
) {
    let (_dir, mut window, view) = workspace(cx);
    window.simulate_keystrokes("i");
    select_mode(&mut window, "ui-mode-writer");
    click(&mut window, "outline");
    assert!(window.debug_bounds("outline-panel").is_some());
    window.simulate_input("First");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), SOURCE));
    assert_writer_chrome(&mut window);

    // Tool buttons remain clickable above each popup's dismissal layer.
    click(&mut window, "help");
    window.update(|_, cx| assert!(view.read(cx).help));
    assert!(window.debug_bounds("help-sheet").is_some());
    assert_writer_chrome(&mut window);
    click(&mut window, "explorer");
    window.update(|_, cx| assert!(!view.read(cx).help));
    assert!(window.debug_bounds("writer-files").is_some());
    assert_writer_chrome(&mut window);
    click(&mut window, "outline");
    window.update(|_, cx| assert!(!view.read(cx).explorer_visible));
    assert!(window.debug_bounds("outline-panel").is_some());
    click(&mut window, "outline");
    window.simulate_input("after outline ");
    let edited = format!("after outline {SOURCE}");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), edited));
    assert_writer_chrome(&mut window);

    click(&mut window, "help");
    assert!(window.debug_bounds("help-sheet").is_some());
    click(&mut window, "close-help");
    window.update(|window, cx| {
        let app = view.read(cx);
        assert!(app.pane == Pane::Editor);
        assert!(app.focus.is_focused(window));
        assert_eq!(app.documents.current().buffer.text(), edited);
    });
    window.simulate_input("continued ");
    window.update(|_, cx| {
        assert_eq!(
            view.read(cx).documents.current().buffer.text(),
            format!("after outline continued {SOURCE}")
        );
    });
    assert_writer_chrome(&mut window);
}
