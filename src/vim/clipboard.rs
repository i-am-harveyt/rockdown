use super::{
    Buffer, Line, Mode, Register, first_nonblank, next_boundary, normalize_newlines,
    previous_boundary,
};

impl Buffer {
    /// Import clipboard text without treating a paste as a new yank.
    pub fn set_clipboard(&mut self, text: Option<String>, linewise: bool) {
        self.register = text.map(|text| {
            let text = normalize_newlines(&text);
            if linewise {
                Register::Lines(
                    text.strip_suffix('\n')
                        .unwrap_or(&text)
                        .split('\n')
                        .map(str::to_owned)
                        .collect(),
                )
            } else {
                Register::Characters(text.into_owned())
            }
        });
        self.register_changed = false;
    }

    /// Export only completed yank/delete/change operations, not motions or undo.
    pub fn take_yank(&mut self) -> Option<(String, bool)> {
        if !std::mem::take(&mut self.register_changed) {
            return None;
        }
        self.register.as_ref().map(|register| match register {
            Register::Characters(text) => (text.clone(), false),
            Register::Lines(lines) => {
                let mut text = lines.join("\n");
                text.push('\n');
                (text, true)
            }
        })
    }

    pub(super) fn paste_visual(&mut self, count: usize) {
        let Some(register) = self.register.clone() else {
            return;
        };
        let (start, end) = self.visual_bounds();
        let linewise = self.mode == Mode::VisualLine;
        let before = self.snapshot();
        self.register = Some(if linewise {
            Register::Lines(
                self.lines[start.row..=end.row]
                    .iter()
                    .map(|line| line.text.clone())
                    .collect(),
            )
        } else {
            Register::Characters(self.range_text(start, end))
        });
        self.register_changed = true;
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        let source_linewise = matches!(&register, Register::Lines(_));
        let text = match register {
            Register::Lines(lines) => {
                let mut text = lines.join("\n");
                text.push('\n');
                text.repeat(count)
            }
            Register::Characters(text) => text.repeat(count),
        };
        if linewise {
            self.replace_lines_with_text(start.row, end.row, &text);
        } else {
            self.delete_range(start, end);
            if source_linewise {
                let prefix = self.col > 0;
                let suffix = self.col < self.lines[self.row].text.len();
                if prefix {
                    self.insert_literal("\n");
                }
                let insertion = self.position();
                self.insert_literal(text.strip_suffix('\n').unwrap_or(&text));
                if suffix {
                    self.insert_literal("\n");
                }
                self.move_to(insertion);
            } else {
                self.insert_literal(&text);
                self.move_to(start);
            }
        }
        self.record(before);
        self.clamp();
    }

    pub(super) fn paste(&mut self, after: bool, count: usize) {
        let Some(register) = self.register.clone() else {
            return;
        };
        let before = self.snapshot();
        match register {
            Register::Lines(texts) => {
                let first = self.row + usize::from(after);
                let mut insertion = first;
                for _ in 0..count {
                    for text in &texts {
                        let id = self.allocate_id();
                        self.lines.insert(
                            insertion,
                            Line {
                                id,
                                text: text.clone(),
                            },
                        );
                        insertion += 1;
                    }
                }
                self.row = first;
                self.col = first_nonblank(&self.lines[self.row].text);
                self.bump_revision();
            }
            Register::Characters(text) => {
                if after {
                    self.col = next_boundary(&self.lines[self.row].text, self.col);
                }
                let start = self.position();
                for _ in 0..count {
                    self.insert_literal(&text);
                }
                if text.contains('\n') {
                    self.row = start.row;
                    self.col = start.col;
                } else {
                    self.col = previous_boundary(&self.lines[self.row].text, self.col);
                }
            }
        }
        self.record(before);
        self.preferred_column = None;
        self.clamp();
    }
}
