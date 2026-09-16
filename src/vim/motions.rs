use unicode_segmentation::UnicodeSegmentation;

use super::{
    Buffer, Line, Mode, Position, first_nonblank, floor_boundary, next_boundary, previous_boundary,
};

impl Buffer {
    pub(super) fn motion(&mut self, key: &str, count: usize, explicit: bool) -> bool {
        let mut target = self.position();
        let mut vertical = false;
        match key {
            "h" | "left" | "backspace" => {
                for _ in 0..count.min(self.col) {
                    target.col = previous_boundary(&self.lines[self.row].text, target.col);
                }
            }
            "l" | "right" => {
                for _ in 0..count.min(self.lines[self.row].text.len()) {
                    target.col = next_boundary(&self.lines[self.row].text, target.col);
                }
            }
            "j" | "down" | "k" | "up" => {
                vertical = true;
                let column = *self.preferred_column.get_or_insert_with(|| {
                    self.lines[self.row].text[..self.col]
                        .graphemes(true)
                        .count()
                });
                target.row = if key == "j" || key == "down" {
                    self.row.saturating_add(count).min(self.lines.len() - 1)
                } else {
                    self.row.saturating_sub(count)
                };
                target.col = self.lines[target.row]
                    .text
                    .grapheme_indices(true)
                    .nth(column)
                    .map_or(self.lines[target.row].text.len(), |(offset, _)| offset);
            }
            "w" | "b" | "e" => {
                for _ in 0..count {
                    let next = match key {
                        "w" => self.word_forward(target),
                        "b" => self.word_backward(target),
                        _ => self.word_end(target),
                    };
                    if next == target {
                        break;
                    }
                    target = next;
                }
            }
            "0" => target.col = 0,
            "^" => target.col = first_nonblank(&self.lines[self.row].text),
            "$" | "end" => {
                target.row = target
                    .row
                    .saturating_add(count - 1)
                    .min(self.lines.len() - 1);
                target.col = self.lines[target.row].text.len();
            }
            "home" => target.col = 0,
            "G" => {
                target.row = if explicit {
                    count.saturating_sub(1).min(self.lines.len() - 1)
                } else {
                    self.lines.len() - 1
                };
                target.col = first_nonblank(&self.lines[target.row].text);
            }
            "enter" => {
                target.row = target.row.saturating_add(count).min(self.lines.len() - 1);
                target.col = first_nonblank(&self.lines[target.row].text);
            }
            _ => return false,
        }
        self.row = target.row;
        self.col = target.col;
        if !vertical {
            self.preferred_column = None;
        }
        self.clamp();
        true
    }

    pub(super) fn word_forward(&self, mut position: Position) -> Position {
        let class = self.class_at(position);
        if class != 0 {
            loop {
                let Some(next) = self.next_position(position) else {
                    return position;
                };
                position = next;
                if self.class_at(position) != class {
                    break;
                }
            }
        }
        while self.class_at(position) == 0 {
            let Some(next) = self.next_position(position) else {
                break;
            };
            position = next;
        }
        position
    }

    pub(super) fn word_backward(&self, position: Position) -> Position {
        let Some(mut position) = self.previous_position(position) else {
            return position;
        };
        while self.class_at(position) == 0 {
            let Some(previous) = self.previous_position(position) else {
                return position;
            };
            position = previous;
        }
        let class = self.class_at(position);
        while let Some(previous) = self.previous_position(position) {
            if self.class_at(previous) != class {
                break;
            }
            position = previous;
        }
        position
    }

    pub(super) fn word_end(&self, position: Position) -> Position {
        self.word_end_including_current(self.next_position(position).unwrap_or(position))
    }

    pub(super) fn word_end_including_current(&self, mut position: Position) -> Position {
        while self.class_at(position) == 0 {
            let Some(next) = self.next_position(position) else {
                return position;
            };
            position = next;
        }
        let class = self.class_at(position);
        while let Some(next) = self.next_position(position) {
            if self.class_at(next) != class {
                break;
            }
            position = next;
        }
        position
    }

    pub(super) fn class_at(&self, position: Position) -> u8 {
        let text = &self.lines[position.row].text;
        let Some(grapheme) = text[position.col..].graphemes(true).next() else {
            return 0;
        };
        if grapheme.chars().all(char::is_whitespace) {
            0
        } else if grapheme
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            1
        } else {
            2
        }
    }

    fn next_position(&self, position: Position) -> Option<Position> {
        let text = &self.lines[position.row].text;
        if position.col < text.len() {
            Some(Position {
                row: position.row,
                col: next_boundary(text, position.col),
            })
        } else if position.row + 1 < self.lines.len() {
            Some(Position {
                row: position.row + 1,
                col: 0,
            })
        } else {
            None
        }
    }

    fn previous_position(&self, position: Position) -> Option<Position> {
        if position.col > 0 {
            Some(Position {
                row: position.row,
                col: previous_boundary(&self.lines[position.row].text, position.col),
            })
        } else if position.row > 0 {
            Some(Position {
                row: position.row - 1,
                col: self.lines[position.row - 1].text.len(),
            })
        } else {
            None
        }
    }

    pub(super) fn position(&self) -> Position {
        Position {
            row: self.row,
            col: self.col,
        }
    }

    pub(super) fn move_to(&mut self, position: Position) {
        self.row = position.row;
        self.col = position.col;
        self.preferred_column = None;
        self.clamp();
    }

    pub(super) fn clamp(&mut self) {
        if self.lines.is_empty() {
            let id = self.allocate_id();
            self.lines.push(Line {
                id,
                text: String::new(),
            });
        }
        self.row = self.row.min(self.lines.len() - 1);
        let text = &self.lines[self.row].text;
        self.col = floor_boundary(text, self.col.min(text.len()));
        if self.mode != Mode::Insert && self.col == text.len() && !text.is_empty() {
            self.col = previous_boundary(text, self.col);
        }
        if let Some(anchor) = &mut self.visual_anchor {
            anchor.row = anchor.row.min(self.lines.len() - 1);
            anchor.col = floor_boundary(&self.lines[anchor.row].text, anchor.col);
        }
    }
}
