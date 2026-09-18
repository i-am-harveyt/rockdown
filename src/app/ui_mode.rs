use super::{Pane, UiMode, Workspace};
use gpui::{prelude::*, *};

const POPUP_GAP: f32 = 20.;
pub(super) const WRITER_POPUP_BOTTOM: f32 = 16. + 40. + POPUP_GAP;
pub(super) const WRITER_POPUP_TOP: f32 = 52.;

impl Workspace {
    pub(super) fn set_ui_mode(
        &mut self,
        mode: UiMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui_mode_picker = None;
        if self.ui_mode == mode {
            self.focus.focus(window);
            cx.notify();
            return;
        }

        if mode == UiMode::Writer {
            self.dev_panes = (self.explorer_visible, self.terminal_visible);
            self.explorer_visible = false;
            self.terminal_visible = false;
        }
        self.ui_mode = mode;
        self.writer_chrome_visible = true;
        self.explorer_resize = None;
        self.writer_tabs_open = false;
        self.help = false;
        self.dismiss_outline();
        self.finish_theme_picker(false, cx);
        self.marked = None;
        self.command = None;
        self.window_prefix = false;
        self.set_pane(Pane::Editor, cx);
        if mode == UiMode::Dev {
            self.explorer_visible = self.dev_panes.0;
            self.terminal_visible = self.dev_panes.1 && self.terminal.is_some();
        }
        self.focus.focus(window);
        cx.notify();
    }

    pub(super) fn toggle_ui_mode_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.writer_tabs_open = false;
        if self.ui_mode_picker.take().is_none() {
            self.help = false;
            self.dismiss_outline();
            self.finish_theme_picker(false, cx);
            self.set_pane(Pane::Editor, cx);
            self.ui_mode_picker = Some(self.ui_mode);
        }
        self.focus.focus(window);
        cx.notify();
    }

    pub(super) fn toggle_writer_chrome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.writer_chrome_visible = !self.writer_chrome_visible;
        if !self.writer_chrome_visible {
            self.writer_tabs_open = false;
            self.help = false;
            self.dismiss_outline();
            self.finish_theme_picker(false, cx);
            self.ui_mode_picker = None;
            self.set_pane(Pane::Editor, cx);
        }
        self.focus.focus(window);
        cx.notify();
    }

    pub(super) fn toggle_writer_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.writer_tabs_open = !self.writer_tabs_open;
        self.help = false;
        self.dismiss_outline();
        self.finish_theme_picker(false, cx);
        self.ui_mode_picker = None;
        self.set_pane(Pane::Editor, cx);
        self.focus.focus(window);
        cx.notify();
    }

    fn mode_glass(&self) -> Div {
        div()
            .relative()
            .bg(self.color(&self.config.theme.panel).opacity(0.96))
            .border_1()
            .border_color(self.color(&self.config.theme.foreground).opacity(0.24))
            .text_color(self.color(&self.config.theme.foreground))
            .shadow_lg()
    }

    pub(super) fn ui_mode_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        self.mode_glass()
            .id("ui-mode-control")
            .debug_selector(|| "ui-mode-control".into())
            .absolute()
            .top(px(4.))
            .right(px(16.))
            .w(px(104.))
            .h(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .occlude()
            .hover(|style| style.bg(accent.opacity(0.2)))
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_ui_mode_picker(window, cx);
                cx.stop_propagation();
            }))
            .child(match self.ui_mode {
                UiMode::Dev => "Dev",
                UiMode::Writer => "Writer",
            })
            .child(div().text_size(px(10.)).child("⌄"))
    }

    pub(super) fn ui_mode_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let selected = self.ui_mode_picker.unwrap_or(self.ui_mode);
        div()
            .id("ui-mode-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| {
                this.ui_mode_picker = None;
                this.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                self.mode_glass()
                    .id("ui-mode-selector")
                    .debug_selector(|| "ui-mode-selector".into())
                    .absolute()
                    .top(px(4. + 32. + POPUP_GAP))
                    .right(px(16.))
                    .w(px(264.))
                    .rounded_xl()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("Workspace mode"),
                    )
                    .children(
                        [
                            (
                                UiMode::Dev,
                                "ui-mode-dev",
                                "Dev",
                                "Tabs, files and terminal docked",
                            ),
                            (
                                UiMode::Writer,
                                "ui-mode-writer",
                                "Writer",
                                "Quiet page, floating tools",
                            ),
                        ]
                        .into_iter()
                        .map(|(mode, id, title, description)| {
                            div()
                                .id(id)
                                .debug_selector(move || id.into())
                                .rounded_lg()
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(if selected == mode {
                                    accent.opacity(0.6)
                                } else {
                                    transparent_black()
                                })
                                .bg(if selected == mode {
                                    accent.opacity(0.14)
                                } else {
                                    transparent_black()
                                })
                                .cursor_pointer()
                                .hover(|style| style.bg(accent.opacity(0.2)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_ui_mode(mode, window, cx);
                                    cx.stop_propagation();
                                }))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(description),
                                )
                        }),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child("↑ ↓ select · Return apply · Esc close"),
                    ),
            )
    }

    pub(super) fn writer_chrome(
        &self,
        mode: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        if !self.writer_chrome_visible {
            return div().absolute().inset_0().child(
                self.mode_glass()
                    .id("writer-restore-control")
                    .debug_selector(|| "writer-restore-control".into())
                    .absolute()
                    .bottom(px(16.))
                    .left(px(16.))
                    .size(px(40.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .occlude()
                    .hover(|style| style.bg(accent.opacity(0.2)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_writer_chrome(window, cx);
                        cx.stop_propagation();
                    }))
                    .child("Show"),
            );
        }
        div()
            .absolute()
            .inset_0()
            .child(
                self.mode_glass()
                    .id("writer-mode-indicator")
                    .debug_selector(|| "writer-mode-indicator".into())
                    .absolute()
                    .bottom(px(16.))
                    .left(px(16.))
                    .size(px(40.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(accent)
                    .tooltip(self.mode_tooltip())
                    .child(&mode[..1]),
            )
            .child(
                self.mode_glass()
                    .id("writer-tabs-toggle")
                    .debug_selector(|| "writer-tabs-toggle".into())
                    .absolute()
                    .bottom(px(16.))
                    .left(px(64.))
                    .size(px(40.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .occlude()
                    .when(self.writer_tabs_open, |button| button.text_color(accent))
                    .hover(|style| style.bg(accent.opacity(0.2)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_writer_tabs(window, cx);
                        cx.stop_propagation();
                    }))
                    .child("Tabs"),
            )
            .child(
                self.mode_glass()
                    .id("writer-chrome-toggle")
                    .debug_selector(|| "writer-chrome-toggle".into())
                    .absolute()
                    .bottom(px(16.))
                    .left(px(112.))
                    .size(px(40.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .occlude()
                    .hover(|style| style.bg(accent.opacity(0.2)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_writer_chrome(window, cx);
                        cx.stop_propagation();
                    }))
                    .child("Hide"),
            )
            .child(
                div()
                    .id("writer-tools")
                    .debug_selector(|| "writer-tools".into())
                    .absolute()
                    .bottom(px(16.))
                    .right(px(16.))
                    .flex()
                    .gap_2()
                    .occlude()
                    .child(self.dock_button("Export PDF", "export-pdf", false, cx))
                    .child(self.dock_button(
                        "Outline",
                        "outline",
                        self.outline_picker.is_some(),
                        cx,
                    ))
                    .child(self.dock_button("Files", "explorer", self.explorer_visible, cx))
                    .child(self.dock_button("Help", "help", self.help, cx)),
            )
    }

    pub(super) fn writer_tabs_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let width = (f32::from(window.viewport_size().width) - 32.).clamp(0., 420.);
        let max_height =
            (f32::from(window.viewport_size().height) - WRITER_POPUP_TOP - WRITER_POPUP_BOTTOM)
                .max(0.);
        let height = (44. + self.documents.entries().len() as f32 * 40. + 16.).min(max_height);
        div()
            .id("writer-tabs-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| {
                this.writer_tabs_open = false;
                this.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                self.mode_glass()
                    .id("writer-tabs-popup")
                    .debug_selector(|| "writer-tabs-popup".into())
                    .absolute()
                    .bottom(px(WRITER_POPUP_BOTTOM))
                    .left(px(16.))
                    .w(px(width))
                    .h(px(height))
                    .rounded_xl()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(44.))
                            .flex_shrink_0()
                            .px_4()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Tabs"),
                            )
                            .child(
                                div()
                                    .id("close-writer-tabs")
                                    .debug_selector(|| "close-writer-tabs".into())
                                    .size(px(28.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_size(px(18.))
                                    .text_color(muted)
                                    .hover(|style| {
                                        style.bg(accent.opacity(0.15)).text_color(accent)
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.writer_tabs_open = false;
                                        this.focus.focus(window);
                                        cx.notify();
                                        cx.stop_propagation();
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(self.buffer_bar(cx)),
            )
    }

    pub(super) fn writer_pane(
        &self,
        pane: Pane,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let files = pane == Pane::Explorer;
        let id = if files {
            "writer-files"
        } else {
            "writer-terminal"
        };
        let title = if files { "Files" } else { "Terminal" };
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let width = (f32::from(window.viewport_size().width) - 32.)
            .clamp(0., if files { 420. } else { 620. });
        let height =
            (f32::from(window.viewport_size().height) - WRITER_POPUP_TOP - WRITER_POPUP_BOTTOM)
                .clamp(0., 440.);
        div()
            .id("writer-pane-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| {
                this.set_pane(Pane::Editor, cx);
                this.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                self.mode_glass()
                    .id(id)
                    .debug_selector(move || id.into())
                    .absolute()
                    .right(px(16.))
                    .bottom(px(WRITER_POPUP_BOTTOM))
                    .w(px(width))
                    .h(px(height))
                    .rounded_xl()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(44.))
                            .flex_shrink_0()
                            .px_4()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title)
                                    .when(files && self.explorer.dirty(), |header| {
                                        header.child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(accent)
                                                .child("• staged changes"),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .id("close-writer-pane")
                                    .debug_selector(|| "close-writer-pane".into())
                                    .size(px(28.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_size(px(18.))
                                    .text_color(muted)
                                    .hover(|style| {
                                        style.bg(accent.opacity(0.15)).text_color(accent)
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_pane(Pane::Editor, cx);
                                        this.focus.focus(window);
                                        cx.notify();
                                        cx.stop_propagation();
                                    }))
                                    .child("×"),
                            ),
                    )
                    .when(files, |card| {
                        card.child(
                            div()
                                .px_4()
                                .pb_2()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(muted)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(self.explorer.directory.display().to_string()),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(self.surface(pane, cx)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(muted.opacity(0.14))
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(if files {
                                "Edit names with Vim · Return open · :w apply · :e! discard"
                            } else {
                                "Return execute · Ctrl-C interrupt · Ctrl-` hide"
                            }),
                    ),
            )
    }
}
