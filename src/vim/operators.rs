use super::{Buffer, Line, Mode, Position, Register, first_nonblank, next_boundary, ordered};

impl Buffer {
    pub(super) fn operator_motion(
        &mut self,
        operator: char,
        key: &str,
        count: usize,
        explicit: bool,
    ) {
        let start = self.position();
        if matches!(key, "j" | "down" | "k" | "up" | "G") {
            let target = match key {
                "j" | "down" => self.row.saturating_add(count).min(self.lines.len() - 1),
                "k" | "up" => self.row.saturating_sub(count),
                _ => {
                    if explicit {
                        count.saturating_sub(1).min(self.lines.len() - 1)
                    } else {
                        self.lines.len() - 1
                    }
                }
            };
            self.operate_lines(operator, self.row.min(target), self.row.max(target));
            return;
        }
        let mut end = start;
        match key {
            "$" => {
                end.row = end.row.saturating_add(count - 1).min(self.lines.len() - 1);
                end.col = self.lines[end.row].text.len();
            }
            "w" | "e" | "b" => {
                let change_word = operator == 'c' && key == "w" && self.class_at(start) != 0;
                for iteration in 0..count {
                    let next = if key == "b" {
                        self.word_backward(end)
                    } else if change_word && iteration == 0 {
                        self.word_end_including_current(end)
                    } else if key == "e" || change_word {
                        self.word_end(end)
                    } else {
                        self.word_forward(end)
                    };
                    if next == end && !(change_word && iteration == 0) {
                        break;
                    }
                    end = next;
                }
                if key == "e" || change_word {
                    end.col = next_boundary(&self.lines[end.row].text, end.col);
                }
                if key == "w" && !change_word && end.row > start.row && end.col == 0 {
                    // Vim's exclusive word motion does not eat the next line
                    // when only trailing whitespace remains on this line.
                    end.row -= 1;
                    end.col = self.lines[end.row].text.len();
                }
            }
            "h" | "left" | "0" | "^" | "l" | "right" => {
                if !self.motion(key, count, explicit) {
                    return;
                }
                end = self.position();
                // A rightward deletion may include the final character, unlike
                // the normal-mode cursor which cannot rest past it.
                if key == "l" || key == "right" {
                    end = start;
                    for _ in 0..count.min(self.lines[start.row].text.len()) {
                        end.col = next_boundary(&self.lines[start.row].text, end.col);
                    }
                }
                self.row = start.row;
                self.col = start.col;
            }
            _ => return,
        }
        let (start, end) = ordered(start, end);
        self.operate_range(operator, start, end);
    }

    /// A shift uses the same literal tab as Insert's Tab key. A leading run of
    /// spaces is outdented by one four-column tab stop without slicing Unicode.
    pub(super) fn shift_lines(&mut self, right: bool, start: usize, end: usize, levels: usize) {
        let before = self.snapshot();
        let indent = if right {
            "\t".repeat(levels)
        } else {
            String::new()
        };
        let width = levels.saturating_mul(4);
        let mut changed = false;
        for line in &mut self.lines[start..=end] {
            if right {
                if !line.text.is_empty() {
                    line.text.insert_str(0, &indent);
                    changed = true;
                }
            } else {
                let mut columns = 0;
                let mut bytes = 0;
                for byte in line.text.bytes() {
                    if columns >= width {
                        break;
                    }
                    match byte {
                        b' ' => columns += 1,
                        b'\t' => columns += 4 - columns % 4,
                        _ => break,
                    }
                    bytes += 1;
                }
                if bytes > 0 {
                    line.text.drain(..bytes);
                    changed = true;
                }
            }
        }
        if changed {
            self.bump_revision();
        }
        self.row = start;
        self.col = first_nonblank(&self.lines[start].text);
        self.preferred_column = None;
        self.record(before);
        self.clamp();
    }

    pub(super) fn operate_lines(&mut self, operator: char, start: usize, end: usize) {
        let start = start.min(self.lines.len() - 1);
        let end = end.max(start).min(self.lines.len() - 1);
        if matches!(operator, '>' | '<') {
            self.shift_lines(operator == '>', start, end, 1);
            return;
        }
        self.register = Some(Register::Lines(
            self.lines[start..=end]
                .iter()
                .map(|line| line.text.clone())
                .collect(),
        ));
        self.register_changed = true;
        if operator == 'y' {
            return;
        }
        let before = self.snapshot();
        let changes_text = if operator == 'c' {
            start != end || !self.lines[start].text.is_empty()
        } else {
            self.lines.len() != 1 || !self.lines[0].text.is_empty()
        };
        if changes_text {
            self.bump_revision();
        }
        if operator == 'c' {
            self.lines[start].text.clear();
            self.lines.drain(start + 1..=end);
            self.row = start;
            self.col = 0;
            self.mode = Mode::Insert;
            self.insert_start = Some(before);
        } else {
            self.lines.drain(start..=end);
            if self.lines.is_empty() {
                let id = self.allocate_id();
                self.lines.push(Line {
                    id,
                    text: String::new(),
                });
            }
            self.row = start.min(self.lines.len() - 1);
            self.col = first_nonblank(&self.lines[self.row].text);
            self.record(before);
        }
        self.preferred_column = None;
        self.clamp();
    }

    pub(super) fn operate_range(&mut self, operator: char, start: Position, end: Position) {
        if matches!(operator, '>' | '<') {
            self.shift_lines(operator == '>', start.row, end.row, 1);
            return;
        }
        if start != end {
            self.register = Some(Register::Characters(self.range_text(start, end)));
            self.register_changed = true;
        }
        if operator == 'y' {
            return;
        }
        let before = self.snapshot();
        self.delete_range(start, end);
        if operator == 'c' {
            self.mode = Mode::Insert;
            self.insert_start = Some(before);
        } else {
            self.record(before);
            self.clamp();
        }
    }
}
