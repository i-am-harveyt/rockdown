use std::{borrow::Cow, ops::Range};

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub id: u64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Position {
    row: usize,
    col: usize,
}

#[derive(Clone)]
struct Snapshot {
    lines: Vec<Line>,
    cursor: Position,
}

#[derive(Clone)]
enum Register {
    Characters(String),
    Lines(Vec<String>),
}

#[derive(Clone, Copy)]
enum Pending {
    Operator(char, usize),
    G,
    OperatorG(char, usize),
    Z(Option<usize>),
}

#[derive(Clone, Copy)]
pub enum ViewportMotion {
    Center,
    Top,
    HalfPage { down: bool, rows: usize },
}

/// Upper bound on historical undo/redo snapshots to prevent unbounded heap growth.
const MAX_UNDO_DEPTH: usize = 100;

/// UTF-8 byte columns always point to a grapheme boundary. Existing line identities
/// survive text edits and splits; newly inserted or pasted lines get fresh IDs.
pub struct Buffer {
    pub lines: Vec<Line>,
    pub row: usize,
    pub col: usize,
    pub mode: Mode,
    next_id: u64,
    revision: u64,
    saved_text: String,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    insert_start: Option<Snapshot>,
    visual_anchor: Option<Position>,
    register: Option<Register>,
    register_changed: bool,
    pending: Option<Pending>,
    count: Option<usize>,
    preferred_column: Option<usize>,
    pub page_rows: usize,
    scroll_rows: Option<usize>,
    viewport_motion: Option<ViewportMotion>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let text = normalize_newlines(text);
        let mut next_id = 1;
        let lines = text
            .split('\n')
            .map(|text| {
                let line = Line {
                    id: next_id,
                    text: text.to_owned(),
                };
                next_id += 1;
                line
            })
            .collect();
        Self {
            lines,
            row: 0,
            col: 0,
            mode: Mode::Normal,
            next_id,
            revision: 0,
            saved_text: text.into_owned(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            insert_start: None,
            visual_anchor: None,
            register: None,
            register_changed: false,
            pending: None,
            page_rows: 1,
            scroll_rows: None,
            viewport_motion: None,
            count: None,
            preferred_column: None,
        }
    }

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

    pub fn take_viewport_motion(&mut self) -> Option<ViewportMotion> {
        self.viewport_motion.take()
    }

    /// Substitute each physical line, preserving identities and one undo transaction.
    pub fn substitute(&mut self, command: &str) -> anyhow::Result<usize> {
        use anyhow::{Context, bail};
        let mut chars = command.chars();
        let delimiter = chars.next().context("Use :%s/pattern/replacement/[giI]")?;
        if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
            bail!("Substitution requires a non-alphanumeric delimiter");
        }
        let field = |chars: &mut std::str::Chars<'_>, required: bool| -> anyhow::Result<String> {
            let mut result = String::new();
            while let Some(ch) = chars.next() {
                if ch == delimiter {
                    return Ok(result);
                }
                if ch == '\\' {
                    let next = chars.next().context("Trailing backslash in substitution")?;
                    if next == delimiter {
                        // Keep regex metacharacters literal when the delimiter is escaped.
                        if required {
                            result.push_str(&regex::escape(&next.to_string()));
                        } else {
                            if matches!(next, '&' | '\\') {
                                result.push('\\');
                            }
                            result.push(next);
                        }
                    } else {
                        result.push('\\');
                        result.push(next);
                    }
                } else {
                    result.push(ch);
                }
            }
            if required {
                bail!("Missing pattern delimiter");
            }
            Ok(result)
        };
        let pattern = field(&mut chars, true)?;
        if pattern.is_empty() {
            bail!("Substitution pattern must not be empty");
        }
        let replacement = field(&mut chars, false)?;
        let mut global = false;
        let mut ignore_case = false;
        for flag in chars.as_str().trim().chars() {
            match flag {
                'g' => global = true,
                'i' => ignore_case = true,
                'I' => ignore_case = false,
                _ => bail!("Unsupported substitution flag: {flag} (use g, i, or I)"),
            }
        }
        let regex = regex::RegexBuilder::new(&pattern)
            .case_insensitive(ignore_case)
            .build()
            .context("Invalid substitution pattern")?;
        enum Part {
            Text(String),
            Capture(usize),
        }
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut replacement = replacement.chars();
        while let Some(ch) = replacement.next() {
            let capture = match ch {
                '&' => Some(0),
                '\\' => {
                    let next = replacement
                        .next()
                        .context("Trailing backslash in replacement")?;
                    if let Some(index) = next.to_digit(10) {
                        Some(index as usize)
                    } else {
                        match next {
                            '\\' | '&' => literal.push(next),
                            'r' | 'n' => literal.push('\n'),
                            't' => literal.push('\t'),
                            _ => bail!("Unsupported replacement escape: \\{next}"),
                        }
                        None
                    }
                }
                _ => {
                    literal.push(ch);
                    None
                }
            };
            if let Some(index) = capture {
                if index >= regex.captures_len() {
                    bail!("No capture group \\{index} in pattern");
                }
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal)));
                }
                parts.push(Part::Capture(index));
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        let mut substitutions = 0;
        let mut patches = Vec::new();
        let mut last_row = self.row;
        let mut extra_rows = 0;
        for (row, line) in self.lines.iter().enumerate() {
            let before_count = substitutions;
            let text = regex.replacen(
                &line.text,
                if global { 0 } else { 1 },
                |captures: &regex::Captures<'_>| {
                    substitutions += 1;
                    let mut result = String::new();
                    for part in &parts {
                        match part {
                            Part::Text(text) => result.push_str(text),
                            Part::Capture(index) => {
                                if let Some(value) = captures.get(*index) {
                                    result.push_str(value.as_str());
                                }
                            }
                        }
                    }
                    result
                },
            );
            if substitutions != before_count {
                last_row = row + extra_rows;
            }
            if text != line.text {
                extra_rows += text.bytes().filter(|byte| *byte == b'\n').count();
                patches.push((row, text.into_owned()));
            }
        }
        if substitutions == 0 {
            bail!("Pattern not found: {pattern}");
        }
        if !patches.is_empty() {
            let before = self.snapshot();
            for (row, text) in patches.into_iter().rev() {
                let id = self.lines[row].id;
                let lines = text
                    .split('\n')
                    .enumerate()
                    .map(|(index, text)| Line {
                        id: if index == 0 { id } else { self.allocate_id() },
                        text: text.to_owned(),
                    })
                    .collect::<Vec<_>>();
                self.lines.splice(row..=row, lines);
            }
            self.bump_revision();
            self.record(before);
        }
        self.row = last_row;
        self.col = first_nonblank(&self.lines[self.row].text);
        self.reset_command();
        self.clamp();
        Ok(substitutions)
    }

    pub fn text(&self) -> String {
        let size = self.lines.iter().map(|line| line.text.len()).sum::<usize>()
            + self.lines.len().saturating_sub(1);
        let mut text = String::with_capacity(size);
        for (index, line) in self.lines.iter().enumerate() {
            if index != 0 {
                text.push('\n');
            }
            text.push_str(&line.text);
        }
        text
    }

    /// A content invalidation token, unchanged by cursor motion or saving.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replace the document as a fresh, saved buffer, retaining IDs for unchanged
    /// prefix/suffix lines and corresponding edited lines in between.
    pub fn set_text(&mut self, text: &str) {
        let text = normalize_newlines(text);
        let parts: Vec<&str> = text.split('\n').collect();
        if !self
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .eq(parts.iter().copied())
        {
            self.bump_revision();
        }
        let old = std::mem::take(&mut self.lines);
        let prefix = old
            .iter()
            .zip(&parts)
            .take_while(|(a, b)| a.text == **b)
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(parts[prefix..].iter().rev())
            .take_while(|(a, b)| a.text == **b)
            .count();
        let old_middle_end = old.len() - suffix;
        let new_middle_end = parts.len() - suffix;
        self.lines = parts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let old_index = if index < prefix {
                    Some(index)
                } else if index >= new_middle_end {
                    Some(old_middle_end + index - new_middle_end)
                } else if index < old_middle_end {
                    Some(index)
                } else {
                    None
                };
                Line {
                    id: old_index
                        .map(|i| old[i].id)
                        .unwrap_or_else(|| self.allocate_id()),
                    text: (*text).to_owned(),
                }
            })
            .collect();
        self.saved_text = text.into_owned();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.insert_start = None;
        self.visual_anchor = None;
        self.pending = None;
        self.count = None;
        self.mode = Mode::Normal;
        self.preferred_column = None;
        self.scroll_rows = None;
        self.viewport_motion = None;
        self.clamp();
    }

    pub fn dirty(&self) -> bool {
        !self
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .eq(self.saved_text.split('\n'))
    }

    pub fn mark_saved(&mut self) {
        self.saved_text = self.text();
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }

    fn push_redo(&mut self, snapshot: Snapshot) {
        if self.redo_stack.len() >= MAX_UNDO_DEPTH {
            self.redo_stack.remove(0);
        }
        self.redo_stack.push(snapshot);
    }

    pub fn undo(&mut self) {
        self.finish_insert();
        self.reset_command();
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        if let Some(previous) = self.undo_stack.pop() {
            let current = self.snapshot();
            self.restore(previous);
            self.push_redo(current);
        }
        self.clamp();
    }

    pub fn redo(&mut self) {
        self.finish_insert();
        self.reset_command();
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        if let Some(next) = self.redo_stack.pop() {
            let current = self.snapshot();
            self.restore(next);
            self.push_undo(current);
        }
        self.clamp();
    }

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
        if self.mode != Mode::Visual || row >= self.lines.len() {
            return None;
        }
        let anchor = self.visual_anchor?;
        let (start, end) = ordered(anchor, self.position());
        if row < start.row || row > end.row {
            return None;
        }
        let text = &self.lines[row].text;
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
        if self.mode == Mode::Visual {
            let (start, end) = self.visual_bounds();
            self.delete_range(start, end);
            self.visual_anchor = None;
            self.mode = Mode::Normal;
        }
        self.insert_literal(text);
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
        if self.mode == Mode::Visual {
            match key {
                "v" => {
                    self.mode = Mode::Normal;
                    self.visual_anchor = None;
                }
                "y" | "d" | "c" => self.operate_visual(key.as_bytes()[0] as char),
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
            "d" | "c" | "y" => {
                self.pending = Some(Pending::Operator(key.as_bytes()[0] as char, count))
            }
            "v" => {
                self.visual_anchor = Some(self.position());
                self.mode = Mode::Visual;
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

    fn motion(&mut self, key: &str, count: usize, explicit: bool) -> bool {
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

    fn operator_motion(&mut self, operator: char, key: &str, count: usize, explicit: bool) {
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

    fn operate_lines(&mut self, operator: char, start: usize, end: usize) {
        let start = start.min(self.lines.len() - 1);
        let end = end.max(start).min(self.lines.len() - 1);
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

    fn operate_range(&mut self, operator: char, start: Position, end: Position) {
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

    fn visual_bounds(&self) -> (Position, Position) {
        let (start, mut end) = ordered(
            self.visual_anchor.unwrap_or(self.position()),
            self.position(),
        );
        end.col = next_boundary(&self.lines[end.row].text, end.col);
        (start, end)
    }

    fn operate_visual(&mut self, operator: char) {
        let (start, end) = self.visual_bounds();
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.operate_range(operator, start, end);
        if operator == 'y' {
            self.move_to(start);
        }
    }

    fn paste(&mut self, after: bool, count: usize) {
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

    fn insert_literal(&mut self, text: &str) {
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

    fn range_text(&self, start: Position, end: Position) -> String {
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

    fn delete_range(&mut self, start: Position, end: Position) {
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

    fn word_forward(&self, mut position: Position) -> Position {
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

    fn word_backward(&self, position: Position) -> Position {
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

    fn word_end(&self, position: Position) -> Position {
        self.word_end_including_current(self.next_position(position).unwrap_or(position))
    }

    fn word_end_including_current(&self, mut position: Position) -> Position {
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

    fn class_at(&self, position: Position) -> u8 {
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

    fn position(&self) -> Position {
        Position {
            row: self.row,
            col: self.col,
        }
    }

    fn move_to(&mut self, position: Position) {
        self.row = position.row;
        self.col = position.col;
        self.preferred_column = None;
        self.clamp();
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("line identity space exhausted");
        id
    }

    fn clamp(&mut self) {
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

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.position(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        if !self
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .eq(snapshot.lines.iter().map(|line| line.text.as_str()))
        {
            self.bump_revision();
        }
        self.lines = snapshot.lines;
        self.row = snapshot.cursor.row;
        self.col = snapshot.cursor.col;
        self.preferred_column = None;
    }

    fn record(&mut self, before: Snapshot) {
        if before.lines != self.lines {
            self.push_undo(before);
            self.redo_stack.clear();
        }
    }

    fn ensure_insert_start(&mut self) {
        if self.insert_start.is_none() {
            self.insert_start = Some(self.snapshot());
        }
    }

    fn finish_insert(&mut self) {
        if let Some(before) = self.insert_start.take() {
            self.record(before);
        }
    }

    fn bump_revision(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("buffer revision space exhausted");
    }

    fn reset_command(&mut self) {
        self.pending = None;
        self.count = None;
        self.preferred_column = None;
    }
}

fn normalize_newlines(text: &str) -> Cow<'_, str> {
    // A lone CR is text, not a line separator.
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn ordered(a: Position, b: Position) -> (Position, Position) {
    if a <= b { (a, b) } else { (b, a) }
}

fn floor_boundary(text: &str, col: usize) -> usize {
    if col >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .take_while(|(offset, _)| *offset <= col)
        .last()
        .map_or(0, |(offset, _)| offset)
}

fn ceil_boundary(text: &str, col: usize) -> usize {
    text.grapheme_indices(true)
        .find(|(offset, _)| *offset >= col)
        .map_or(text.len(), |(offset, _)| offset)
}

fn next_boundary(text: &str, col: usize) -> usize {
    if col >= text.len() {
        return text.len();
    }
    text[col..]
        .graphemes(true)
        .next()
        .map_or(text.len(), |g| col + g.len())
}

fn previous_boundary(text: &str, col: usize) -> usize {
    text[..col.min(text.len())]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(offset, _)| offset)
}

fn first_nonblank(text: &str) -> usize {
    text.grapheme_indices(true)
        .find(|(_, g)| !g.chars().all(char::is_whitespace))
        .map_or(0, |(offset, _)| offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(buffer: &mut Buffer, keys: &[&str]) {
        for key in keys {
            assert!(buffer.key(key), "unconsumed key: {key}");
        }
    }

    #[test]
    fn markdown_enter_preserves_tail_and_undo() {
        let mut buffer = Buffer::new("- hello 世界");
        buffer.key("i");
        buffer.col = 8;
        buffer.markdown_enter();
        assert_eq!(buffer.text(), "- hello \n- 世界");
        assert_eq!(buffer.col, 2);
        buffer.key("escape");
        buffer.undo();
        assert_eq!(buffer.text(), "- hello 世界");
        buffer.redo();
        assert_eq!(buffer.text(), "- hello \n- 世界");
    }

    #[test]
    fn empty_task_exit_and_checkbox_toggles_are_undoable() {
        let mut buffer = Buffer::new("> - [ ] ");
        buffer.key("A");
        buffer.markdown_enter();
        assert_eq!(buffer.text(), "> ");
        buffer.key("escape");
        buffer.undo();
        assert_eq!(buffer.text(), "> - [ ] ");
        buffer.key("i");
        assert!(buffer.toggle_markdown_task(0, 4));
        assert_eq!(buffer.text(), "> - [x] ");
        assert_eq!(buffer.mode, Mode::Insert);
        assert!(buffer.take_yank().is_none());
        buffer.insert_text("typed");
        buffer.key("escape");
        buffer.undo();
        assert_eq!(buffer.text(), "> - [x] ");
        buffer.undo();
        assert_eq!(buffer.text(), "> - [ ] ");
        buffer.redo();
        assert_eq!(buffer.text(), "> - [x] ");
        assert!(!buffer.toggle_markdown_task(0, usize::MAX));
        assert!(!buffer.toggle_markdown_task(1, 0));
    }

    #[test]
    fn crlf_buffers_and_replacements_are_clean_and_keep_lone_cr() {
        let mut buffer = Buffer::new("one\r\ntwo\rthree\r\nlast\r");
        assert_eq!(buffer.text(), "one\ntwo\rthree\nlast\r");
        assert!(!buffer.dirty());
        let original = buffer.lines.clone();
        let revision = buffer.revision();
        buffer.set_text("one\r\ntwo\rthree\r\nlast\r");
        assert_eq!(buffer.lines, original);
        assert_eq!(buffer.revision(), revision);
        buffer.set_text("one\r\nnew\r\ntwo\rthree\r\nlast\r");
        assert_eq!(buffer.text(), "one\nnew\ntwo\rthree\nlast\r");
        assert_eq!(buffer.lines[2].id, original[1].id);
        assert_eq!(buffer.lines[3].id, original[2].id);
        assert!(!buffer.dirty());
    }

    #[test]
    fn literal_crlf_paste_is_one_undo_in_normal_insert_and_visual_modes() {
        for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
            let mut buffer = Buffer::new("ab");
            let original = buffer.lines.clone();
            match mode {
                Mode::Insert => keys(&mut buffer, &["i"]),
                Mode::Visual => keys(&mut buffer, &["v"]),
                Mode::Normal => {}
            }
            buffer.insert_text("x\r\ny\rz\r\n");
            keys(&mut buffer, &["escape"]);
            let expected = if mode == Mode::Visual {
                "x\ny\rz\nb"
            } else {
                "x\ny\rz\nab"
            };
            assert_eq!(buffer.text(), expected);
            assert!(buffer.dirty());
            buffer.undo();
            assert_eq!(buffer.lines, original);
            assert!(!buffer.dirty());
            buffer.redo();
            assert_eq!(buffer.text(), expected);
        }
    }

    #[test]
    fn clipboard_register_pastes_normalize_crlf_in_both_register_types() {
        for linewise in [false, true] {
            for key in ["p", "P"] {
                let mut buffer = Buffer::new("ab");
                let original = buffer.lines.clone();
                buffer.set_clipboard(Some("x\r\ny\rz\r\n".into()), linewise);
                assert!(buffer.take_yank().is_none());
                keys(&mut buffer, &[key]);
                let expected = match (linewise, key) {
                    (true, "p") => "ab\nx\ny\rz",
                    (true, _) => "x\ny\rz\nab",
                    (false, "p") => "ax\ny\rz\nb",
                    (false, _) => "x\ny\rz\nab",
                };
                assert_eq!(buffer.text(), expected);
                assert!(buffer.take_yank().is_none());
                buffer.undo();
                assert_eq!(buffer.lines, original);
                buffer.redo();
                assert_eq!(buffer.text(), expected);
            }
        }
    }

    #[test]
    fn substitutions_preserve_lines_and_undo_as_one_edit() {
        let mut buffer = Buffer::new("Cat cat\ncat/path\n世界");
        let original = buffer.lines.clone();
        assert_eq!(buffer.substitute("/(cat)/[\\1:&]/gi").unwrap(), 3);
        assert_eq!(buffer.text(), "[Cat:Cat] [cat:cat]\n[cat:cat]/path\n世界");
        assert!(buffer.dirty());
        buffer.key("u");
        assert_eq!(buffer.lines, original);
        assert!(!buffer.dirty());
        buffer.key("ctrl-r");
        assert_eq!(buffer.text(), "[Cat:Cat] [cat:cat]\n[cat:cat]/path\n世界");
        buffer.key("u");
        buffer.substitute(r"/cat\/path/one\rtwo/").unwrap();
        assert_eq!(buffer.text(), "Cat cat\none\ntwo\n世界");
        assert_eq!(buffer.lines[1].id, original[1].id);
        assert_eq!(buffer.lines[3].id, original[2].id);
        assert!(!original.iter().any(|line| line.id == buffer.lines[2].id));
    }

    #[test]
    fn substitution_errors_are_atomic_and_default_is_first_match_per_line() {
        let mut buffer = Buffer::new("a a\na a");
        for command in ["/[/x/", "/a/x/c", "/a/\\9/", "/missing/x/g", "/a"] {
            assert!(buffer.substitute(command).is_err(), "{command}");
            assert_eq!(buffer.text(), "a a\na a");
            assert!(!buffer.dirty());
        }
        assert_eq!(buffer.substitute("#a#\\&#").unwrap(), 2);
        assert_eq!(buffer.text(), "& a\n& a");
        buffer.key("u");
        assert_eq!(buffer.text(), "a a\na a");
    }

    #[test]
    fn half_page_counts_boundaries_and_cancelled_z_prefix() {
        let mut buffer = Buffer::new(
            &(0..40)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        buffer.page_rows = 20;
        keys(&mut buffer, &["ctrl-d"]);
        assert_eq!(buffer.row, 10);
        keys(&mut buffer, &["3", "ctrl-u"]);
        assert_eq!(buffer.row, 7);
        keys(&mut buffer, &["ctrl-u"]);
        assert_eq!(buffer.row, 4);
        keys(&mut buffer, &["ctrl-u", "ctrl-u"]);
        assert_eq!(buffer.row, 0);
        buffer.take_viewport_motion();
        keys(&mut buffer, &["z", "escape"]);
        buffer.key("t");
        assert!(buffer.take_viewport_motion().is_none());
        keys(&mut buffer, &["3", "0", "z", "t"]);
        assert_eq!(buffer.row, 29);
        assert!(matches!(
            buffer.take_viewport_motion(),
            Some(ViewportMotion::Top)
        ));
    }

    #[test]
    fn graphemes_are_atomic_for_motion_deletion_and_insert_undo() {
        let mut buffer = Buffer::new("e\u{301}👩‍💻界");
        keys(&mut buffer, &["l", "x"]);
        assert_eq!(buffer.text(), "e\u{301}界");
        assert_eq!(buffer.col, "e\u{301}".len());
        keys(
            &mut buffer,
            &["u", "a", "backspace", "enter", "🦀", "escape"],
        );
        assert_eq!(buffer.text(), "e\u{301}\n🦀界");
        buffer.undo();
        assert_eq!(buffer.text(), "e\u{301}👩‍💻界");
        buffer.redo();
        assert_eq!(buffer.text(), "e\u{301}\n🦀界");
    }

    #[test]
    fn line_identity_survives_edits_splits_and_undo_but_not_copies() {
        let mut buffer = Buffer::new("one\ntwo\nthree");
        let ids: Vec<_> = buffer.lines.iter().map(|line| line.id).collect();
        keys(&mut buffer, &["i", "X", "enter", "escape"]);
        assert_eq!(buffer.lines[0].id, ids[0]);
        assert_eq!(buffer.lines[2].id, ids[1]);
        let split_id = buffer.lines[1].id;
        buffer.undo();
        assert_eq!(
            buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
            ids
        );
        keys(&mut buffer, &["y", "y", "p"]);
        assert_eq!(buffer.text(), "one\none\ntwo\nthree");
        assert!(!ids.contains(&buffer.lines[1].id));
        assert_ne!(buffer.lines[1].id, split_id);
        keys(&mut buffer, &["d", "d"]);
        assert_eq!(
            buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
            ids
        );
    }

    #[test]
    fn insert_session_is_one_undo_and_saved_state_is_textual() {
        let mut buffer = Buffer::new("abc");
        keys(&mut buffer, &["A", "x", "y", "left", "z", "escape"]);
        assert_eq!(buffer.text(), "abcxzy");
        buffer.mark_saved();
        keys(&mut buffer, &["0", "l"]);
        assert!(!buffer.dirty());
        buffer.undo();
        assert_eq!(buffer.text(), "abc");
        assert!(buffer.dirty());
        keys(&mut buffer, &["ctrl-r"]);
        assert!(!buffer.dirty());
        buffer.undo();
        keys(&mut buffer, &["x", "ctrl-r"]);
        assert_eq!(buffer.text(), "bc");
    }

    #[test]
    fn visual_is_inclusive_and_reversed_multiline_selection_is_safe() {
        let mut buffer = Buffer::new("αβ\n👩‍💻z");
        keys(&mut buffer, &["G", "l", "v", "g", "g"]);
        assert_eq!(buffer.selected_range(0), Some(0.."αβ".len()));
        assert_eq!(buffer.selected_range(1), Some(0.."👩‍💻z".len()));
        keys(&mut buffer, &["d"]);
        assert_eq!(buffer.text(), "");
        buffer.undo();
        assert_eq!(buffer.text(), "αβ\n👩‍💻z");
        keys(&mut buffer, &["g", "g", "v", "c", "Q", "escape"]);
        assert_eq!(buffer.text(), "Qβ\n👩‍💻z");
    }

    #[test]
    fn counts_words_and_empty_boundaries() {
        let mut buffer = Buffer::new("one two, three\n\nlast");
        keys(&mut buffer, &["2", "w"]);
        assert_eq!(buffer.col, 7);
        keys(&mut buffer, &["b"]);
        assert_eq!(buffer.col, 4);
        keys(&mut buffer, &["c", "w", "X", "escape"]);
        assert_eq!(buffer.text(), "one X, three\n\nlast");
        buffer.undo();
        keys(&mut buffer, &["0", "2", "d", "w"]);
        assert_eq!(buffer.text(), ", three\n\nlast");
        keys(
            &mut buffer,
            &["9", "9", "9", "G", "9", "9", "9", "l", "e", "w", "j"],
        );
        assert_eq!((buffer.row, buffer.col), (2, 3));
        keys(&mut buffer, &["9", "9", "9", "d", "d"]);
        assert_eq!(buffer.text(), ", three\n");
        keys(
            &mut buffer,
            &["g", "g", "9", "9", "9", "d", "d", "b", "e", "h", "k", "x"],
        );
        assert_eq!(buffer.text(), "");
        assert_eq!((buffer.row, buffer.col), (0, 0));
    }

    #[test]
    fn set_text_preserves_shifted_suffix_identities() {
        let mut buffer = Buffer::new("a\nb\nc");
        let b = buffer.lines[1].id;
        let c = buffer.lines[2].id;
        buffer.set_text("a\nnew\nb\nc");
        assert_eq!(buffer.lines[2].id, b);
        assert_eq!(buffer.lines[3].id, c);
        assert!(!buffer.dirty());
    }

    #[test]
    fn revision_invalidates_content_not_cursor_or_saved_state() {
        let mut buffer = Buffer::new("abc");
        let initial = buffer.revision();
        keys(&mut buffer, &["l", "v", "escape", "i", "escape"]);
        buffer.mark_saved();
        assert_eq!(buffer.revision(), initial);
        keys(&mut buffer, &["i", "X"]);
        let inserted = buffer.revision();
        assert!(inserted > initial);
        keys(&mut buffer, &["escape"]);
        assert_eq!(buffer.revision(), inserted);
        buffer.undo();
        let undone = buffer.revision();
        assert!(undone > inserted);
        buffer.redo();
        assert!(buffer.revision() > undone);
        let redone = buffer.revision();
        buffer.set_text("Xabc");
        assert_eq!(buffer.revision(), redone);
        buffer.set_text("replacement");
        assert!(buffer.revision() > redone);
    }

    #[test]
    fn changing_single_character_words_includes_the_character() {
        let mut buffer = Buffer::new("a b c");
        keys(&mut buffer, &["c", "w", "Z", "escape"]);
        assert_eq!(buffer.text(), "Z b c");
        buffer.undo();
        keys(&mut buffer, &["2", "c", "w", "Q", "escape"]);
        assert_eq!(buffer.text(), "Q c");
    }

    #[test]
    fn vertical_motion_preserves_grapheme_column_across_short_lines() {
        let mut buffer = Buffer::new("αβγ\nx\n👩‍💻界z");
        keys(&mut buffer, &["2", "l", "j"]);
        assert_eq!((buffer.row, buffer.col), (1, 0));
        keys(&mut buffer, &["j"]);
        assert_eq!((buffer.row, buffer.col), (2, "👩‍💻界".len()));
        keys(&mut buffer, &["i", "up", "up"]);
        assert_eq!((buffer.row, buffer.col), (0, "αβ".len()));
        buffer.col = 1;
        keys(&mut buffer, &["delete"]);
        assert_eq!(buffer.lines[0].text, "βγ");
    }

    #[test]
    fn undo_stack_is_bounded() {
        let mut buffer = Buffer::new("start");
        for _ in 0..150 {
            keys(&mut buffer, &["o", "x", "escape"]);
        }
        assert_eq!(buffer.undo_stack.len(), MAX_UNDO_DEPTH);
        for _ in 0..MAX_UNDO_DEPTH {
            buffer.undo();
        }
        assert_eq!(buffer.undo_stack.len(), 0);
        assert_eq!(buffer.redo_stack.len(), MAX_UNDO_DEPTH);
    }
}

#[cfg(test)]
mod mouse_tests {
    use super::*;

    #[test]
    fn mouse_word_selection_preserves_unicode_graphemes_and_undo() {
        let mut buffer = Buffer::new("hello café 👩‍💻 world");
        buffer.key("i");
        buffer.col = "hello ca".len();
        buffer.select_word_with_mouse();
        assert_eq!(buffer.selected_range(0), Some(6..11));
        buffer.key("d");
        assert_eq!(buffer.text(), "hello  👩‍💻 world");
        buffer.key("u");
        assert_eq!(buffer.text(), "hello café 👩‍💻 world");
        buffer.select_with_mouse((0, 12), (0, 13));
        assert_eq!(buffer.selected_range(0), Some(12..23));
    }
}
