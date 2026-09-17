use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{Buffer, Mode, next_boundary, ordered, previous_boundary};

/// Markdown source delimiters applied to the current visual selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineFormat {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
}

impl InlineFormat {
    fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::Bold => ("**", "**"),
            Self::Italic => ("*", "*"),
            Self::Underline => ("<u>", "</u>"),
            Self::Strikethrough => ("~~", "~~"),
            Self::Code => ("`", "`"),
        }
    }
}

struct FormatEdit {
    range: Range<usize>,
    content: Range<usize>,
    replacement: String,
    prefix: usize,
}

impl FormatEdit {
    fn map_boundary(&self, col: usize) -> usize {
        if col < self.range.start {
            col
        } else if col > self.range.end {
            col - self.range.len() + self.replacement.len()
        } else {
            self.range.start
                + self.prefix
                + col
                    .saturating_sub(self.content.start)
                    .min(self.content.len())
        }
    }
}

impl Buffer {
    /// Toggle source formatting on each nonblank selected physical line.
    /// The content stays selected, with the original direction and visual mode;
    /// all changed lines form one undo transaction and retain their identities.
    /// Markdown/code-block eligibility is checked by the editor caller.
    pub fn toggle_inline_format(&mut self, format: InlineFormat) -> bool {
        if !matches!(self.mode, Mode::Visual | Mode::VisualLine) {
            return false;
        }
        let Some(anchor) = self.visual_anchor else {
            return false;
        };
        let reversed = anchor > self.position();
        let (mut start, mut end) = ordered(anchor, self.position());
        end.col = next_boundary(&self.lines[end.row].text, end.col);
        let mut before = None;
        for row in start.row..=end.row {
            let Some(selection) = self.selected_range(row) else {
                continue;
            };
            let Some(edit) = format_edit(&self.lines[row].text, selection, format) else {
                continue;
            };
            if before.is_none() {
                before = Some(self.snapshot());
            }
            if row == start.row {
                start.col = edit.map_boundary(start.col);
            }
            if row == end.row {
                end.col = edit.map_boundary(end.col);
            }
            self.lines[row]
                .text
                .replace_range(edit.range, &edit.replacement);
        }
        let Some(before) = before else {
            return false;
        };
        end.col = previous_boundary(&self.lines[end.row].text, end.col);
        let (anchor, head) = if reversed { (end, start) } else { (start, end) };
        self.visual_anchor = Some(anchor);
        self.row = head.row;
        self.col = head.col;
        self.pending = None;
        self.count = None;
        self.preferred_column = None;
        self.clamp();
        self.bump_revision();
        self.record(before);
        true
    }
}

fn format_edit(text: &str, selection: Range<usize>, format: InlineFormat) -> Option<FormatEdit> {
    let selected = &text[selection.clone()];
    let mut graphemes = selected
        .grapheme_indices(true)
        .filter(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace));
    let (first, first_grapheme) = graphemes.next()?;
    let (last, last_grapheme) = graphemes.next_back().unwrap_or((first, first_grapheme));
    let content = selection.start + first..selection.start + last + last_grapheme.len();

    if let Some((range, inner)) = existing_format(text, content.clone(), format) {
        return Some(FormatEdit {
            replacement: text[inner.clone()].to_owned(),
            range,
            content: inner,
            prefix: 0,
        });
    }

    let value = &text[content.clone()];
    let delimiter;
    let (open, close, padded) = if format == InlineFormat::Code {
        let longest = value.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
        delimiter = "`".repeat(longest + 1);
        (
            delimiter.as_str(),
            delimiter.as_str(),
            value.starts_with('`') || value.ends_with('`'),
        )
    } else {
        let (open, close) = format.delimiters();
        (open, close, false)
    };
    let padding = usize::from(padded);
    let mut replacement =
        String::with_capacity(open.len() + content.len() + close.len() + padding * 2);
    replacement.push_str(open);
    if padded {
        replacement.push(' ');
    }
    replacement.push_str(value);
    if padded {
        replacement.push(' ');
    }
    replacement.push_str(close);
    Some(FormatEdit {
        range: content.clone(),
        content,
        replacement,
        prefix: open.len() + padding,
    })
}

fn existing_format(
    text: &str,
    content: Range<usize>,
    format: InlineFormat,
) -> Option<(Range<usize>, Range<usize>)> {
    if format == InlineFormat::Code {
        return existing_code(text, content);
    }
    let (open, close) = format.delimiters();
    // Prefer a wrapper around the selection: this also reverses formatting
    // applied to a selection that already contains another emphasis style.
    if content.start >= open.len() && content.end + close.len() <= text.len() {
        let outer = content.start - open.len()..content.end + close.len();
        if wrapper_matches(text, &outer, &content, open, close, format) {
            return Some((outer, content));
        }
    }
    if content.len() > open.len() + close.len() {
        let inner = content.start + open.len()..content.end - close.len();
        if wrapper_matches(text, &content, &inner, open, close, format) {
            return Some((content, inner));
        }
    }
    None
}

fn wrapper_matches(
    text: &str,
    outer: &Range<usize>,
    inner: &Range<usize>,
    open: &str,
    close: &str,
    format: InlineFormat,
) -> bool {
    if text.get(outer.start..inner.start) != Some(open)
        || text.get(inner.end..outer.end) != Some(close)
        || escaped(text, outer.start)
        || text
            .get(inner.clone())
            .is_none_or(|value| value.trim().is_empty())
    {
        return false;
    }
    if format == InlineFormat::Italic {
        // A single star from a bold delimiter is not an italic wrapper.
        let left = text[..inner.start]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'*')
            .count()
            + text[inner.start..]
                .bytes()
                .take_while(|byte| *byte == b'*')
                .count();
        let right = text[inner.end..]
            .bytes()
            .take_while(|byte| *byte == b'*')
            .count()
            + text[..inner.end]
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'*')
                .count();
        return left % 2 == 1 && right % 2 == 1;
    }
    true
}

fn escaped(text: &str, col: usize) -> bool {
    text[..col]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

fn existing_code(text: &str, content: Range<usize>) -> Option<(Range<usize>, Range<usize>)> {
    // Generated padding is outside the retained content selection.
    for padding in [0, 1] {
        if content.start < padding || content.end + padding > text.len() {
            continue;
        }
        let left = content.start - padding;
        let right = content.end + padding;
        if padding == 1
            && (text.get(left..content.start) != Some(" ")
                || text.get(content.end..right) != Some(" "))
        {
            continue;
        }
        let opening = text[..left]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'`')
            .count();
        let closing = text[right..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        if opening > 0
            && opening == closing
            && !escaped(text, left - opening)
            && !has_tick_run(&text[content.clone()], opening)
        {
            return Some((left - opening..right + closing, content));
        }
    }
    let value = &text[content.clone()];
    let opening = value.bytes().take_while(|byte| *byte == b'`').count();
    let closing = value.bytes().rev().take_while(|byte| *byte == b'`').count();
    if opening == 0
        || opening != closing
        || value.len() <= opening + closing
        || escaped(text, content.start)
    {
        return None;
    }
    let mut inner = content.start + opening..content.end - closing;
    if has_tick_run(&text[inner.clone()], opening) {
        return None;
    }
    let value = &text[inner.clone()];
    if value.starts_with(' ') && value.ends_with(' ') && !value.chars().all(|ch| ch == ' ') {
        inner.start += 1;
        inner.end -= 1;
    }
    if text[inner.clone()].trim().is_empty() {
        return None;
    }
    Some((content, inner))
}

fn has_tick_run(value: &str, length: usize) -> bool {
    value.split(|ch| ch != '`').any(|run| run.len() == length)
}
