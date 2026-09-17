use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

mod clipboard;
mod editing;
mod formatting;
mod history;
mod keys;
mod motions;
mod operators;
mod selection;
mod substitution;

#[cfg(test)]
mod tests;

pub use formatting::InlineFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
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

    pub(crate) fn from_recovery(text: &str, saved_text: &str) -> Self {
        let mut buffer = Self::new(text);
        buffer.saved_text = normalize_newlines(saved_text).into_owned();
        buffer
    }

    pub(crate) fn saved_text(&self) -> &str {
        &self.saved_text
    }

    /// Recovery always resumes in Normal mode, with a valid grapheme position.
    pub(crate) fn restore_cursor(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
        self.clamp();
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

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("line identity space exhausted");
        id
    }

    fn bump_revision(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("buffer revision space exhausted");
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
