use super::{Pane, Workspace};
use crate::vim::Mode;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

impl Workspace {
    pub(super) fn type_text(&mut self, text: &str) {
        if self.help || self.theme_picker.is_some() {
            return;
        }
        if self.outline_picker.is_some() {
            self.replace_input(None, text);
            return;
        }
        self.preferred_visual_x = None;
        if let Some(command) = &mut self.command {
            command.push_str(&text.replace(['\r', '\n'], ""));
        } else if self.pane == Pane::Terminal {
            if let Some(terminal) = &mut self.terminal {
                let result = if terminal.screen().bracketed_paste() {
                    terminal.send(format!("\x1b[200~{text}\x1b[201~").as_bytes())
                } else {
                    terminal.send(text.as_bytes())
                };
                if let Err(error) = result {
                    self.message = error.to_string();
                }
            }
        } else {
            self.buffer_mut().insert_text(text);
            if self.pane == Pane::Editor {
                self.refresh_projection();
            }
        }
        self.follow_cursor = true;
    }
}

fn utf8_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (offset, c) in text.char_indices() {
        if units + c.len_utf16() > utf16 {
            return offset;
        }
        units += c.len_utf16();
    }
    text.len()
}
fn utf16_offset(text: &str, utf8: usize) -> usize {
    text[..utf8.min(text.len())].encode_utf16().count()
}

impl Workspace {
    fn input_text(&self) -> &str {
        if let Some(picker) = &self.outline_picker {
            return &picker.query;
        }
        if self.pane == Pane::Terminal {
            ""
        } else if let Some(command) = &self.command {
            command
        } else {
            &self.buffer().lines[self.buffer().row].text
        }
    }
    fn input_col(&self) -> usize {
        if let Some(picker) = &self.outline_picker {
            return picker.col;
        }
        if self.pane == Pane::Terminal {
            0
        } else {
            self.command.as_ref().map_or(self.buffer().col, String::len)
        }
    }
    fn replace_input(&mut self, range: Option<Range<usize>>, text: &str) {
        if self.help || self.theme_picker.is_some() {
            return;
        }
        if let Some(picker) = &mut self.outline_picker {
            let range = range
                .or_else(|| self.marked.clone())
                .map(|range| {
                    utf8_offset(&picker.query, range.start)..utf8_offset(&picker.query, range.end)
                })
                .unwrap_or(picker.col..picker.col);
            let text = text.replace(['\r', '\n'], "");
            picker.col = range.start + text.len();
            picker.query.replace_range(range, &text);
            self.marked = None;
            self.filter_outline();
            return;
        }
        if self.pane == Pane::Terminal {
            if let Some(terminal) = &mut self.terminal
                && let Err(error) = terminal.send(text.as_bytes())
            {
                self.message = error.to_string();
            }
            return;
        }
        let range = range.or_else(|| self.marked.clone());
        if let Some(command) = &mut self.command {
            let range = range
                .map(|r| utf8_offset(command, r.start)..utf8_offset(command, r.end))
                .unwrap_or(command.len()..command.len());
            command.replace_range(range, text);
        } else if self.buffer().mode == Mode::Insert {
            if let Some(range) = range {
                let current = self.input_text();
                let start = utf8_offset(current, range.start);
                let end = utf8_offset(current, range.end);
                // IME ranges are UTF-16; extend to full graphemes before editing.
                let start = current
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(current.len()))
                    .take_while(|i| *i <= start)
                    .last()
                    .unwrap_or(0);
                let end = current
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .find(|i| *i >= end)
                    .unwrap_or(current.len());
                let count = current[start..end].graphemes(true).count();
                self.buffer_mut().col = start;
                for _ in 0..count {
                    self.buffer_mut().key("delete");
                }
            }
            self.type_text(text);
        }
        self.marked = None;
    }
}
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.input_text();
        let start = utf8_offset(text, range.start);
        let end = utf8_offset(text, range.end).max(start);
        *actual = Some(utf16_offset(text, start)..utf16_offset(text, end));
        Some(text[start..end].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.help
            || (self.outline_picker.is_none()
                && self.command.is_none()
                && self.pane != Pane::Terminal
                && self.buffer().mode != Mode::Insert)
        {
            return None;
        }
        let col = utf16_offset(self.input_text(), self.input_col());
        Some(UTF16Selection {
            range: col..col,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone()
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_input(range, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.help || self.theme_picker.is_some() {
            return;
        }
        let start = range.as_ref().or(self.marked.as_ref()).map_or_else(
            || utf16_offset(self.input_text(), self.input_col()),
            |r| r.start,
        );
        self.replace_input(range, text);
        self.marked = (!text.is_empty()).then_some(start..start + text.encode_utf16().count());
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if let Some(picker) = &self.outline_picker {
            let line = picker.input_line.as_ref()?;
            let shift = (line.x_for_index(picker.col) - picker.input_bounds.size.width + px(3.))
                .max(px(0.));
            let start = utf8_offset(&picker.query, range.start);
            return Some(
                Bounds::new(
                    picker.input_bounds.origin
                        + point((line.x_for_index(start) - shift).max(px(0.)), px(0.)),
                    size(px(2.), px(30.)),
                )
                .intersect(&picker.input_bounds),
            );
        }
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        let start = utf8_offset(self.input_text(), range.start);
        Some(
            Bounds::new(
                row.position_for_index(start),
                size(px(2.), px(self.config.line_height)),
            )
            .intersect(&bounds),
        )
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        if let Some(picker) = &self.outline_picker {
            let line = picker.input_line.as_ref()?;
            let shift = (line.x_for_index(picker.col) - picker.input_bounds.size.width + px(3.))
                .max(px(0.));
            let index = line.closest_index_for_x(position.x - picker.input_bounds.origin.x + shift);
            return Some(utf16_offset(&picker.query, index));
        }
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        Some(utf16_offset(
            self.input_text(),
            row.index_for_position(position),
        ))
    }
}
