use super::{Pane, Workspace};
use crate::surface::Surface;
use gpui::{prelude::*, *};

impl Workspace {
    pub(super) fn buffer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.documents.active_id();
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let panel = self.color(&self.config.theme.panel);
        let background = self.color(&self.config.theme.background);
        div()
            .id("buffer-tabs")
            .debug_selector(|| "buffer-tabs".into())
            .h(px(48.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_3()
            .gap_2()
            .overflow_x_scroll()
            .bg(panel)
            .border_b_1()
            .border_color(muted.opacity(0.12))
            .text_xs()
            .children(self.documents.entries().iter().map(|entry| {
                let id = entry.id;
                let name = entry
                    .document
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into());
                let label = format!(
                    "{}{}",
                    name,
                    if entry.document.buffer.dirty() {
                        " •"
                    } else {
                        ""
                    }
                );
                let path = entry
                    .document
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Untitled".into());
                div()
                    .id(("buffer-tab", id))
                    .tooltip(move |_, cx| {
                        cx.new(|_| ControlTooltip {
                            label: path.clone().into(),
                            background,
                            foreground: muted,
                            border: accent.opacity(0.3),
                        })
                        .into()
                    })
                    .h(px(32.))
                    .rounded_md()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .bg(if id == active { background } else { panel })
                    .hover(|style| {
                        style
                            .bg(muted.opacity(0.08))
                            .text_color(self.color(&self.config.theme.foreground))
                    })
                    .active(|style| style.bg(accent.opacity(0.12)))
                    .text_color(if id == active {
                        self.color(&self.config.theme.foreground)
                    } else {
                        muted
                    })
                    .border_1()
                    .border_color(if id == active {
                        muted.opacity(0.18)
                    } else {
                        transparent_black()
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Err(error) =
                            this.change_document(|documents| documents.select(id), cx)
                        {
                            this.set_message(error.to_string());
                        }
                        this.focus.focus(window);
                        cx.notify();
                    }))
                    .child(label)
                    .child(
                        div()
                            .id(("close-buffer", id))
                            .w(px(22.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(17.))
                            .text_color(muted)
                            .hover(|style| style.bg(accent.opacity(0.18)).text_color(accent))
                            .active(|style| style.bg(accent.opacity(0.3)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, window, cx);
                            }))
                            .child("×"),
                    )
            }))
    }

    pub(super) fn dock_button(
        &self,
        label: &'static str,
        action: &'static str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        let background = self.color(&self.config.theme.background);
        let foreground = self.color(&self.config.theme.foreground);
        div()
            .id(action)
            .debug_selector(move || action.into())
            .w(px(if action == "outline" { 82. } else { 30. }))
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .rounded_md()
            .flex_shrink_0()
            .cursor_pointer()
            .bg(if selected {
                accent.opacity(0.1)
            } else {
                transparent_black()
            })
            .text_color(self.color(if selected {
                &self.config.theme.accent
            } else {
                &self.config.theme.muted
            }))
            .hover(|style| style.bg(accent.opacity(0.2)).text_color(accent))
            .active(|style| style.bg(accent.opacity(0.3)))
            .tooltip(move |_, cx| {
                cx.new(|_| ControlTooltip {
                    label: label.into(),
                    background,
                    foreground,
                    border: accent.opacity(0.3),
                })
                .into()
            })
            .on_click(cx.listener(move |this, _, window, cx| this.run_action(action, window, cx)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let p = |x, y| bounds.origin + point(px(x), px(y));
                        let mut path = PathBuilder::stroke(px(1.5));
                        match action {
                            "outline" => {
                                for (indent, y) in [(2., 5.), (6., 10.), (6., 15.)] {
                                    path.move_to(p(indent, y));
                                    path.line_to(p(18., y));
                                }
                            }
                            "terminal" => {
                                path.move_to(p(2., 3.));
                                path.line_to(p(18., 3.));
                                path.line_to(p(18., 17.));
                                path.line_to(p(2., 17.));
                                path.close();
                                path.move_to(p(5., 7.));
                                path.line_to(p(8., 10.));
                                path.line_to(p(5., 13.));
                                path.move_to(p(10., 13.));
                                path.line_to(p(15., 13.));
                            }
                            "explorer" => {
                                path.move_to(p(2., 5.));
                                path.line_to(p(8., 5.));
                                path.line_to(p(10., 7.));
                                path.line_to(p(18., 7.));
                                path.line_to(p(18., 16.));
                                path.line_to(p(2., 16.));
                                path.close();
                            }
                            "help" => {
                                path.move_to(p(10., 2.));
                                path.cubic_bezier_to(p(18., 10.), p(14.4, 2.), p(18., 5.6));
                                path.cubic_bezier_to(p(10., 18.), p(18., 14.4), p(14.4, 18.));
                                path.cubic_bezier_to(p(2., 10.), p(5.6, 18.), p(2., 14.4));
                                path.cubic_bezier_to(p(10., 2.), p(2., 5.6), p(5.6, 2.));
                                path.close();
                                path.move_to(p(7.5, 7.));
                                path.cubic_bezier_to(p(12.5, 7.), p(7.5, 4.5), p(12.5, 4.5));
                                path.cubic_bezier_to(p(10., 11.5), p(12.5, 9.5), p(10., 9.));
                                path.move_to(p(10., 14.));
                                path.line_to(p(10., 15.));
                            }
                            _ => unreachable!("unknown dock control"),
                        }
                        window.paint_path(
                            path.build().expect("valid control icon"),
                            window.text_style().color,
                        );
                    },
                )
                .size(px(20.)),
            )
            .when(action == "outline", |button| button.child("Outline"))
    }
    pub(super) fn surface(&self, pane: Pane, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(match pane {
                Pane::Editor => "editor",
                Pane::Explorer => "explorer",
                Pane::Terminal => "terminal",
            })
            .size_full()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, window, cx| {
                    this.mouse_down(pane, event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |this, event, _, cx| this.mouse_move(pane, event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.mouse_anchor = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.mouse_anchor = None),
            )
            .on_scroll_wheel(cx.listener(move |this, event, _, cx| this.scroll(pane, event, cx)))
            .when(pane == Pane::Editor, |surface| {
                surface.on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                    if !this.help
                        && this.theme_picker.is_none()
                        && this.outline_picker.is_none()
                        && this.command.is_none()
                    {
                        this.set_pane(Pane::Editor, cx);
                        let buffer = &this.documents.current().buffer;
                        let previous = (buffer.row, buffer.col);
                        if this.accepts_images()
                            && let Some((row, col)) =
                                this.mouse_location(Pane::Editor, window.mouse_position())
                        {
                            let buffer = &mut this.documents.current_mut().buffer;
                            buffer.row = row;
                            buffer.col = col;
                        }
                        this.import_image_files(paths.paths(), window, cx);
                        let buffer = &mut this.documents.current_mut().buffer;
                        // The async request captured the drop caret. Leave the
                        // live caret alone until a successful insertion completes.
                        buffer.row = previous.0;
                        buffer.col = previous.1;
                    }
                }))
            })
            .child(Surface {
                workspace: cx.entity(),
                pane,
            })
    }
}

struct ControlTooltip {
    label: SharedString,
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
}

impl Render for ControlTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .text_color(self.foreground)
            .text_size(px(12.))
            .child(self.label.clone())
    }
}
