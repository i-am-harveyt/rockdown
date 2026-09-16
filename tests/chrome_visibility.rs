use gpui::{Modifiers, TestAppContext, px, size};
use rockdown::{app::Workspace, config::Config, document::Document, explorer::Explorer};
use std::time::Duration;

#[gpui::test]
fn hidden_chrome_preserves_commands_buffers_and_restore_controls(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "tab_bar_visible = false\nstatus_bar_visible = false\n",
    )
    .unwrap();
    let (config, path) = Config::load(Some(&config_path)).unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            config,
            path,
            Document::untitled("original"),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.simulate_resize(size(px(720.), px(480.)));
    window.run_until_parked();
    assert!(window.debug_bounds("buffer-tabs").is_none());
    assert!(window.debug_bounds("status-bar").is_none());
    assert!(window.debug_bounds("command-feedback").is_none());
    let full = window.debug_bounds("editor-column").unwrap();

    window.simulate_keystrokes("cmd-alt-t");
    window.run_until_parked();
    assert!(window.debug_bounds("buffer-tabs").is_some());
    assert!(window.debug_bounds("status-bar").is_none());
    let tabs_only = window.debug_bounds("editor-column").unwrap();
    assert!(tabs_only.top() > full.top());
    assert_eq!(tabs_only.bottom(), full.bottom());
    window.simulate_keystrokes("cmd-alt-t cmd-alt-s");
    window.run_until_parked();
    let status_only = window.debug_bounds("editor-column").unwrap();
    assert_eq!(status_only.top(), full.top());
    assert!(status_only.bottom() < full.bottom());
    window.simulate_keystrokes("cmd-alt-s");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);
    window.simulate_keystrokes("cmd-n ctrl-pageup");
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), "original"));

    window.simulate_keystrokes(":");
    window.simulate_input("not-a-command");
    window.run_until_parked();
    assert!(window.debug_bounds("command-feedback").is_some());
    assert!(window.debug_bounds("editor-column").unwrap().bottom() < full.bottom());
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    window.update(|_, cx| assert!(view.read(cx).message.contains("Unknown command")));
    let dismiss = window.debug_bounds("dismiss-feedback").unwrap();
    window.simulate_click(dismiss.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);

    window.simulate_keystrokes("f1 left");
    window.run_until_parked();
    for action in ["tab-bar", "status-bar"] {
        let button = window.debug_bounds(action).unwrap();
        window.simulate_click(button.center(), Modifiers::default());
        window.run_until_parked();
    }
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    let both = window.debug_bounds("editor-column").unwrap();
    assert_eq!(both.top(), tabs_only.top());
    assert_eq!(both.bottom(), status_only.bottom());

    window.simulate_keystrokes(":");
    window.simulate_input("config");
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    let dismiss = window.debug_bounds("dismiss-feedback").unwrap();
    window.simulate_click(dismiss.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);
    for command in ["tabbar", "statusbar"] {
        window.simulate_keystrokes(":");
        window.simulate_input(command);
        window.simulate_keystrokes("enter");
    }
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), both);
    window.update(|_, cx| assert_eq!(view.read(cx).documents.current().buffer.text(), "original"));
}

#[gpui::test]
fn notifications_expire_without_dismissing_newer_messages_or_command_input(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config {
                status_bar_visible: false,
                ..Config::default()
            },
            None,
            Document::untitled("original"),
            Explorer::open(dir.path()).unwrap(),
            window,
            cx,
        )
    });
    let mut window = window.clone();
    window.run_until_parked();
    let full = window.debug_bounds("editor-column").unwrap();

    window.simulate_keystrokes(":");
    window.simulate_input("invalid");
    window.simulate_keystrokes("enter");
    cx.background_executor.advance_clock(Duration::from_secs(4));
    window.run_until_parked();
    assert!(window.debug_bounds("editor-column").unwrap().bottom() < full.bottom());

    // Even an identical notification gets a fresh five seconds.
    window.simulate_keystrokes(":");
    window.simulate_input("invalid");
    window.simulate_keystrokes("enter");
    cx.background_executor.advance_clock(Duration::from_secs(1));
    window.run_until_parked();
    window.update(|_, cx| assert!(!view.read(cx).message.is_empty()));
    assert!(window.debug_bounds("editor-column").unwrap().bottom() < full.bottom());

    // Expiring a notification must not discard an in-progress command.
    window.simulate_keystrokes(":");
    window.simulate_input("status");
    cx.background_executor.advance_clock(Duration::from_secs(4));
    window.run_until_parked();
    window.update(|_, cx| {
        assert!(view.read(cx).message.is_empty());
        assert_eq!(view.read(cx).command.as_deref(), Some(":status"));
    });
    assert!(window.debug_bounds("editor-column").unwrap().bottom() < full.bottom());
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);

    // Manual dismissal cancels its timer, not the next notification's timer.
    window.simulate_keystrokes(":");
    window.simulate_input("invalid");
    window.simulate_keystrokes("enter");
    cx.background_executor.advance_clock(Duration::from_secs(1));
    window.run_until_parked();
    let dismiss = window.debug_bounds("dismiss-feedback").unwrap();
    window.simulate_click(dismiss.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);
    window.simulate_keystrokes(":");
    window.simulate_input("another-invalid");
    window.simulate_keystrokes("enter");
    cx.background_executor.advance_clock(Duration::from_secs(4));
    window.run_until_parked();
    assert!(window.debug_bounds("editor-column").unwrap().bottom() < full.bottom());
    cx.background_executor.advance_clock(Duration::from_secs(1));
    window.run_until_parked();
    assert_eq!(window.debug_bounds("editor-column").unwrap(), full);
    window.update(|_, cx| {
        assert!(view.read(cx).message.is_empty());
        assert_eq!(view.read(cx).documents.current().buffer.text(), "original");
    });
}
