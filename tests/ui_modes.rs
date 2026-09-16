use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
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
        "writer-tabs-toggle",
        "writer-mode-indicator",
        "writer-chrome-toggle",
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
fn writer_tabs_and_files_preserve_staged_edits_and_restore_dev_panes(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    window.simulate_keystrokes("cmd-e A");
    window.simulate_input(".pending");
    window.simulate_keystrokes("escape");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(app.explorer_visible);
        assert!(app.explorer.dirty());
        assert_eq!(app.explorer.buffer.text(), "note.md.pending");
        assert_eq!(app.documents.current().buffer.text(), SOURCE);
    });

    window.run_until_parked();
    let dev_editor = window.debug_bounds("editor-column").unwrap();
    let dev_mode_control = window.debug_bounds("ui-mode-control").unwrap();
    assert!(dev_editor.top() > px(0.));
    assert!(dev_editor.bottom() < px(480.));
    select_mode(&mut window, "ui-mode-writer");
    assert_writer_chrome(&mut window);
    let writer_editor = window.debug_bounds("editor-column").unwrap();
    assert_eq!(
        window.debug_bounds("ui-mode-control").unwrap(),
        dev_mode_control
    );
    assert_eq!(writer_editor.top(), px(0.));
    assert_eq!(writer_editor.bottom(), px(480.));
    assert!(writer_editor.size.width > dev_editor.size.width);
    window.simulate_keystrokes(":");
    window.simulate_input("w");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), writer_editor);
    window.simulate_keystrokes("escape");
    window.update(|_, cx| assert!(!view.read(cx).explorer_visible));
    window.update(|_, cx| {
        assert_eq!(view.read(cx).explorer.buffer.text(), "note.md.pending");
    });

    click(&mut window, "writer-tabs-toggle");
    let toggle = window.debug_bounds("writer-tabs-toggle").unwrap();
    let vim = window.debug_bounds("writer-mode-indicator").unwrap();
    let popup = window.debug_bounds("writer-tabs-popup").unwrap();
    let tabs = window.debug_bounds("buffer-tabs").unwrap();
    assert_eq!(toggle.top(), vim.top());
    assert!(toggle.left() > vim.right());
    assert_eq!(toggle.size, vim.size);
    assert!(popup.bottom() < toggle.top());
    assert!(popup.left() <= toggle.left() && popup.right() >= toggle.right());
    assert!(popup.left() >= px(0.) && popup.right() <= px(720.));
    assert!(tabs.top() >= popup.top() && tabs.bottom() <= popup.bottom());
    assert_eq!(window.debug_bounds("editor-column").unwrap(), writer_editor);
    click(&mut window, "writer-tabs-toggle");
    assert_writer_chrome(&mut window);

    click(&mut window, "explorer");
    let files = window.debug_bounds("writer-files").unwrap();
    assert!(files.left() >= px(0.) && files.right() <= px(720.));
    assert!(files.top() >= px(0.) && files.bottom() <= px(480.));
    window.update(|_, cx| assert!(view.read(cx).pane == Pane::Explorer));
    window.simulate_keystrokes("A");
    window.simulate_input(".writer");
    window.simulate_keystrokes("escape");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.explorer.buffer.text(), "note.md.pending.writer");
        assert!(app.explorer.dirty());
        assert_eq!(app.documents.current().buffer.text(), SOURCE);
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        SOURCE
    );
    assert!(!dir.path().join("note.md.pending.writer").exists());

    let outside = point(px(4.), px(240.));
    assert!(!files.contains(&outside));
    window.simulate_click(outside, Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| assert!(!view.read(cx).explorer_visible));
    window.update(|window, cx| {
        let app = view.read(cx);
        assert!(app.pane == Pane::Editor);
        assert!(app.focus.is_focused(window));
    });
    window.simulate_keystrokes("i");
    window.simulate_input("typed ");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.documents.current().buffer.text(),
            format!("typed {SOURCE}")
        );
        assert_eq!(app.explorer.buffer.text(), "note.md.pending.writer");
    });

    select_mode(&mut window, "ui-mode-dev");
    assert!(window.debug_bounds("buffer-tabs").is_some());
    assert!(window.debug_bounds("status-bar").is_some());
    assert_eq!(window.debug_bounds("editor-column").unwrap(), dev_editor);
    assert_eq!(
        window.debug_bounds("ui-mode-control").unwrap(),
        dev_mode_control
    );
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(app.explorer_visible);
        assert!(app.explorer.dirty());
        assert_eq!(app.explorer.buffer.text(), "note.md.pending.writer");
        assert_eq!(
            app.documents.current().buffer.text(),
            format!("typed {SOURCE}")
        );
    });
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

#[gpui::test]
fn writer_tabs_are_vertical_modal_and_preserve_dirty_documents(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    let first = window.update(|_, cx| view.read(cx).documents.active_id());
    window.simulate_keystrokes("cmd-n i");
    window.simulate_input("second draft");
    let second = window.update(|_, cx| view.read(cx).documents.active_id());
    // GPUI's debug lookup requires static selectors, including dynamic document IDs.
    static SELECTORS: std::sync::OnceLock<[String; 3]> = std::sync::OnceLock::new();
    let [first_selector, second_selector, close_second] = SELECTORS.get_or_init(|| {
        [
            format!("buffer-tab-{first}"),
            format!("buffer-tab-{second}"),
            format!("close-buffer-{second}"),
        ]
    });
    select_mode(&mut window, "ui-mode-writer");
    click(&mut window, "writer-tabs-toggle");
    let first_tab = window.debug_bounds(first_selector).unwrap();
    let second_tab = window.debug_bounds(second_selector).unwrap();
    assert_eq!(first_tab.left(), second_tab.left());
    assert_eq!(first_tab.right(), second_tab.right());
    assert!(second_tab.top() >= first_tab.bottom());
    window.simulate_input("must not leak");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.active_id(), second);
        assert_eq!(
            app.documents.get(second).unwrap().buffer.text(),
            "second draft"
        );
        assert_eq!(app.documents.get(first).unwrap().buffer.text(), SOURCE);
    });

    click(&mut window, first_selector);
    window.simulate_keystrokes("A");
    window.simulate_input(" selected");
    let selected = SOURCE.replacen("intro", "intro selected", 1);
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.active_id(), first);
        assert_eq!(app.documents.current().buffer.text(), selected);
    });
    click(&mut window, "writer-tabs-toggle");
    click(&mut window, "writer-tabs-toggle");
    window.simulate_input(" toggled");
    let toggled = SOURCE.replacen("intro", "intro selected toggled", 1);
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), toggled));

    click(&mut window, "writer-tabs-toggle");
    window.simulate_keystrokes("escape");
    window.simulate_input(" escaped");
    let escaped = SOURCE.replacen("intro", "intro selected toggled escaped", 1);
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), escaped));

    window.simulate_keystrokes("escape");
    click(&mut window, "writer-tabs-toggle");
    let popup = window.debug_bounds("writer-tabs-popup").unwrap();
    let outside = point(px(4.), px(240.));
    assert!(!popup.contains(&outside));
    window.simulate_click(outside, Modifiers::default());
    window.run_until_parked();
    // No Escape here: it would conceal a broken outside-click dismissal.
    window.simulate_keystrokes("g g 0 i");
    window.simulate_input("outside ");
    let edited = format!("outside {escaped}");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), edited));

    click(&mut window, "writer-tabs-toggle");
    click(&mut window, close_second);
    cx.simulate_prompt_answer("Cancel");
    window.run_until_parked();
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.entries().len(), 2);
        assert_eq!(app.documents.active_id(), first);
        assert_eq!(app.documents.get(first).unwrap().buffer.text(), edited);
        assert_eq!(
            app.documents.get(second).unwrap().buffer.text(),
            "second draft"
        );
        assert!(app.documents.get(second).unwrap().buffer.dirty());
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        SOURCE
    );
}

#[gpui::test]
fn writer_hidden_chrome_restores_without_reflow_or_losing_document_and_staged_edits(
    cx: &mut TestAppContext,
) {
    let (dir, mut window, view) = workspace(cx);
    window.simulate_keystrokes("cmd-e A");
    window.simulate_input(".pending");
    window.simulate_keystrokes("escape");
    select_mode(&mut window, "ui-mode-writer");
    let editor = window.debug_bounds("editor-column").unwrap();
    let old_help = window.debug_bounds("help").unwrap().center();
    click(&mut window, "writer-chrome-toggle");
    assert!(window.debug_bounds("writer-restore-control").is_some());
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    window.simulate_click(old_help, Modifiers::default());
    window.run_until_parked();
    // The former Help button must no longer intercept editor interaction.
    window.simulate_keystrokes("g g 0 i");
    window.simulate_input("hidden ");
    let hidden = format!("hidden {SOURCE}");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.help);
        assert_eq!(app.documents.current().buffer.text(), hidden);
        assert_eq!(app.explorer.buffer.text(), "note.md.pending");
    });
    click(&mut window, "writer-restore-control");
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    click(&mut window, "help");
    window.update(|_, cx| assert!(view.read(cx).help));
    window.simulate_input("blocked by restored help");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), hidden));
    click(&mut window, "close-help");

    // These two shortcut spellings must toggle the same Writer-only state.
    window.simulate_keystrokes("escape cmd-alt-s");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    window.simulate_click(old_help, Modifiers::default());
    window.simulate_keystrokes("g g 0 i");
    window.simulate_input("shortcut ");
    let shortcut = format!("shortcut {hidden}");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.help);
        assert_eq!(app.documents.current().buffer.text(), shortcut);
    });
    window.simulate_keystrokes("ctrl-alt-s");
    window.run_until_parked();
    click(&mut window, "help");
    window.update(|_, cx| assert!(view.read(cx).help));
    click(&mut window, "close-help");

    window.simulate_keystrokes("escape :");
    window.simulate_input("statusbar");
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    window.simulate_click(old_help, Modifiers::default());
    window.simulate_keystrokes("g g 0 i");
    window.simulate_input("command ");
    let edited = format!("command {shortcut}");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.help);
        assert_eq!(app.documents.current().buffer.text(), edited);
    });
    // Command entry and error feedback remain usable while controls are hidden.
    window.simulate_keystrokes("escape :");
    window.simulate_input("not-a-command");
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    window.update(|_, cx| assert!(view.read(cx).message.contains("Unknown command")));
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    click(&mut window, "dismiss-feedback");
    window.simulate_keystrokes(":");
    window.simulate_input("statusbar");
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), editor);
    // The recovered mode control must actually work, not just retain bounds.
    select_mode(&mut window, "ui-mode-dev");
    let dev_editor = window.debug_bounds("editor-column").unwrap();
    assert!(dev_editor.top() > editor.top());
    assert!(dev_editor.bottom() < editor.bottom());
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), edited);
        assert!(app.explorer.dirty());
        assert_eq!(app.explorer.buffer.text(), "note.md.pending");
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.md")).unwrap(),
        SOURCE
    );
    assert!(!dir.path().join("note.md.pending").exists());
}
