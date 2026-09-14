use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use rockdown::{
    app::Workspace,
    config::{Config, ThemePreset},
    document::Document,
    explorer::Explorer,
};

fn workspace(cx: &mut TestAppContext) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.theme.accent = "#123456".into();
    config.markdown.colors.link = Some("#abcdef".into());
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            config,
            None,
            Document::untitled("# Heading\n\n```rust\nlet value = 42;\n```"),
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

#[gpui::test]
fn preview_cancel_and_confirm_preserve_document_and_custom_styles(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    let original = window.update(|_, cx| view.read(cx).config.theme.clone());
    let source = window.update(|_, cx| view.read(cx).documents.current().buffer.text());
    let code_colors = |app: &Workspace| {
        app.projection[3]
            .spans
            .iter()
            .map(|span| span.color.clone())
            .collect::<Vec<_>>()
    };
    let original_colors = window.update(|_, cx| code_colors(view.read(cx)));
    window.simulate_keystrokes("i ctrl-shift-t end");
    window.run_until_parked();
    window.update(|_, cx| {
        assert_eq!(
            view.read(cx).config.theme.preset,
            ThemePreset::SolarizedLight
        )
    });
    window.update(|_, cx| assert_ne!(code_colors(view.read(cx)), original_colors));
    window.simulate_input("must not enter the document");
    window.simulate_keystrokes("cmd-n");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.documents.current().buffer.text(), source);
        assert_eq!(app.documents.entries().len(), 1);
    });
    window.simulate_keystrokes("escape");
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme, original));
    window.update(|_, cx| assert_eq!(code_colors(view.read(cx)), original_colors));
    window.simulate_keystrokes("ctrl-shift-t end up enter");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.config.theme.preset, ThemePreset::Paper);
        assert_eq!(app.config.markdown.colors.link.as_deref(), Some("#abcdef"));
        assert_eq!(
            app.documents.current().buffer.mode,
            rockdown::vim::Mode::Insert
        );
    });
    window.simulate_input("typed");
    window.update(|_, cx| {
        assert!(
            view.read(cx)
                .documents
                .current()
                .buffer
                .text()
                .starts_with("typed")
        )
    });
}

#[gpui::test]
fn mouse_picker_fits_minimum_window_and_outside_click_cancels(cx: &mut TestAppContext) {
    let (_dir, mut window, view) = workspace(cx);
    let button = window.debug_bounds("themes").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    window.run_until_parked();
    let picker = window.debug_bounds("theme-selector").unwrap();
    let last = window.debug_bounds("theme-option-6").unwrap();
    assert!(picker.top() >= px(0.) && picker.bottom() <= px(480.));
    assert!(last.bottom() <= picker.bottom());
    let paper = window.debug_bounds("theme-option-5").unwrap();
    window.simulate_click(paper.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, ThemePreset::Paper));
    window.simulate_keystrokes("ctrl-shift-t down down");
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, ThemePreset::Nord));
    window.simulate_click(point(px(10.), px(100.)), Modifiers::default());
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, ThemePreset::Paper));
}
