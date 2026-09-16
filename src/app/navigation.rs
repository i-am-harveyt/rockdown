use super::{Pane, Workspace};
use crate::{markdown, outline, vim::Mode};
use gpui::*;
use unicode_segmentation::UnicodeSegmentation;

impl Workspace {
    /// Shape the destination as editable source, even while it is still shown as preview.
    /// This keeps arrow movement stable across Markdown syntax and heading font changes.
    pub(super) fn editable_navigation_row(
        &self,
        source_row: usize,
        window: &mut Window,
    ) -> Option<crate::surface::HitRow> {
        let width = self.layouts[Pane::Editor.index()].text_width;
        if width <= px(0.) {
            return None;
        }
        let text: SharedString = self.buffer().lines.get(source_row)?.text.clone().into();
        let runs = [TextRun {
            len: text.len(),
            font: font(self.config.font_family.clone()),
            color: self.color(&self.config.theme.foreground),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let line =
            window
                .text_system()
                .shape_line(text.clone(), px(self.config.font_size), &runs, None);
        let wrap = self
            .projection
            .get(source_row)
            .is_some_and(|row| row.table.is_none() && row.kind != markdown::BlockKind::Code);
        let wrapped = if wrap && line.width > width {
            window
                .text_system()
                .shape_text(text, px(self.config.font_size), &runs, Some(width), None)
                .ok()?
                .pop()
        } else {
            None
        };
        let line_height = px(self.config.line_height);
        let height = line_height
            * wrapped
                .as_ref()
                .map_or(1, |line| line.wrap_boundaries.len() + 1) as f32;
        Some(crate::surface::HitRow {
            source_row,
            origin: point(px(0.), px(0.)),
            line,
            raw: true,
            wrapped,
            line_height,
            height,
        })
    }

    pub(super) fn mouse_location(
        &self,
        pane: Pane,
        position: Point<Pixels>,
    ) -> Option<(usize, usize)> {
        let row = self.layouts[pane.index()]
            .rows
            .iter()
            .find(|row| position.y >= row.origin.y && position.y < row.origin.y + row.height)?;
        let index = row.index_for_position(position);
        Some((
            row.source_row,
            if row.raw {
                index
            } else {
                self.projection
                    .get(row.source_row)
                    .map_or(0, |projection| projection.source_column(index))
            },
        ))
    }

    pub(super) fn mouse_move(
        &mut self,
        pane: Pane,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.mouse_anchor = None;
            return;
        }
        let Some((anchor_pane, row, col)) = self.mouse_anchor else {
            return;
        };
        if anchor_pane != pane {
            return;
        }
        if let Some(head) = self.mouse_location(pane, event.position)
            && (head != (row, col) || matches!(self.buffer().mode, Mode::Visual | Mode::VisualLine))
        {
            self.buffer_mut().select_with_mouse((row, col), head);
            cx.notify();
        }
    }

    pub fn mouse_down(
        &mut self,
        pane: Pane,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mouse_anchor = None;
        self.preferred_visual_x = None;
        let hit = self.layouts[pane.index()].rows.iter().find(|row| {
            event.position.y >= row.origin.y && event.position.y < row.origin.y + row.height
        });
        if pane == Pane::Editor
            && self.documents.current().is_markdown()
            && (event.modifiers.platform || event.modifiers.control)
        {
            let link = self.layouts[pane.index()]
                .links
                .iter()
                .find(|link| link.bounds.contains(&event.position))
                .map(|link| link.url.as_str())
                .or_else(|| {
                    let row = hit.filter(|row| !row.raw)?;
                    let projection = self.projection.get(row.source_row)?;
                    let mut offset = 0;
                    for span in &projection.spans {
                        let start = offset;
                        offset += span.text.len();
                        let Some(url) = span.link.as_deref() else {
                            continue;
                        };
                        for (index, character) in span.text.char_indices() {
                            let index = start + index;
                            let origin = row.position_for_index(index);
                            let width = row.line.x_for_index(index + character.len_utf8())
                                - row.line.x_for_index(index);
                            if Bounds::new(origin, size(width, row.line_height))
                                .contains(&event.position)
                            {
                                return Some(url);
                            }
                        }
                    }
                    None
                });
            if let Some(link) = link {
                if link.starts_with('#') {
                    if let Some(row) = outline::resolve_anchor(&self.headings, link) {
                        self.jump_to_heading(row, window, cx);
                    } else {
                        self.set_message(format!("No heading matches {link}"));
                    }
                } else {
                    self.set_message(
                        "Only #fragment links within this document are navigated.".into(),
                    );
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        let task = hit.and_then(|row| {
            if pane != Pane::Editor
                || !self.documents.current().is_markdown()
                || event.click_count != 1
                || event.modifiers.shift
                || event.modifiers.control
                || event.modifiers.alt
                || event.modifiers.platform
            {
                return None;
            }
            let marker = self.projection.get(row.source_row)?.task_marker?;
            let display = if row.raw {
                marker
            } else {
                row.line
                    .text
                    .find("[ ]")
                    .or_else(|| row.line.text.find("[x]"))?
            };
            // Check actual glyph rectangles: a narrow column can wrap even the
            // marker, and later visual rows must remain ordinary text targets.
            (display..display + 3)
                .any(|index| {
                    let origin = row.position_for_index(index);
                    let width = row.line.x_for_index(index + 1) - row.line.x_for_index(index);
                    event.position.x >= origin.x
                        && event.position.x < origin.x + width
                        && event.position.y >= origin.y
                        && event.position.y < origin.y + row.line_height
                })
                .then_some((row.source_row, marker))
        });
        if let Some((row, marker)) = task {
            self.set_pane(pane, cx);
            self.focus.focus(window);
            if self.buffer_mut().toggle_markdown_task(row, marker) {
                self.marked = None;
                self.refresh_projection();
            }
            cx.notify();
            return;
        }
        let location = self.mouse_location(pane, event.position);
        self.set_pane(pane, cx);
        self.focus.focus(window);
        if pane != Pane::Terminal
            && let Some((row, col)) = location
        {
            if self.buffer().mode != Mode::Insert {
                self.buffer_mut().key("escape");
            }
            let buffer = self.buffer_mut();
            buffer.row = row.min(buffer.lines.len() - 1);
            let text = &buffer.lines[buffer.row].text;
            buffer.col = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain((buffer.mode == Mode::Insert).then_some(text.len()))
                .take_while(|i| *i <= col)
                .last()
                .unwrap_or(0);
            if event.click_count == 2 {
                buffer.select_word_with_mouse();
            } else {
                self.mouse_anchor = Some((pane, buffer.row, buffer.col));
            }
        }
        cx.notify();
    }
    pub fn scroll(&mut self, pane: Pane, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.viewport_alignment = None;
        if pane != Pane::Terminal {
            let delta: f32 = event
                .delta
                .pixel_delta(px(self.config.line_height))
                .y
                .into();
            let count = if pane == Pane::Editor {
                self.documents.current().buffer.lines.len()
            } else {
                self.explorer.buffer.lines.len()
            };
            let index = pane.index();
            self.row_heights[index].resize(count, self.config.line_height);
            let top = &mut self.tops[index];
            *top = (*top).min(count - 1);
            let offset = &mut self.scroll_offsets[index];
            *offset -= delta;
            while *offset >= self.row_heights[index][*top] && *top + 1 < count {
                *offset -= self.row_heights[index][*top];
                *top += 1;
            }
            while *offset < 0. && *top > 0 {
                *top -= 1;
                *offset += self.row_heights[index][*top];
            }
            let minimum = if pane == Pane::Editor && *top == 0 {
                -(self.pane_heights[index] - self.config.line_height).max(0.) / 2.
            } else {
                0.
            };
            *offset = offset.clamp(minimum, (self.row_heights[index][*top] - 1.).max(0.));
            self.follow_cursor = false;
            cx.notify();
        }
    }
    pub fn ensure_cursor_visible(&mut self, pane: Pane, height: f32) {
        if pane == Pane::Terminal {
            return;
        }
        if pane == Pane::Editor
            && self.pane_heights[pane.index()] == 0.
            && self.tops[pane.index()] == 0
            && self.scroll_offsets[pane.index()] == 0.
        {
            self.scroll_offsets[pane.index()] = -(height - self.config.line_height).max(0.) / 2.;
        }
        self.pane_heights[pane.index()] = height;
        let rows = (height / self.config.line_height).max(1.) as usize;
        let count = if pane == Pane::Editor {
            self.documents.current().buffer.lines.len()
        } else {
            self.explorer.buffer.lines.len()
        };
        self.row_heights[pane.index()].resize(count, self.config.line_height);
        self.tops[pane.index()] = self.tops[pane.index()].min(count - 1);
        // Negative offsets expose the space above the document, never above
        // an interior row. Re-clamp it when the viewport is resized.
        let minimum = if pane == Pane::Editor && self.tops[pane.index()] == 0 {
            -(height - self.config.line_height).max(0.) / 2.
        } else {
            0.
        };
        self.scroll_offsets[pane.index()] = self.scroll_offsets[pane.index()].max(minimum);
        if let Some((target, center)) = self.viewport_alignment
            && target == pane
        {
            let index = pane.index();
            let mut top = self.buffer().row;
            let mut space = if center {
                (height - self.config.line_height).max(0.) / 2.
            } else {
                0.
            };
            while top > 0 && space > 0. {
                top -= 1;
                space -= self.row_heights[index][top];
            }
            self.tops[index] = top;
            self.scroll_offsets[index] = if pane == Pane::Editor {
                -space
            } else {
                (-space).max(0.)
            };
            return;
        }
        if self.follow_cursor && pane == self.pane && pane != Pane::Terminal {
            let row = self.buffer().row;
            let top = &mut self.tops[pane.index()];
            if row < *top {
                *top = row;
                self.scroll_offsets[pane.index()] = 0.;
            } else if row >= *top + rows {
                *top = row.saturating_sub(rows.saturating_sub(1));
            }
        }
    }

    pub fn viewport_is_aligned(&self, pane: Pane) -> bool {
        self.viewport_alignment
            .is_some_and(|(target, _)| target == pane)
    }
}
