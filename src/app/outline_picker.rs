use super::{
    OutlinePicker, Pane, UiMode, Workspace,
    ui_mode::{WRITER_POPUP_BOTTOM, WRITER_POPUP_TOP},
};
use gpui::{prelude::*, *};

impl Workspace {
    pub(super) fn dismiss_outline(&mut self) {
        if self.outline_picker.take().is_some() {
            self.marked = None;
        }
    }

    pub(super) fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        if self.outline_picker.is_some() {
            self.dismiss_outline();
            return;
        }
        self.help = false;
        self.set_pane(Pane::Editor, cx);
        let selected = self
            .headings
            .partition_point(|heading| heading.row <= self.documents.current().buffer.row)
            .saturating_sub(1);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        self.outline_picker = Some(OutlinePicker {
            query: String::new(),
            col: 0,
            matches: (0..self.headings.len()).collect(),
            selected,
            scroll,
            input_bounds: Bounds::default(),
            input_line: None,
        });
    }

    pub(super) fn filter_outline(&mut self) {
        let Some(picker) = &mut self.outline_picker else {
            return;
        };
        let query = picker.query.to_lowercase();
        picker.matches = self
            .headings
            .iter()
            .enumerate()
            .filter(|(_, heading)| heading.title.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        picker.selected = 0;
        picker.scroll.scroll_to_item(0);
    }

    pub(super) fn jump_to_heading(
        &mut self,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_outline();
        self.set_pane(Pane::Editor, cx);
        let buffer = &mut self.documents.current_mut().buffer;
        buffer.row = row.min(buffer.lines.len() - 1);
        buffer.col = 0;
        self.viewport_alignment = Some((Pane::Editor, false));
        self.follow_cursor = false;
        self.focus.focus(window);
        cx.notify();
    }

    pub(super) fn outline_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picker = self.outline_picker.as_ref().expect("open outline");
        let panel = self.color(&self.config.theme.panel);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let current = self
            .headings
            .partition_point(|heading| heading.row <= self.documents.current().buffer.row)
            .checked_sub(1);
        let empty = if !self.documents.current().is_markdown() {
            "Outline is available for Markdown documents (.md)."
        } else if self.headings.is_empty() {
            "No headings yet. Add a Markdown heading to navigate."
        } else {
            "No headings match your search."
        };
        let entity = cx.entity();
        let input_entity = entity.clone();
        div()
            .id("outline-overlay")
            .absolute()
            .inset_0()
            .p_4()
            .flex()
            .items_center()
            .justify_end()
            .occlude()
            .when(self.ui_mode == UiMode::Writer, |overlay| {
                overlay
                    .items_end()
                    .pt(px(WRITER_POPUP_TOP))
                    .pb(px(WRITER_POPUP_BOTTOM))
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.dismiss_outline();
                this.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("outline-panel")
                    .debug_selector(|| "outline-panel".into())
                    .w(px(440.))
                    .max_w_full()
                    .max_h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_xl()
                    .bg(panel)
                    .border_1()
                    .border_color(muted.opacity(0.25))
                    .shadow_lg()
                    .when(self.ui_mode == UiMode::Writer, |card| {
                        card.bg(panel.opacity(0.96))
                            .border_color(foreground.opacity(0.2))
                    })
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .p_4()
                            .flex_shrink_0()
                            .text_color(foreground)
                            .child("Document Outline")
                            .child(
                                div()
                                    .mt_2()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child("Type to filter headings"),
                            ),
                    )
                    .child(
                        div()
                            .mx_4()
                            .mb_2()
                            .px_2()
                            .rounded_md()
                            .bg(self.color(&self.config.theme.background))
                            .child(
                                canvas(
                                    move |bounds, window, cx| {
                                        input_entity.update(cx, |this, _| {
                                            let picker =
                                                this.outline_picker.as_mut().expect("open outline");
                                            let line = window.text_system().shape_line(
                                                picker.query.clone().into(),
                                                px(14.),
                                                &[TextRun {
                                                    len: picker.query.len(),
                                                    font: font(this.config.font_family.clone()),
                                                    color: foreground,
                                                    background_color: None,
                                                    underline: None,
                                                    strikethrough: None,
                                                }],
                                                None,
                                            );
                                            picker.input_bounds = bounds;
                                            picker.input_line = Some(line.clone());
                                            (line, picker.col)
                                        })
                                    },
                                    move |bounds, (line, col), window, cx| {
                                        let focus = entity.read(cx).focus.clone();
                                        window.handle_input(
                                            &focus,
                                            ElementInputHandler::new(bounds, entity.clone()),
                                            cx,
                                        );
                                        let x = line.x_for_index(col);
                                        let shift = (x - bounds.size.width + px(3.)).max(px(0.));
                                        let origin = bounds.origin - point(shift, px(0.));
                                        window.with_content_mask(
                                            Some(ContentMask { bounds }),
                                            |window| {
                                                if let Err(error) =
                                                    line.paint(origin, px(30.), window, cx)
                                                {
                                                    eprintln!("Outline search rendering: {error}");
                                                }
                                                window.paint_quad(fill(
                                                    Bounds::new(
                                                        origin + point(x, px(5.)),
                                                        size(px(1.), px(20.)),
                                                    ),
                                                    accent,
                                                ));
                                            },
                                        );
                                    },
                                )
                                .w_full()
                                .h(px(30.)),
                            ),
                    )
                    .child(
                        div()
                            .id("outline-results")
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&picker.scroll)
                            .px_2()
                            .pb_2()
                            .when(picker.matches.is_empty(), |list| {
                                list.child(
                                    div()
                                        .id("outline-empty")
                                        .debug_selector(|| "outline-empty".into())
                                        .p_4()
                                        .text_size(px(13.))
                                        .text_color(muted)
                                        .child(empty),
                                )
                            })
                            .children(picker.matches.iter().enumerate().map(
                                |(position, &index)| {
                                    let heading = &self.headings[index];
                                    let row = heading.row;
                                    div()
                                        .id(("outline-heading", index))
                                        .debug_selector(move || format!("outline-heading-{index}"))
                                        .h(px(34.))
                                        .flex_shrink_0()
                                        .pl(px(10. + f32::from(heading.level - 1) * 14.))
                                        .pr_2()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(if position == picker.selected {
                                            accent.opacity(0.12)
                                        } else {
                                            transparent_black()
                                        })
                                        .hover(|style| style.bg(accent.opacity(0.18)))
                                        .text_size(px(13.))
                                        .text_color(foreground)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.jump_to_heading(row, window, cx);
                                            cx.stop_propagation();
                                        }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(heading.title.clone()),
                                        )
                                        .when(current == Some(index), |item| {
                                            item.child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(accent)
                                                    .child("current"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(muted)
                                                .child(format!("{}", row + 1)),
                                        )
                                },
                            )),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(muted.opacity(0.15))
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("↑ ↓ select · Enter jump · Esc close"),
                    ),
            )
    }
}
