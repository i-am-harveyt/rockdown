use gpui::{Modifiers, TestAppContext, point, px, size};
use rockdown::{
    app::Workspace,
    config::{Config, ThemePreset},
    document::Document,
    explorer::Explorer,
};

#[gpui::test]
fn help_topics_fit_small_windows_and_dismiss_without_editing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled("original"),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.simulate_resize(size(px(720.), px(480.)));
    window.simulate_keystrokes("i f1");
    window.simulate_input("ignored");
    window.simulate_keystrokes("cmd-n");
    window.update(|_, cx| {
        assert_eq!(view.read(cx).documents.entries().len(), 1);
        assert_eq!(view.read(cx).documents.current().buffer.text(), "original");
    });
    for preset in [ThemePreset::Rockdown, ThemePreset::Paper] {
        window.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.config.theme = preset.theme();
                cx.notify();
            })
        });
        for _ in 0..6 {
            window.run_until_parked();
            let sheet = window.debug_bounds("help-sheet").unwrap();
            let content = window.debug_bounds("help-content").unwrap();
            let close = window.debug_bounds("close-help").unwrap();
            let topic = window.debug_bounds("help-topic-5").unwrap();
            assert!(sheet.top() >= px(0.) && sheet.bottom() <= px(480.));
            assert!(content.size.height > px(100.));
            assert!(close.bottom() < content.top());
            assert!(topic.bottom() <= content.top());
            window.simulate_keystrokes("right");
        }
    }
    let topic = window.debug_bounds("help-topic-3").unwrap();
    window.simulate_click(topic.center(), Modifiers::default());
    window.update(|_, cx| assert!(view.read(cx).help));
    let close = window.debug_bounds("close-help").unwrap();
    window.simulate_click(close.center(), Modifiers::default());
    window.update(|_, cx| assert!(!view.read(cx).help));
    window.simulate_keystrokes("f1");
    window.simulate_click(point(px(4.), px(200.)), Modifiers::default());
    window.update(|_, cx| assert!(!view.read(cx).help));
    window.simulate_keystrokes("f1 escape");
    window.simulate_input("typed");
    window.update(|_, cx| {
        assert_eq!(
            view.read(cx).documents.current().buffer.text(),
            "typedoriginal"
        )
    });
}
