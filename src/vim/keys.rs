use unicode_segmentation::UnicodeSegmentation;

use super::{
    Buffer, Line, Mode, Pending, Position, ViewportMotion, first_nonblank, next_boundary,
    previous_boundary,
};

impl Buffer {
    pub fn take_viewport_motion(&mut self) -> Option<ViewportMotion> {
        self.viewport_motion.take()
    }

    pub fn key(&mut self, key: &str) -> bool {
        self.clamp();
        if key == "escape" {
            if self.mode == Mode::Insert {
                self.finish_insert();
                self.col = previous_boundary(&self.lines[self.row].text, self.col);
            }
            self.mode = Mode::Normal;
            self.visual_anchor = None;
            self.reset_command();
            self.clamp();
            return true;
        }
        if key == "ctrl-r" && self.mode != Mode::Insert {
            let count = self.count.take().unwrap_or(1);
            for _ in 0..count.min(self.redo_stack.len()) {
                self.redo();
            }
            return true;
        }
        if self.mode == Mode::Insert {
            return self.insert_key(key);
        }
        if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() {
            let digit = (key.as_bytes()[0] - b'0') as usize;
            if digit != 0 || self.count.is_some() {
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(digit),
                );
                return true;
            }
        }
        let explicit_count = self.count.is_some();
        let count = self.count.take().unwrap_or(1);
        if let Some(pending) = self.pending.take() {
            match pending {
                Pending::Z(row) => {
                    if matches!(key, "z" | "t") {
                        if let Some(row) = row {
                            self.move_to(Position {
                                row: row.saturating_sub(1),
                                col: self.col,
                            });
                        }
                        self.viewport_motion = Some(if key == "z" {
                            ViewportMotion::Center
                        } else {
                            ViewportMotion::Top
                        });
                    }
                    return true;
                }
                Pending::G => {
                    if key == "g" {
                        self.move_to(Position {
                            row: if explicit_count {
                                count.saturating_sub(1)
                            } else {
                                0
                            },
                            col: 0,
                        });
                    }
                    return true;
                }
                Pending::OperatorG(operator, operator_count) => {
                    if key == "g" {
                        let target = operator_count
                            .saturating_mul(count)
                            .saturating_sub(1)
                            .min(self.lines.len() - 1);
                        self.operate_lines(
                            operator,
                            self.row.min(target),
                            self.row.max(target).min(self.lines.len() - 1),
                        );
                    }
                    return true;
                }
                Pending::Operator(operator, operator_count) => {
                    let total = operator_count.saturating_mul(count);
                    if key == "g" {
                        self.pending = Some(Pending::OperatorG(operator, total));
                    } else if key.len() == 1 && key.as_bytes()[0] == operator as u8 {
                        self.operate_lines(
                            operator,
                            self.row,
                            self.row.saturating_add(total - 1).min(self.lines.len() - 1),
                        );
                    } else {
                        self.operator_motion(
                            operator,
                            key,
                            total,
                            explicit_count || operator_count != 1,
                        );
                    }
                    return true;
                }
            }
        }
        if matches!(key, "ctrl-d" | "ctrl-u") {
            if explicit_count {
                self.scroll_rows = Some(count);
            }
            let rows = self.scroll_rows.unwrap_or((self.page_rows / 2).max(1));
            let down = key == "ctrl-d";
            let before = self.row;
            self.motion(if down { "j" } else { "k" }, rows, true);
            self.viewport_motion = Some(ViewportMotion::HalfPage {
                down,
                rows: self.row.abs_diff(before),
            });
            return true;
        }
        if matches!(self.mode, Mode::Visual | Mode::VisualLine) {
            match key {
                "v" | "V" => {
                    let mode = if key == "V" {
                        Mode::VisualLine
                    } else {
                        Mode::Visual
                    };
                    if self.mode == mode {
                        self.mode = Mode::Normal;
                        self.visual_anchor = None;
                    } else {
                        self.mode = mode;
                    }
                }
                "y" | "d" | "c" => self.operate_visual(key.as_bytes()[0] as char),
                ">" | "<" => {
                    let (start, end) = self.visual_bounds();
                    self.visual_anchor = None;
                    self.mode = Mode::Normal;
                    self.shift_lines(key == ">", start.row, end.row, count);
                }
                "p" | "P" => self.paste_visual(count),
                "g" => {
                    self.pending = Some(Pending::G);
                    self.count = if explicit_count { Some(count) } else { None };
                }
                _ => return self.motion(key, count, explicit_count),
            }
            return true;
        }
        match key {
            "z" => self.pending = Some(Pending::Z(explicit_count.then_some(count))),
            "g" => {
                self.pending = Some(Pending::G);
                self.count = if explicit_count { Some(count) } else { None };
            }
            "d" | "c" | "y" | ">" | "<" => {
                self.pending = Some(Pending::Operator(key.as_bytes()[0] as char, count))
            }
            "v" | "V" => {
                self.visual_anchor = Some(self.position());
                self.mode = if key == "V" {
                    Mode::VisualLine
                } else {
                    Mode::Visual
                };
                if self.mode == Mode::VisualLine && count > 1 {
                    self.motion("j", count - 1, true);
                }
            }
            "i" | "a" | "I" | "A" | "o" | "O" => self.enter_insert(key),
            "x" | "delete" => {
                let start = self.position();
                let mut end = start;
                for _ in 0..count.min(self.lines[self.row].text.len()) {
                    end.col = next_boundary(&self.lines[end.row].text, end.col);
                }
                self.operate_range('d', start, end);
            }
            "D" | "C" => self.operator_motion(if key == "D" { 'd' } else { 'c' }, "$", count, true),
            "Y" => self.operate_lines(
                'y',
                self.row,
                self.row.saturating_add(count - 1).min(self.lines.len() - 1),
            ),
            "p" | "P" => self.paste(key == "p", count),
            "u" => {
                for _ in 0..count.min(self.undo_stack.len()) {
                    self.undo();
                }
            }
            _ => return self.motion(key, count, explicit_count),
        }
        true
    }

    fn insert_key(&mut self, key: &str) -> bool {
        match key {
            "left" | "right" | "up" | "down" => {
                self.motion(key, 1, false);
            }
            "backspace" => {
                self.ensure_insert_start();
                let end = self.position();
                if self.col > 0 {
                    self.delete_range(
                        Position {
                            row: self.row,
                            col: previous_boundary(&self.lines[self.row].text, self.col),
                        },
                        end,
                    );
                } else if self.row > 0 {
                    self.delete_range(
                        Position {
                            row: self.row - 1,
                            col: self.lines[self.row - 1].text.len(),
                        },
                        end,
                    );
                }
            }
            "delete" => {
                self.ensure_insert_start();
                let start = self.position();
                if self.col < self.lines[self.row].text.len() {
                    self.delete_range(
                        start,
                        Position {
                            row: self.row,
                            col: next_boundary(&self.lines[self.row].text, self.col),
                        },
                    );
                } else if self.row + 1 < self.lines.len() {
                    self.delete_range(
                        start,
                        Position {
                            row: self.row + 1,
                            col: 0,
                        },
                    );
                }
            }
            "enter" => self.insert_text("\n"),
            "tab" => self.insert_text("\t"),
            _ if key.graphemes(true).count() == 1 && !key.chars().any(char::is_control) => {
                self.insert_text(key)
            }
            _ => return false,
        }
        true
    }

    fn enter_insert(&mut self, key: &str) {
        self.insert_start = Some(self.snapshot());
        self.mode = Mode::Insert;
        self.preferred_column = None;
        match key {
            "a" => self.col = next_boundary(&self.lines[self.row].text, self.col),
            "I" => self.col = first_nonblank(&self.lines[self.row].text),
            "A" => self.col = self.lines[self.row].text.len(),
            "o" | "O" => {
                let row = self.row + usize::from(key == "o");
                let id = self.allocate_id();
                self.lines.insert(
                    row,
                    Line {
                        id,
                        text: String::new(),
                    },
                );
                self.bump_revision();
                self.row = row;
                self.col = 0;
            }
            _ => {}
        }
    }

    pub(super) fn reset_command(&mut self) {
        self.pending = None;
        self.count = None;
        self.preferred_column = None;
    }
}
