use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use rockdown::{app::Workspace, config::Config, document::Document, explorer::Explorer};

fn workspace(cx: &mut TestAppContext) -> (tempfile::TempDir, VisualTestContext, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("colorscheme")).unwrap();
    std::fs::write(
        dir.path().join("colorscheme/nord.toml"),
        include_str!("../examples/nord.toml"),
    )
    .unwrap();
    let config_path = dir.path().join("config.toml");
    let mut config = Config::default();
    config.theme.accent = "#123456".into();
    config.markdown.colors.link = Some("#abcdef".into());
    config.markdown.colors.bold = Some("#f2cf8f".into());
    config.markdown.h1.color = Some("#c2dded".into());
    config.markdown.h1.font_size = Some(32.);
    config.markdown.h1.underline = true;
    config.markdown.divider.color = Some("#566575".into());
    config.markdown.divider.thickness = 1.5;
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            config,
            Some(config_path),
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
fn preview_cancel_and_confirm_preserve_document_and_typography(cx: &mut TestAppContext) {
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
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, "nord"));
    window.update(|_, cx| assert_ne!(code_colors(view.read(cx)), original_colors));
    window.simulate_keystrokes("home");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.config.theme, original);
        assert_eq!(app.config.markdown.colors.bold.as_deref(), Some("#f2cf8f"));
        assert_eq!(app.config.markdown.h1.color.as_deref(), Some("#c2dded"));
        assert_eq!(code_colors(app), original_colors);
    });
    window.simulate_keystrokes("down down down");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.config.theme.preset, "nord");
        assert_eq!(app.config.markdown.colors.bold.as_deref(), Some("#ebcb8b"));
        assert_ne!(code_colors(app), original_colors);
    });
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
    window.update(|_, cx| {
        let markdown = &view.read(cx).config.markdown;
        assert_eq!(markdown.colors.bold.as_deref(), Some("#f2cf8f"));
        assert_eq!(markdown.colors.link.as_deref(), Some("#abcdef"));
        assert_eq!(markdown.h1.color.as_deref(), Some("#c2dded"));
        assert_eq!(markdown.divider.color.as_deref(), Some("#566575"));
    });
    window.simulate_keystrokes("ctrl-shift-t end up enter");
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.config.theme.preset, "paper");
        assert_eq!(app.config.markdown.colors.link, None);
        assert_eq!(app.config.markdown.colors.bold, None);
        assert_eq!(app.config.markdown.h1.color, None);
        assert_eq!(app.config.markdown.divider.color, None);
        assert_eq!(app.config.markdown.h1.font_size, Some(32.));
        assert!(app.config.markdown.h1.underline);
        assert_eq!(app.config.markdown.divider.thickness, 1.5);
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
    let last = window.debug_bounds("theme-option-3").unwrap();
    assert!(picker.top() >= px(0.) && picker.bottom() <= px(480.));
    assert!(last.bottom() <= picker.bottom());
    let paper = window.debug_bounds("theme-option-2").unwrap();
    window.simulate_click(paper.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, "paper"));
    window.simulate_keystrokes("ctrl-shift-t end");
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, "nord"));
    window.simulate_click(point(px(10.), px(100.)), Modifiers::default());
    window.update(|_, cx| assert_eq!(view.read(cx).config.theme.preset, "paper"));
}

#[gpui::test]
fn picker_rediscovers_files_and_scrolls_to_keyboard_selection(cx: &mut TestAppContext) {
    let (dir, mut window, view) = workspace(cx);
    window.simulate_keystrokes("ctrl-shift-t end enter");
    window.update(|_, cx| {
        assert_eq!(view.read(cx).config.theme.preset, "nord");
        assert_eq!(
            view.read(cx).config.markdown.colors.bold.as_deref(),
            Some("#ebcb8b")
        );
    });
    for index in 0..20 {
        std::fs::write(
            dir.path()
                .join(format!("colorscheme/custom-{index:02}.toml")),
            "base = 'paper'\n[theme]\naccent = '#234567'",
        )
        .unwrap();
    }
    std::fs::write(
        dir.path().join("colorscheme/nord.toml"),
        "name = 'Updated Nord'\n[theme]\naccent = '#654321'",
    )
    .unwrap();
    window.simulate_keystrokes("ctrl-shift-t end");
    window.run_until_parked();
    let picker = window.debug_bounds("theme-selector").unwrap();
    let last = window.debug_bounds("theme-option-23").unwrap();
    assert!(last.top() >= picker.top() && last.bottom() <= picker.bottom());
    window.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.config.theme.name, "Updated Nord");
        assert_eq!(app.config.theme.accent, "#654321");
    });
    window.simulate_keystrokes("escape");
    window.update(|_, cx| {
        assert_eq!(view.read(cx).config.theme.accent, "#88c0d0");
        assert_eq!(
            view.read(cx).config.markdown.colors.bold.as_deref(),
            Some("#ebcb8b")
        );
    });
}
