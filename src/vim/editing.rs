use super::{
    Buffer, Line, Mode, Position, ceil_boundary, first_nonblank, floor_boundary, normalize_newlines,
};

impl Buffer {
    /// Paste literal text, without interpreting key names. During Insert this is
    /// part of the current undo transaction; elsewhere it is one standalone edit.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clamp();
        let before = if self.mode == Mode::Insert {
            self.ensure_insert_start();
            None
        } else {
            Some(self.snapshot())
        };
        if self.mode == Mode::VisualLine {
            let (start, end) = self.visual_bounds();
            self.replace_lines_with_text(start.row, end.row, text);
            self.visual_anchor = None;
            self.mode = Mode::Normal;
        } else {
            if self.mode == Mode::Visual {
                let (start, end) = self.visual_bounds();
                self.delete_range(start, end);
                self.visual_anchor = None;
                self.mode = Mode::Normal;
            }
            self.insert_literal(text);
        }
        if let Some(before) = before {
            self.record(before);
            self.clamp();
        }
    }

    /// Continue a Markdown container in the current Insert undo transaction.
    /// The caller checks the parsed context so code and plain text stay literal.
    pub fn markdown_enter(&mut self) {
        use crate::markdown_edit::{EnterEdit, enter_edit};
        self.clamp();
        if self.mode != Mode::Insert {
            return;
        }
        match enter_edit(&self.lines[self.row].text, self.col) {
            Some(EnterEdit::Continue(text)) => self.insert_text(&text),
            Some(EnterEdit::Exit { from, to }) => {
                self.ensure_insert_start();
                self.delete_range(
                    Position {
                        row: self.row,
                        col: from,
                    },
                    Position {
                        row: self.row,
                        col: to,
                    },
                );
                self.col = from;
            }
            None => self.insert_text("\n"),
        }
    }

    /// Toggle a parser-identified task marker as a standalone undoable edit,
    /// retaining the current caret, selection, clipboard and editing mode.
    pub fn toggle_markdown_task(&mut self, row: usize, marker: usize) -> bool {
        let Some(text) = self.lines.get(row).map(|line| &line.text) else {
            return false;
        };
        let Some(end) = marker.checked_add(3) else {
            return false;
        };
        let Some(task) = text.get(marker..end) else {
            return false;
        };
        let replacement = match task {
            "[ ]" => "[x]",
            "[x]" | "[X]" => "[ ]",
            _ => return false,
        };
        self.finish_insert();
        let before = self.snapshot();
        self.lines[row].text.replace_range(marker..end, replacement);
        self.bump_revision();
        self.record(before);
        true
    }

    /// Replace physical rows, not a character range: neither neighbor may be
    /// joined to the paste, even when it has no final newline or reaches EOF.
    pub(super) fn replace_lines_with_text(&mut self, start: usize, end: usize, text: &str) {
        let text = normalize_newlines(text);
        let mut parts = text.strip_suffix('\n').unwrap_or(&text).split('\n');
        self.lines[start].text = parts.next().unwrap_or("").to_owned();
        let replacement: Vec<_> = parts
            .map(|part| Line {
                id: self.allocate_id(),
                text: part.to_owned(),
            })
            .collect();
        self.lines.splice(start + 1..=end, replacement);
        self.row = start;
        self.col = first_nonblank(&self.lines[start].text);
        self.preferred_column = None;
        self.bump_revision();
    }

    pub(super) fn insert_literal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = normalize_newlines(text);
        self.bump_revision();
        if !text.contains('\n') {
            self.lines[self.row].text.insert_str(self.col, &text);
            self.col = ceil_boundary(&self.lines[self.row].text, self.col + text.len());
            self.preferred_column = None;
            return;
        }
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or("");
        let tail = self.lines[self.row].text.split_off(self.col);
        self.lines[self.row].text.push_str(first);
        self.col += first.len();
        for part in parts {
            let id = self.allocate_id();
            self.row += 1;
            self.lines.insert(
                self.row,
                Line {
                    id,
                    text: part.to_owned(),
                },
            );
            self.col = part.len();
        }
        self.lines[self.row].text.push_str(&tail);
        // Combining marks or ZWJ sequences may merge with the following text.
        self.col = ceil_boundary(&self.lines[self.row].text, self.col);
        self.preferred_column = None;
    }

    pub(super) fn range_text(&self, start: Position, end: Position) -> String {
        if start.row == end.row {
            return self.lines[start.row].text[start.col..end.col].to_owned();
        }
        let mut result = self.lines[start.row].text[start.col..].to_owned();
        for line in &self.lines[start.row + 1..end.row] {
            result.push('\n');
            result.push_str(&line.text);
        }
        result.push('\n');
        result.push_str(&self.lines[end.row].text[..end.col]);
        result
    }

    pub(super) fn delete_range(&mut self, start: Position, end: Position) {
        if start != end {
            self.bump_revision();
        }
        if start.row == end.row {
            self.lines[start.row]
                .text
                .replace_range(start.col..end.col, "");
        } else {
            let tail = self.lines[end.row].text[end.col..].to_owned();
            self.lines[start.row].text.truncate(start.col);
            self.lines[start.row].text.push_str(&tail);
            self.lines.drain(start.row + 1..=end.row);
        }
        self.row = start.row;
        self.col = floor_boundary(&self.lines[start.row].text, start.col);
        self.preferred_column = None;
    }
}
