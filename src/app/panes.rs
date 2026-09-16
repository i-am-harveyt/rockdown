use super::{Pane, UiMode, Workspace};
use crate::terminal::Terminal;
use anyhow::Result;
use gpui::*;
use std::time::Duration;

impl Workspace {
    pub(super) fn set_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.viewport_alignment = None;
        self.preferred_visual_x = None;
        self.mouse_anchor = None;
        if self.ui_mode == UiMode::Writer {
            self.explorer_visible = pane == Pane::Explorer;
            self.terminal_visible = pane == Pane::Terminal;
        }
        if pane == Pane::Explorer {
            self.explorer_visible = true;
        }
        self.pane = pane;
        self.command = None;
        self.marked = None;
        self.follow_cursor = true;
        cx.notify();
    }
    pub(super) fn toggle_explorer(&mut self, cx: &mut Context<Self>) {
        self.explorer_resize = None;
        if self.explorer_visible {
            self.explorer_visible = false;
            if self.pane == Pane::Explorer {
                self.set_pane(Pane::Editor, cx);
            }
        } else {
            self.set_pane(Pane::Explorer, cx);
        }
        cx.notify();
    }
    pub(super) fn explorer_width(&self, window: &Window) -> f32 {
        Self::clamp_explorer_width(self.config.explorer_width, window)
    }
    fn clamp_explorer_width(width: f32, window: &Window) -> f32 {
        let max = (f32::from(window.viewport_size().width) - 320.).clamp(0., 600.);
        width.clamp(180_f32.min(max), max)
    }
    pub fn start_explorer_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_resize = Some((event.position.x, self.explorer_width(window)));
        cx.stop_propagation();
        cx.notify();
    }
    pub fn resize_explorer(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start_x, start_width)) = self.explorer_resize else {
            return;
        };
        if !self.explorer_visible || !event.dragging() {
            self.stop_explorer_resize(cx);
            return;
        }
        self.config.explorer_width =
            Self::clamp_explorer_width(start_width + f32::from(start_x - event.position.x), window);
        cx.notify();
    }
    pub fn stop_explorer_resize(&mut self, cx: &mut Context<Self>) {
        if self.explorer_resize.take().is_some() {
            cx.notify();
        }
    }
    pub(super) fn toggle_terminal(&mut self, cx: &mut Context<Self>) -> Result<()> {
        if self.terminal.is_none() {
            self.terminal = Some(Terminal::spawn(
                &self.explorer.directory,
                &self.config.shell,
                12,
                100,
            )?);
            self.terminal_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    Timer::after(Duration::from_millis(16)).await;
                    let running = this
                        .update(cx, |this, cx| {
                            let Some(terminal) = &mut this.terminal else {
                                return false;
                            };
                            let changed = terminal.poll();
                            let exited = terminal.exited();
                            if changed {
                                if let Some(error) = terminal.error().map(str::to_owned) {
                                    this.set_message(error);
                                }
                                if this.terminal_visible {
                                    cx.notify();
                                }
                            }
                            if exited {
                                this.terminal_visible = false;
                                this.terminal = None;
                                if this.pane == Pane::Terminal {
                                    this.set_pane(Pane::Editor, cx);
                                }
                                cx.notify();
                                return false;
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !running {
                        break;
                    }
                }
            }));
            self.terminal_visible = true;
        } else {
            self.terminal_visible = !self.terminal_visible;
        }
        self.set_pane(
            if self.terminal_visible {
                Pane::Terminal
            } else {
                Pane::Editor
            },
            cx,
        );
        Ok(())
    }
}
