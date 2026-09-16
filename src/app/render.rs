use super::{
    BufferDelete, EditorPane, ExplorerToggle, HelpToggle, NewDocument, NextBuffer, OpenDocument,
    OutlineToggle, Pane, Paste, PreviousBuffer, Save, SaveAs, StatusBarToggle, TabBarToggle,
    TerminalToggle, ThemesToggle, UiMode, UiModeToggle, Workspace,
};
use crate::vim::Mode;
use gpui::{prelude::*, *};

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.message_expiry_pending) {
            let timer = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(5));
            self.message_expiry_task = Some(cx.spawn(async move |this, cx| {
                timer.await;
                let _ = this.update(cx, |this, cx| {
                    this.message.clear();
                    cx.notify();
                });
            }));
        }
        let background = self.color(&self.config.theme.background);
        let panel = self.color(&self.config.theme.panel);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let writer = self.ui_mode == UiMode::Writer;
        let mode = if self.pane == Pane::Terminal {
            "TERMINAL"
        } else {
            match self.buffer().mode {
                Mode::Normal => "NORMAL",
                Mode::Insert => "INSERT",
                Mode::Visual => "VISUAL",
                Mode::VisualLine => "VISUAL LINE",
            }
        };
        let location = format!(
            "{}:{}",
            self.buffer().row + 1,
            self.buffer().lines[self.buffer().row].text[..self.buffer().col]
                .chars()
                .count()
                + 1
        );
        let explorer_width = self.explorer_width(window);
        let explorer_label = if self.explorer_visible {
            "Hide Files"
        } else {
            "Show Files"
        };
        let buffer_name = self
            .documents
            .current()
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let window_title = format!("Rockdown — {buffer_name}");
        window.set_window_title(&window_title);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(
                cx.listener(|this, _: &Save, window, cx| this.run_action("save", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewDocument, window, cx| this.run_action("new", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenDocument, window, cx| {
                    this.run_action("open", window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SaveAs, window, cx| this.run_action("save-as", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Paste, window, cx| this.run_action("paste", window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExplorerToggle, window, cx| {
                this.run_action("explorer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TerminalToggle, window, cx| {
                this.run_action("terminal", window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &EditorPane, window, cx| {
                    this.run_action("editor", window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &HelpToggle, window, cx| this.run_action("help", window, cx)),
            )
            .on_action(cx.listener(|this, _: &ThemesToggle, window, cx| {
                this.run_action("themes", window, cx)
            }))
            .on_action(cx.listener(|this, _: &UiModeToggle, window, cx| {
                this.run_action("ui-mode", window, cx)
            }))
            .on_action(cx.listener(|this, _: &OutlineToggle, window, cx| {
                this.run_action("outline", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TabBarToggle, window, cx| {
                this.run_action("tab-bar", window, cx)
            }))
            .on_action(cx.listener(|this, _: &StatusBarToggle, window, cx| {
                this.run_action("status-bar", window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousBuffer, window, cx| {
                this.run_action("previous-buffer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextBuffer, window, cx| {
                this.run_action("next-buffer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &BufferDelete, window, cx| {
                this.run_action("buffer-delete", window, cx)
            }))
            .on_mouse_move(cx.listener(Self::resize_explorer))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)),
            )
            .when(self.explorer_resize.is_some(), |root| {
                root.cursor(CursorStyle::ResizeLeftRight)
            })
            .when(cfg!(target_os = "macos") && !writer, |root| {
                root.child(
                    div()
                        .id("window-title")
                        .debug_selector(|| "window-title".into())
                        .h(px(40.))
                        .flex_shrink_0()
                        .px(px(80.))
                        .pr(px(144.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(panel)
                        .text_size(px(12.))
                        .text_color(muted)
                        .window_control_area(WindowControlArea::Drag)
                        .on_mouse_down(MouseButton::Left, |event, window, _| {
                            if event.click_count == 2 {
                                window.zoom_window();
                            } else {
                                window.start_window_move();
                            }
                        })
                        .child(div().overflow_hidden().text_ellipsis().child(window_title)),
                )
            })
            .child(
                // The transparent drag region keeps the native window movable
                // without bringing back a title bar in Writer Mode.
                div()
                    .absolute()
                    .top_0()
                    .left(px(80.))
                    .right(px(144.))
                    .h(px(if writer { 8. } else { 40. }))
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(self.config.tab_bar_visible && !writer, |column| {
                                column.child(self.buffer_bar(cx))
                            })
                            .child(
                                div().flex_1().min_h_0().flex().justify_center().child(
                                    div()
                                        .id("editor-column")
                                        .debug_selector(|| "editor-column".into())
                                        .w_full()
                                        .min_w_0()
                                        .h_full()
                                        .when(self.documents.current().is_markdown(), |column| {
                                            column.max_w(px(self.config.writing_width)).py_6()
                                        })
                                        .child(self.surface(Pane::Editor, cx)),
                                ),
                            ),
                    )
                    .when(self.explorer_visible && !writer, |body| {
                        body.child(
                            div()
                                .w(px(explorer_width))
                                .flex_shrink_0()
                                .flex()
                                .bg(panel)
                                .child(
                                    div()
                                        .id("explorer-splitter")
                                        .w(px(6.))
                                        .flex_shrink_0()
                                        .h_full()
                                        .cursor(CursorStyle::ResizeLeftRight)
                                        .child(div().w(px(1.)).h_full().bg(
                                            if self.pane == Pane::Explorer {
                                                accent.opacity(0.5)
                                            } else {
                                                muted.opacity(0.16)
                                            },
                                        ))
                                        .hover(|style| style.bg(accent.opacity(0.15)))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(Self::start_explorer_resize),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .h(px(48.))
                                                .flex_shrink_0()
                                                .px_4()
                                                .flex()
                                                .items_center()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(foreground)
                                                .child(if self.explorer.dirty() {
                                                    "Files  •"
                                                } else {
                                                    "Files"
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px_4()
                                                .pb_3()
                                                .text_size(px(11.))
                                                .text_color(muted)
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(
                                                    self.explorer.directory.display().to_string(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h_0()
                                                .child(self.surface(Pane::Explorer, cx)),
                                        ),
                                ),
                        )
                    }),
            )
            .when(self.terminal_visible && !writer, |root| {
                root.child(
                    div()
                        .h(px(self.config.terminal_height))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .border_t_1()
                        .border_color(muted.opacity(0.18))
                        .bg(background)
                        .child(
                            div()
                                .h(px(36.))
                                .flex_shrink_0()
                                .px_4()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(foreground)
                                        .child("Terminal"),
                                )
                                .child("Ctrl-` to hide"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(self.surface(Pane::Terminal, cx)),
                        ),
                )
            })
            .when_some(self.recovery_error.clone(), |root, error| {
                root.child(
                    div()
                        .id("recovery-error")
                        .debug_selector(|| "recovery-error".into())
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .bg(panel)
                        .border_t_1()
                        .border_color(accent)
                        .text_color(foreground)
                        .text_size(px(12.))
                        .when(writer, |notice| {
                            notice
                                .absolute()
                                .top(px(52.))
                                .left(px(80.))
                                .right(px(144.))
                                .rounded_lg()
                                .shadow_lg()
                        })
                        .child(error),
                )
            })
            .when(self.config.status_bar_visible && !writer, |root| {
                root.child(
                    div()
                        .id("status-bar")
                        .debug_selector(|| "status-bar".into())
                        .h(px(38.))
                        .flex_shrink_0()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(panel)
                        .border_t_1()
                        .border_color(muted.opacity(0.12))
                        .text_size(px(11.))
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(accent.opacity(0.1))
                                .text_color(accent)
                                .font_weight(FontWeight::MEDIUM)
                                .child(mode),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(if self.command.is_some() {
                                    foreground
                                } else {
                                    muted
                                })
                                .child(
                                    self.command.clone().unwrap_or_else(|| self.message.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(muted)
                                .font_family(self.config.font_family.clone())
                                .child(location),
                        )
                        .child(self.dock_button(
                            "Outline",
                            "outline",
                            self.outline_picker.is_some(),
                            cx,
                        ))
                        .when(self.terminal_visible, |footer| {
                            footer.child(self.dock_button("Hide Terminal", "terminal", true, cx))
                        })
                        .child(self.dock_button(
                            explorer_label,
                            "explorer",
                            self.explorer_visible,
                            cx,
                        ))
                        .child(self.dock_button(
                            if self.help { "Hide Help" } else { "Show Help" },
                            "help",
                            self.help,
                            cx,
                        )),
                )
            })
            .when(
                (!self.config.status_bar_visible || writer)
                    && (self.command.is_some() || !self.message.is_empty()),
                |root| {
                    root.child(
                        div()
                            .id("command-feedback")
                            .debug_selector(|| "command-feedback".into())
                            .flex_shrink_0()
                            .px_3()
                            .py_2()
                            .flex()
                            .gap_3()
                            .bg(panel)
                            .when(writer, |feedback| {
                                feedback
                                    .absolute()
                                    .bottom(px(16.))
                                    .left(px(if self.writer_chrome_visible {
                                        164.
                                    } else {
                                        84.
                                    }))
                                    .right(px(if self.writer_chrome_visible {
                                        192.
                                    } else {
                                        16.
                                    }))
                                    .rounded_xl()
                                    .border_1()
                                    .border_color(muted.opacity(0.25))
                                    .shadow_lg()
                            })
                            .child(div().flex_1().min_w_0().child(
                                self.command.clone().unwrap_or_else(|| self.message.clone()),
                            ))
                            .when(self.command.is_none(), |feedback| {
                                feedback.child(
                                    div()
                                        .id("dismiss-feedback")
                                        .debug_selector(|| "dismiss-feedback".into())
                                        .cursor_pointer()
                                        .child("×")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_message(String::new());
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                },
            )
            .when(writer && self.writer_tabs_open, |root| {
                root.child(self.writer_tabs_popup(window, cx))
            })
            .when(writer && self.explorer_visible, |root| {
                root.child(self.writer_pane(Pane::Explorer, window, cx))
            })
            .when(writer && self.terminal_visible, |root| {
                root.child(self.writer_pane(Pane::Terminal, window, cx))
            })
            .when(self.help, |root| root.child(self.help_panel(cx)))
            .when(self.theme_picker.is_some(), |root| {
                root.child(self.theme_selector(cx))
            })
            .when(self.outline_picker.is_some(), |root| {
                root.child(self.outline_panel(cx))
            })
            .when(writer, |root| root.child(self.writer_chrome(mode, cx)))
            .when(self.ui_mode_picker.is_some(), |root| {
                root.child(self.ui_mode_selector(cx))
            })
            .when(!writer || self.writer_chrome_visible, |root| {
                root.child(self.ui_mode_control(cx))
            })
    }
}
