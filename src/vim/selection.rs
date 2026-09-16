use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{Buffer, Mode, Position, floor_boundary, next_boundary, ordered, previous_boundary};

impl Buffer {
    /// Mouse selection uses the existing inclusive Visual selection and register behavior.
    pub fn select_with_mouse(&mut self, anchor: (usize, usize), head: (usize, usize)) {
        if self.mode == Mode::Insert {
            self.key("escape");
        }
        self.visual_anchor = Some(Position {
            row: anchor.0,
            col: anchor.1,
        });
        self.row = head.0;
        self.col = head.1;
        self.mode = Mode::Visual;
        self.pending = None;
        self.count = None;
        self.preferred_column = None;
        self.clamp();
    }

    pub fn select_word_with_mouse(&mut self) {
        let text = &self.lines[self.row].text;
        let range = text
            .split_word_bound_indices()
            .find(|(start, part)| *start <= self.col && self.col < *start + part.len())
            .map(|(start, part)| (start, previous_boundary(text, start + part.len())));
        if let Some((start, end)) = range {
            self.select_with_mouse((self.row, start), (self.row, end));
        }
    }

    pub fn selected_range(&self, row: usize) -> Option<Range<usize>> {
        if !matches!(self.mode, Mode::Visual | Mode::VisualLine) || row >= self.lines.len() {
            return None;
        }
        let anchor = self.visual_anchor?;
        let (start, end) = ordered(anchor, self.position());
        if row < start.row || row > end.row {
            return None;
        }
        let text = &self.lines[row].text;
        if self.mode == Mode::VisualLine {
            return Some(0..text.len());
        }
        let first = if row == start.row {
            floor_boundary(text, start.col)
        } else {
            0
        };
        let last = if row == end.row {
            next_boundary(text, floor_boundary(text, end.col))
        } else {
            text.len()
        };
        Some(first..last)
    }

    pub(super) fn visual_bounds(&self) -> (Position, Position) {
        let (start, mut end) = ordered(
            self.visual_anchor.unwrap_or(self.position()),
            self.position(),
        );
        end.col = next_boundary(&self.lines[end.row].text, end.col);
        (start, end)
    }

    pub(super) fn operate_visual(&mut self, operator: char) {
        let (start, end) = self.visual_bounds();
        let linewise = self.mode == Mode::VisualLine;
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        if linewise {
            self.operate_lines(operator, start.row, end.row);
        } else {
            self.operate_range(operator, start, end);
        }
        if operator == 'y' {
            self.move_to(if linewise {
                Position {
                    row: start.row,
                    col: 0,
                }
            } else {
                start
            });
        }
    }
}
