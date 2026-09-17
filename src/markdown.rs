use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color, FontStyle as SyntaxFontStyle, Theme as SyntaxTheme, ThemeSet},
    parsing::SyntaxSet,
};

use crate::config::Theme;

// Share syntect's language rules and base styles across projections. Map Ocean
// foreground roles to the active palette only when emitting a highlighted span.
static HIGHLIGHTER: LazyLock<(SyntaxSet, SyntaxTheme)> = LazyLock::new(|| {
    let mut themes = ThemeSet::load_defaults();
    let base = themes.themes.remove("base16-ocean.dark").unwrap();
    (SyntaxSet::load_defaults_newlines(), base)
});

fn syntax_color(color: Color, palette: &[u32; 10]) -> u32 {
    let rgb = (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b);
    let index = match rgb {
        0x65737e => 1, // comments
        0xbf616a => 2, // variables, tags
        0xd08770 => 3, // constants, numbers
        0xebcb8b => 4, // types
        0xa3be8c => 5, // strings
        0x96b5b4 => 6, // regular expressions, escapes
        0x8fa1b3 => 7, // functions
        0xb48ead => 8, // keywords
        0xab7967 => 9, // embedded markup
        _ => 0,        // neutral text
    };
    palette[index]
}

/// Append one code-block text chunk as syntax-highlighted spans. Pulldown
/// delivers the fenced body through plain Text events, so the highlighter is
/// created at the opening fence and fed line by line until the closer.
fn append_highlighted(
    lines: &mut [RenderedLine],
    row: usize,
    text: &str,
    highlighter: &mut HighlightLines,
    palette: &[u32; 10],
) {
    let (syntaxes, _) = &*HIGHLIGHTER;
    for (offset, part) in text.split_inclusive('\n').enumerate() {
        let Some(target) = lines.get_mut(row + offset) else {
            break;
        };
        let regions = highlighter
            .highlight_line(part, syntaxes)
            .unwrap_or_default();
        for (style, fragment) in regions {
            // Keep the newline for the highlighter's line-based grammars, but
            // store spans without it: a physical row never contains a break.
            let text = fragment.strip_suffix('\n').unwrap_or(fragment);
            let text = text.strip_suffix('\r').unwrap_or(text);
            if text.is_empty() {
                continue;
            }
            target.spans.push(Span {
                text: text.to_owned(),
                bold: style.font_style.contains(SyntaxFontStyle::BOLD),
                italic: style.font_style.contains(SyntaxFontStyle::ITALIC),
                code: true,
                strike: false,
                underline: false,
                link: None,
                color: Some(format!("#{:06x}", syntax_color(style.foreground, palette))),
            });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    Code,
    Rule,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub underline: bool,
    pub link: Option<String>,
    /// Highlighted foreground color as `#rrggbb`, when a fence names a known
    /// language. `None` keeps the renderer's default styling.
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub start: usize,
    pub end: usize,
    pub header: bool,
    pub separator: bool,
    pub alignments: Arc<[Alignment]>,
    pub cells: Vec<Vec<Span>>,
}

/// Width of an inline image preview. `None` uses the renderer's default share
/// of the pane width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageWidth {
    /// Fraction of the editor pane width, e.g. `0.6` for 60%.
    Fraction(f32),
    /// Absolute width in points.
    Points(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewImage {
    pub url: String,
    pub width: Option<ImageWidth>,
}

/// Parse an image title as a width override: `"40%"` is a pane-width share,
/// `"320px"` an absolute width. Anything else is not a size.
fn parse_image_width(title: &str) -> Option<ImageWidth> {
    let title = title.trim();
    if let Some(percent) = title.strip_suffix('%') {
        let percent: f32 = percent.trim().parse().ok()?;
        return (percent.is_finite() && percent > 0.0)
            .then_some(ImageWidth::Fraction(percent / 100.0));
    }
    if let Some(points) = title.strip_suffix("px") {
        let points: f32 = points.trim().parse().ok()?;
        return (points.is_finite() && points > 0.0).then_some(ImageWidth::Points(points));
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedLine {
    pub source_row: usize,
    /// Rendered UTF-8 boundaries paired with byte columns in the physical source row.
    pub source_offsets: Vec<(usize, usize)>,
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    pub images: Vec<PreviewImage>,
    pub table: Option<TableRow>,
    /// Byte offset of the task checkbox in the physical source line.
    pub task_marker: Option<usize>,
    /// Source and display byte offsets where a list item's body begins.
    pub list_content: Option<(usize, usize)>,
}

impl RenderedLine {
    pub fn source_column(&self, display_column: usize) -> usize {
        self.source_offsets
            .iter()
            .rev()
            .find(|(display, _)| *display <= display_column)
            .map_or(0, |(_, source)| *source)
    }
}

#[derive(Default)]
struct Style {
    strong: usize,
    emphasis: usize,
    strike: usize,
    underline: usize,
    code: usize,
    header: usize,
    links: Vec<String>,
}

impl Style {
    fn append(&self, line: &mut RenderedLine, text: &str) {
        self.append_spans(&mut line.spans, text);
    }

    fn append_spans(&self, spans: &mut Vec<Span>, text: &str) {
        if text.is_empty() {
            return;
        }
        let bold = self.strong > 0 || self.header > 0;
        let italic = self.emphasis > 0;
        let code = self.code > 0;
        let strike = self.strike > 0;
        let underline = self.underline > 0;
        let link = self.links.last();
        if let Some(last) = spans.last_mut()
            && last.bold == bold
            && last.italic == italic
            && last.code == code
            && last.strike == strike
            && last.underline == underline
            && last.link.as_ref() == link
            && last.color.is_none()
        {
            last.text.push_str(text);
        } else {
            spans.push(Span {
                text: text.to_owned(),
                bold,
                italic,
                code,
                strike,
                underline,
                link: link.cloned(),
                color: None,
            });
        }
    }
}

#[derive(Clone, Copy)]
enum Container {
    Quote,
    Item(usize),
}

/// Recognize only balanced, exact underline tags within one Markdown block.
/// Code events and all other HTML stay literal; this is not an HTML renderer.
fn underline_events(source: &str, options: Options) -> Vec<(Event<'_>, Range<usize>, bool)> {
    let mut events = Vec::new();
    let mut replaced_html = false;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        if replaced_html {
            if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                replaced_html = false;
            }
            continue;
        }
        if matches!(event, Event::Start(Tag::HtmlBlock))
            && source[range.clone()].trim_start().starts_with("<u>")
        {
            // A standalone <u> at line start is classified as block HTML.
            // A plain prefix forces inline parsing without changing the actual
            // source offsets or enabling arbitrary HTML rendering.
            let inline = format!("x{}", &source[range.clone()]);
            for (mut event, local) in Parser::new_ext(&inline, options).into_offset_iter() {
                if let Event::Text(text) = &mut event
                    && local.start == 0
                {
                    *text = text.strip_prefix('x').unwrap_or(text).to_owned().into();
                }
                if local.end <= 1 {
                    continue;
                }
                let start = range.start + local.start.saturating_sub(1);
                let end = range.start + local.end - 1;
                events.push((event.into_static(), start..end, false));
            }
            replaced_html = true;
        } else {
            events.push((event, range, false));
        }
    }
    let mut openings = Vec::new();
    for index in 0..events.len() {
        match &events[index].0 {
            Event::InlineHtml(tag) if tag.as_ref() == "<u>" => openings.push(index),
            Event::InlineHtml(tag) if tag.as_ref() == "</u>" => {
                if let Some(opening) = openings.pop() {
                    events[opening].2 = true;
                    events[index].2 = true;
                }
            }
            Event::Start(
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::Item
                | Tag::TableCell,
            )
            | Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::Item
                | TagEnd::TableCell,
            )
            | Event::Rule => openings.clear(),
            _ => {}
        }
    }
    events
}

/// Project a single full-document parse onto physical source lines. Delimiter-only
/// and trailing empty lines deliberately remain present, so a click always refers
/// to the same row in the editable buffer. Offsets are UTF-8 byte offsets.
/// `theme` supplies syntax colors suited to the actual editor background.
pub fn project(source: &str, theme: &Theme) -> Vec<RenderedLine> {
    let mut starts = vec![0];
    starts.extend(
        source
            .bytes()
            .enumerate()
            .filter_map(|(i, byte)| (byte == b'\n').then_some(i + 1)),
    );
    let mut lines: Vec<_> = (0..starts.len())
        .map(|source_row| RenderedLine {
            source_row,
            source_offsets: Vec::new(),
            kind: BlockKind::Paragraph,
            spans: Vec::new(),
            images: Vec::new(),
            table: None,
            task_marker: None,
            list_content: None,
        })
        .collect();
    // Difference arrays classify nested containers in one final linear pass,
    // rather than revisiting all their rows for every level of nesting.
    let mut quotes = vec![0isize; starts.len() + 1];
    let mut lists = vec![0isize; starts.len() + 1];
    let mut style = Style::default();
    let mut containers = Vec::new();
    let mut ordinals: Vec<Option<u64>> = Vec::new();
    let mut table_row = None;
    let mut cell = None;
    let mut cell_index = 0usize;
    let mut code_highlighter: Option<HighlightLines<'static>> = None;
    let palette = theme.syntax_palette();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    // Most documents need neither lookahead nor an event buffer.
    let underlined = source
        .contains("<u>")
        .then(|| underline_events(source, options));
    let ordinary = underlined
        .is_none()
        .then(|| Parser::new_ext(source, options).into_offset_iter())
        .into_iter()
        .flatten()
        .map(|(event, range)| (event, range, false));
    for (event, range, underline_tag) in ordinary.chain(underlined.into_iter().flatten()) {
        let row = source_row(&starts, range.start);
        let mapped_rows = match &event {
            Event::Text(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => covered_rows(&starts, &range),
            Event::Start(Tag::Item) | Event::TaskListMarker(_) | Event::FootnoteReference(_) => {
                row..row + 1
            }
            _ => row..row,
        };
        // Appends may extend the last style span or create new spans. Capture only
        // that boundary; never rescan or copy the already-rendered paragraph.
        let previous: Vec<_> = mapped_rows
            .clone()
            .map(|row| {
                let line = &lines[row];
                (
                    line.spans.len(),
                    line.spans.last().map_or(0, |span| span.text.len()),
                )
            })
            .collect();
        let source_range = range.clone();
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    set_kind(&mut lines, &starts, &range, BlockKind::Heading(level as u8));
                }
                Tag::BlockQuote(_) => {
                    cover(&mut quotes, &starts, &range);
                    containers.push(Container::Quote);
                }
                Tag::CodeBlock(kind) => {
                    set_kind(&mut lines, &starts, &range, BlockKind::Code);
                    style.code += 1;
                    // The fence info's first word is the language token. Unknown
                    // tokens highlight as plain text, preserving code styling.
                    let token = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or(""),
                        CodeBlockKind::Indented => "",
                    };
                    let (syntaxes, base) = &*HIGHLIGHTER;
                    let syntax = syntaxes
                        .find_syntax_by_token(token)
                        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
                    code_highlighter = Some(HighlightLines::new(syntax, base));
                }
                Tag::List(first) => {
                    cover(&mut lists, &starts, &range);
                    ordinals.push(first);
                }
                Tag::Item => {
                    containers.push(Container::Item(item_indent(&source[range.clone()])));
                    for _ in 1..ordinals.len() {
                        style.append(&mut lines[row], "  ");
                    }
                    if let Some(Some(number)) = ordinals.last_mut() {
                        style.append(&mut lines[row], &format!("{number}. "));
                        *number = number.saturating_add(1);
                    } else {
                        style.append(&mut lines[row], "• ");
                    }
                    let raw = &source[range.clone()];
                    let marker_end = raw.find([' ', '\t']).unwrap_or(raw.len());
                    let body = marker_end + raw[marker_end..].len()
                        - raw[marker_end..].trim_start_matches([' ', '\t']).len();
                    lines[row].list_content = Some((
                        range.start - starts[row] + body,
                        lines[row].spans.iter().map(|span| span.text.len()).sum(),
                    ));
                }
                Tag::Emphasis => style.emphasis += 1,
                Tag::Strong => style.strong += 1,
                Tag::Strikethrough => style.strike += 1,
                Tag::Image {
                    dest_url, title, ..
                } => {
                    lines[row].images.push(PreviewImage {
                        url: dest_url.to_string(),
                        width: parse_image_width(&title),
                    });
                    style.links.push(dest_url.into_string());
                }
                Tag::Link { dest_url, .. } => {
                    style.links.push(dest_url.into_string());
                }
                Tag::Table(alignments) => {
                    let rows = covered_rows(&starts, &range);
                    let alignments: Arc<[Alignment]> = alignments.into();
                    for line in &mut lines[rows.clone()] {
                        line.table = Some(TableRow {
                            start: rows.start,
                            end: rows.end,
                            header: false,
                            separator: true,
                            alignments: Arc::clone(&alignments),
                            cells: Vec::new(),
                        });
                    }
                }
                Tag::TableHead | Tag::TableRow => {
                    let header = matches!(tag, Tag::TableHead);
                    let table = lines[row].table.as_mut().expect("row belongs to a table");
                    table.header = header;
                    table.separator = false;
                    table.cells.resize_with(table.alignments.len(), Vec::new);
                    style.header += usize::from(header);
                    table_row = Some(row);
                    cell_index = 0;
                }
                Tag::TableCell => {
                    // Synthesized missing cells may point at the next physical
                    // line. Their enclosing row, not that offset, owns them.
                    cell = Some((table_row.expect("cell belongs to a table row"), cell_index));
                    cell_index += 1;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::CodeBlock => {
                    style.code -= 1;
                    code_highlighter = None;
                }
                TagEnd::BlockQuote(_) | TagEnd::Item => {
                    containers.pop();
                }
                TagEnd::List(_) => {
                    ordinals.pop();
                }
                TagEnd::Emphasis => style.emphasis -= 1,
                TagEnd::Strong => style.strong -= 1,
                TagEnd::Strikethrough => style.strike -= 1,
                TagEnd::Link | TagEnd::Image => {
                    style.links.pop();
                }
                TagEnd::TableCell => cell = None,
                TagEnd::TableHead => {
                    style.header -= 1;
                    table_row = None;
                }
                TagEnd::TableRow => table_row = None,
                _ => {}
            },
            Event::InlineHtml(tag) if underline_tag => {
                if tag.as_ref() == "<u>" {
                    style.underline += 1;
                } else {
                    style.underline -= 1;
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                if let Some(highlighter) = code_highlighter.as_mut() {
                    append_highlighted(&mut lines, row, &text, highlighter, &palette);
                } else {
                    append_text(&mut lines, row, &text, &style, cell);
                }
            }
            Event::Code(text) => {
                style.code += 1;
                let raw = &source[range];
                if cell.is_some() {
                    append_text(&mut lines, row, &text, &style, cell);
                } else if raw.contains('\n') {
                    append_multiline_code(&mut lines, row, raw, &containers, &style);
                } else {
                    style.append(&mut lines[row], &text);
                }
                style.code -= 1;
            }
            Event::Rule => lines[row].kind = BlockKind::Rule,
            Event::TaskListMarker(checked) => {
                lines[row].task_marker = Some(range.start - starts[row]);
                style.append(&mut lines[row], if checked { "[x] " } else { "[ ] " });
                if let Some((body_source, display)) = lines[row].list_content.as_mut() {
                    *body_source = range.end - starts[row];
                    while matches!(
                        source.as_bytes().get(starts[row] + *body_source),
                        Some(b' ' | b'\t')
                    ) {
                        *body_source += 1;
                    }
                    *display += 4;
                }
            }
            // Breaks already have distinct physical rows. Adding a space here
            // would change the visual line ending and can leak inline styling.
            Event::SoftBreak | Event::HardBreak => {}
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                append_text(&mut lines, row, &text, &style, cell);
            }
            Event::FootnoteReference(label) => {
                append_text(&mut lines, row, &label, &style, cell);
            }
        }
        for (row, (span_count, last_len)) in mapped_rows.zip(previous) {
            let line = &lines[row];
            if line.spans.len() == span_count
                && line.spans.last().map_or(0, |span| span.text.len()) == last_len
            {
                continue;
            }
            let previous = line
                .source_offsets
                .last()
                .map_or(0, |(display, _)| *display);
            let mut added = String::new();
            if span_count > 0 {
                added.push_str(&line.spans[span_count - 1].text[last_len..]);
            }
            for span in &line.spans[span_count..] {
                added.push_str(&span.text);
            }
            let row_start = starts[row];
            let row_end = starts.get(row + 1).map_or(source.len(), |start| start - 1);
            let start = source_range.start.max(row_start).min(row_end);
            let end = source_range.end.min(row_end).max(start);
            let literal = source[start..end].find(&added);
            let offsets = &mut lines[row].source_offsets;
            // The next visible run owns shared boundaries, skipping hidden delimiters.
            if offsets
                .last()
                .is_some_and(|(display, _)| *display == previous)
            {
                offsets.pop();
            }
            for (byte, _) in added.char_indices() {
                offsets.push((
                    previous + byte,
                    literal.map_or(start, |offset| start + offset + byte) - row_start,
                ));
            }
            offsets.push((
                previous + added.len(),
                literal.map_or(end, |offset| start + offset + added.len()) - row_start,
            ));
        }
    }

    let (mut quote_depth, mut list_depth) = (0, 0);
    for (row, line) in lines.iter_mut().enumerate() {
        quote_depth += quotes[row];
        list_depth += lists[row];
        if line.kind == BlockKind::Paragraph {
            line.kind = if list_depth > 0 {
                BlockKind::List
            } else if quote_depth > 0 {
                BlockKind::Quote
            } else {
                BlockKind::Paragraph
            };
        }
    }
    lines
}

fn source_row(starts: &[usize], offset: usize) -> usize {
    starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1)
}

fn covered_rows(starts: &[usize], range: &Range<usize>) -> Range<usize> {
    let first = source_row(starts, range.start);
    let last = source_row(starts, range.end.saturating_sub(1).max(range.start));
    first..last + 1
}

fn cover(depth: &mut [isize], starts: &[usize], range: &Range<usize>) {
    let rows = covered_rows(starts, range);
    depth[rows.start] += 1;
    depth[rows.end] -= 1;
}

fn set_kind(lines: &mut [RenderedLine], starts: &[usize], range: &Range<usize>, kind: BlockKind) {
    for line in &mut lines[covered_rows(starts, range)] {
        line.kind = kind;
    }
}

fn append_text(
    lines: &mut [RenderedLine],
    row: usize,
    text: &str,
    style: &Style,
    cell: Option<(usize, usize)>,
) {
    if let Some((row, column)) = cell {
        let table = lines[row].table.as_mut().expect("cell belongs to a table");
        style.append_spans(&mut table.cells[column], text);
        return;
    }
    // Block-code and HTML events can contain several lines. Other text events
    // (including decoded entities) normally occupy just one source line.
    for (offset, part) in text.split('\n').enumerate() {
        if let Some(line) = lines.get_mut(row + offset) {
            style.append(line, part.strip_suffix('\r').unwrap_or(part));
        }
    }
}

fn item_indent(raw: &str) -> usize {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while bytes.get(i) == Some(&b' ') {
        i += 1;
    }
    let marker = i;
    if matches!(bytes.get(i), Some(b'-' | b'+' | b'*')) {
        i += 1;
    } else {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if matches!(bytes.get(i), Some(b'.' | b')')) {
            i += 1;
        }
    }
    let marker_end = i;
    let mut column = i;
    while let Some(byte) = bytes.get(i) {
        match byte {
            b' ' => column += 1,
            b'\t' => column += 4 - column % 4,
            _ => break,
        }
        i += 1;
    }
    let padding = column - marker_end;
    if marker_end == marker {
        0
    } else if padding == 0 || padding > 4 {
        marker_end + 1
    } else {
        column
    }
}

fn strip_containers<'a>(mut text: &'a str, containers: &[Container]) -> &'a str {
    for container in containers {
        match container {
            Container::Quote => {
                let spaces = text.bytes().take_while(|byte| *byte == b' ').count().min(3);
                if let Some(rest) = text[spaces..].strip_prefix('>') {
                    text = rest
                        .strip_prefix(' ')
                        .or_else(|| rest.strip_prefix('\t'))
                        .unwrap_or(rest);
                }
            }
            Container::Item(indent) => {
                let mut bytes = 0;
                let mut column = 0;
                for byte in text.bytes() {
                    if column >= *indent {
                        break;
                    }
                    match byte {
                        b' ' => column += 1,
                        b'\t' => column += 4 - column % 4,
                        _ => break,
                    }
                    bytes += 1;
                }
                text = &text[bytes..];
            }
        }
    }
    text
}

fn append_multiline_code(
    lines: &mut [RenderedLine],
    row: usize,
    raw: &str,
    containers: &[Container],
    style: &Style,
) {
    // CommonMark normalizes newlines in code spans to spaces. Using that flat
    // event would move all text onto the opening row. Keep the physical breaks,
    // removing matching backtick runs and continuation container prefixes only.
    let delimiter = raw.bytes().take_while(|byte| *byte == b'`').count();
    let body = &raw[delimiter..raw.len() - delimiter];
    let trim = body.starts_with([' ', '\r', '\n'])
        && body.ends_with([' ', '\r', '\n'])
        && body
            .bytes()
            .any(|byte| !matches!(byte, b' ' | b'\r' | b'\n'));
    let last = body.bytes().filter(|byte| *byte == b'\n').count();
    for (offset, part) in body.split('\n').enumerate() {
        let mut part = part.strip_suffix('\r').unwrap_or(part);
        if offset > 0 {
            part = strip_containers(part, containers);
        }
        if trim && offset == 0 {
            part = part.strip_prefix(' ').unwrap_or(part);
        }
        if trim && offset == last {
            part = part.strip_suffix(' ').unwrap_or(part);
        }
        style.append(&mut lines[row + offset], part);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ThemePreset};

    fn custom_theme() -> Theme {
        let directory = tempfile::tempdir().unwrap();
        let colorschemes = directory.path().join("colorscheme");
        std::fs::create_dir(&colorschemes).unwrap();
        std::fs::write(
            colorschemes.join("custom.toml"),
            r##"
                [syntax]
                foreground = "#123450"
                comment = "#123451"
                variable = "#123452"
                constant = "#123453"
                type = "#123454"
                string = "#123455"
                escape = "#123456"
                function = "#123457"
                keyword = "#123458"
                embedded = "#123459"
            "##,
        )
        .unwrap();
        Config::parse(
            &directory.path().join("config.toml"),
            "[theme]\npreset = 'custom'",
        )
        .unwrap()
        .theme
    }

    fn text(line: &RenderedLine) -> String {
        line.spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn cell_texts(table: &TableRow) -> Vec<String> {
        table
            .cells
            .iter()
            .map(|cell| cell.iter().map(|span| span.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn underline_nests_and_combines_with_markdown_styles() {
        let lines = project(
            "<u>outer <u>**世界**</u> *italic* ~~strike~~ [link](target) `code`</u> plain",
            &Theme::default(),
        );
        assert_eq!(text(&lines[0]), "outer 世界 italic strike link code plain");
        for (fragment, check) in [
            ("世界", (true, false, false, false)),
            ("italic", (false, true, false, false)),
            ("strike", (false, false, true, false)),
            ("code", (false, false, false, true)),
        ] {
            let span = lines[0]
                .spans
                .iter()
                .find(|span| span.text == fragment)
                .unwrap();
            assert!(span.underline);
            assert_eq!((span.bold, span.italic, span.strike, span.code), check);
        }
        let link = lines[0]
            .spans
            .iter()
            .find(|span| span.text == "link")
            .unwrap();
        assert!(link.underline);
        assert_eq!(link.link.as_deref(), Some("target"));
        assert!(!lines[0].spans.last().unwrap().underline);
        assert_eq!(
            lines[0].source_column("outer ".len()),
            "<u>outer <u>**".len()
        );
    }

    #[test]
    fn underline_at_block_start_preserves_physical_rows() {
        let source = "<u>\n**first**\n<u>second</u>\n</u>\n\nplain";
        let lines = project(source, &Theme::default());
        assert_eq!(text(&lines[0]), "");
        assert_eq!(text(&lines[1]), "first");
        assert!(lines[1].spans[0].bold && lines[1].spans[0].underline);
        assert_eq!(text(&lines[2]), "second");
        assert!(lines[2].spans[0].underline);
        assert_eq!(lines[2].source_column(0), 3);
        assert_eq!(text(&lines[3]), "");
        assert_eq!(text(&lines[5]), "plain");
        assert!(!lines[5].spans[0].underline);
    }

    #[test]
    fn underline_does_not_consume_tags_in_code_or_escape_blocks() {
        let lines = project(
            "`<u>literal</u>` <u>yes `</u>` still</u>\n\n```\n<u>fenced</u>\n```\n\n<u>unclosed\n\nplain</u>",
            &Theme::default(),
        );
        assert_eq!(text(&lines[0]), "<u>literal</u> yes </u> still");
        assert!(!lines[0].spans[0].underline);
        let closing = lines[0]
            .spans
            .iter()
            .find(|span| span.text == "</u>")
            .unwrap();
        assert!(closing.code && closing.underline);
        assert_eq!(text(&lines[3]), "<u>fenced</u>");
        assert!(lines[3].spans.iter().all(|span| !span.underline));
        assert_eq!(text(&lines[6]), "<u>unclosed");
        assert_eq!(text(&lines[8]), "plain</u>");
        assert!(
            lines[6..]
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| !span.underline)
        );
    }

    #[test]
    fn underline_keeps_unsupported_html_and_escaped_tags_literal() {
        let source = "<div>\n<u>not interpreted</u>\n</div>\n\nbefore <span>literal</span> <u class=\"x\">unsupported</u> &lt;u&gt;escaped&lt;/u&gt; \\<u>escaped\\</u>";
        let lines = project(source, &Theme::default());
        assert_eq!(text(&lines[0]), "<div>");
        assert_eq!(text(&lines[1]), "<u>not interpreted</u>");
        assert_eq!(text(&lines[2]), "</div>");
        assert_eq!(
            text(&lines[4]),
            "before <span>literal</span> <u class=\"x\">unsupported</u> <u>escaped</u> <u>escaped</u>"
        );
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| !span.underline)
        );
    }

    #[test]
    fn task_offsets_come_from_parser_and_ignore_code() {
        let text = "- [ ] 世界\n> - [X] done\n\n```md\n- [ ] code\n```\n\nplain [ ] text";
        let projected = project(text, &Theme::default());
        assert_eq!(projected[0].task_marker, Some(2));
        assert_eq!(projected[1].task_marker, Some(4));
        assert_eq!(projected[4].task_marker, None);
        assert_eq!(projected[7].task_marker, None);
    }

    #[test]
    fn multiline_styles_keep_unicode_and_physical_rows() {
        let lines = project("**hé *世界*\nencore**\n\n", &Theme::default());
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines.iter().map(|line| line.source_row).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(text(&lines[0]), "hé 世界");
        assert_eq!(text(&lines[1]), "encore");
        assert!(lines[0].spans.iter().all(|span| span.bold));
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.text == "世界" && span.italic)
        );
        assert!(lines[1].spans[0].bold);
        assert!(lines[2].spans.is_empty());
        assert!(lines[3].spans.is_empty());
    }

    #[test]
    fn fenced_blocks_do_not_parse_markdown_or_shift_following_rows() {
        let lines = project(
            "```rust\n**literal**\n\n世界\n```\nafter\n",
            &Theme::default(),
        );
        assert_eq!(lines.len(), 7);
        assert!(lines[0].spans.is_empty());
        assert_eq!(text(&lines[1]), "**literal**");
        assert!(lines[1].spans[0].code);
        assert!(!lines[1].spans[0].bold);
        assert!(lines[2].spans.is_empty());
        assert_eq!(text(&lines[3]), "世界");
        assert!(lines[4].spans.is_empty());
        assert_eq!(text(&lines[5]), "after");
        assert_eq!(lines[5].kind, BlockKind::Paragraph);
        assert!(lines[6].spans.is_empty());
    }

    #[test]
    fn fenced_code_is_highlighted_with_colors_and_no_line_breaks() {
        let lines = project(
            "```rust\nfn main() {\n    let x = 1;\n}\n```\nprose\n",
            &Theme::default(),
        );
        // Fence lines stay empty; body rows carry highlighted spans.
        assert!(lines[0].spans.is_empty());
        assert_eq!(text(&lines[1]), "fn main() {");
        assert_eq!(text(&lines[2]), "    let x = 1;");
        assert!(lines[4].spans.is_empty());
        assert_eq!(text(&lines[5]), "prose");
        for line in &lines[1..4] {
            assert!(!line.spans.is_empty(), "body row must have spans");
            for span in &line.spans {
                assert!(span.code);
                let hex = span.color.as_deref().expect("highlighted color");
                assert_eq!(hex.len(), 7);
                assert!(span.text.lines().count() <= 1, "no breaks in spans");
            }
        }
        // A keyword and a plain identifier highlight differently.
        let colors: Vec<_> = lines[1]
            .spans
            .iter()
            .filter_map(|s| s.color.as_deref())
            .collect();
        assert!(colors.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn syntax_themes_change_colors_without_changing_source_or_mapping() {
        let source = "```rust\nfn main() {\n    let greeting = \"hé 世界\";\n    // comment\n}\n```\n**prose** [link](target)\n";
        let mut baseline = project(source, &Theme::default());
        for line in &mut baseline {
            for span in &mut line.spans {
                span.color = None;
            }
        }
        let mut palettes = Vec::new();
        for theme in ThemePreset::ALL
            .iter()
            .map(|preset| preset.theme())
            .chain([custom_theme()])
        {
            let mut projection = project(source, &theme);
            let colors: Vec<_> = projection
                .iter()
                .flat_map(|line| &line.spans)
                .filter_map(|span| span.color.clone())
                .collect();
            assert!(
                !palettes.contains(&colors),
                "{} must not reuse another theme's syntax palette",
                theme.preset
            );
            palettes.push(colors);
            for (row, physical) in source.lines().enumerate().take(5).skip(1) {
                assert_eq!(text(&projection[row]), physical);
                for (column, _) in physical.char_indices() {
                    assert_eq!(projection[row].source_column(column), column);
                }
            }
            for line in &mut projection {
                for span in &mut line.spans {
                    span.color = None;
                }
            }
            assert_eq!(
                projection, baseline,
                "{} changed more than colors",
                theme.preset
            );
        }
    }

    #[test]
    fn custom_syntax_colors_reach_highlighted_roles_on_either_background() {
        let source = "```rust\nfn main() { let greeting = \"hé 世界\"; }\n// comment\n```\n```\nplain body\n```";
        let mut theme = custom_theme();
        for background in ["#171b22", "#faf9f6"] {
            theme.background = background.into();
            let lines = project(source, &theme);
            for (fragment, expected) in [
                ("fn", "#123458"),
                ("main", "#123457"),
                ("hé 世界", "#123455"),
            ] {
                let span = lines[1]
                    .spans
                    .iter()
                    .find(|span| span.text.contains(fragment))
                    .expect("syntax token");
                assert_eq!(span.color.as_deref(), Some(expected), "{fragment}");
            }
            assert!(
                lines[2]
                    .spans
                    .iter()
                    .all(|span| span.color.as_deref() == Some("#123451"))
            );
            assert!(
                lines[5]
                    .spans
                    .iter()
                    .all(|span| span.color.as_deref() == Some("#123450"))
            );
        }
    }

    #[test]
    fn syntax_uses_legible_light_ink_even_for_dark_presets_with_overrides() {
        fn luminance(hex: &str) -> f64 {
            let rgb = crate::config::parse_color(hex).unwrap();
            let linear = |shift: u32| {
                let channel = ((rgb >> shift) & 0xff) as f64 / 255.;
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
        }

        let source =
            "```rust\nfn main() { let value: i32 = 123; println!(\"hé 世界\"); }\n// comment\n```";
        for &preset in ThemePreset::ALL {
            let mut theme = preset.theme();
            if theme.is_dark() {
                theme.background = "#faf9f6".into();
            }
            let light = project(source, &theme);
            let background = luminance(&theme.background);
            let colors: Vec<_> = light
                .iter()
                .flat_map(|line| &line.spans)
                .filter_map(|span| span.color.as_deref())
                .collect();
            assert!(colors.windows(2).any(|pair| pair[0] != pair[1]));
            for color in colors {
                let contrast = (background + 0.05) / (luminance(color) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "{preset:?}: {color} has contrast {contrast}"
                );
            }
            // The same preset must also honor a dark background override.
            theme.background = "#171b22".into();
            let dark = project(source, &theme);
            assert_ne!(light[1].spans, dark[1].spans, "{preset:?}");
        }
    }

    #[test]
    fn unknown_fence_language_highlights_as_plain_text() {
        let lines = project("```\nplain body\n```\n", &Theme::default());
        assert_eq!(text(&lines[1]), "plain body");
        assert!(lines[1].spans.iter().all(|span| span.code));
        assert!(lines[1].spans.iter().all(|span| span.color.is_some()));
    }

    #[test]
    fn image_titles_set_preview_width() {
        let lines = project(
            "![a](a.png)\n![b](b.png \"40%\")\n![c](c.png \"320px\")\n![d](d.png \"nope\")\n",
            &Theme::default(),
        );
        assert_eq!(lines[0].images[0].width, None);
        assert_eq!(lines[1].images[0].width, Some(ImageWidth::Fraction(0.4)));
        assert_eq!(lines[2].images[0].width, Some(ImageWidth::Points(320.0)));
        assert_eq!(lines[3].images[0].width, None);
        assert_eq!(lines[3].images[0].url, "d.png");
    }

    #[test]
    fn multiline_code_keeps_container_prefixes_out_of_content() {
        let lines = project("> - before ` hé\n>   世界 ` after\n", &Theme::default());
        assert_eq!(text(&lines[0]), "• before hé");
        assert_eq!(text(&lines[1]), "世界 after");
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.text == "hé" && span.code)
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|span| span.text == "世界" && span.code)
        );
    }

    #[test]
    fn tables_links_and_entities_keep_source_rows() {
        let lines = project(
            "| name | value |\n| --- | --- |\n| [**A**](https://a.test) | &amp; |\n\n![*alt*](image.png)\n",
            &Theme::default(),
        );
        let header = lines[0].table.as_ref().unwrap();
        assert_eq!(cell_texts(header), ["name", "value"]);
        assert!(header.header);
        assert!(header.cells.iter().flatten().all(|span| span.bold));
        let separator = lines[1].table.as_ref().unwrap();
        assert!(separator.separator);
        assert!(!separator.header);
        assert!(separator.cells.is_empty());
        let body = lines[2].table.as_ref().unwrap();
        assert_eq!(cell_texts(body), ["A", "&"]);
        assert!(!body.header);
        assert!(!body.separator);
        for line in &lines[..3] {
            let table = line.table.as_ref().unwrap();
            assert_eq!((table.start, table.end), (0, 3));
            assert!(line.spans.is_empty());
        }
        assert!(lines[3..].iter().all(|line| line.table.is_none()));
        let link = &body.cells[0][0];
        assert!(link.bold);
        assert_eq!(link.link.as_deref(), Some("https://a.test"));
        assert!(lines[3].spans.is_empty());
        assert_eq!(text(&lines[4]), "alt");
        assert!(lines[4].spans[0].italic);
        assert_eq!(lines[4].spans[0].link.as_deref(), Some("image.png"));
        assert!(lines[5].spans.is_empty());
    }

    #[test]
    fn table_cells_preserve_parser_escaping_alignment_and_inline_styles() {
        let lines = project(
            "| left | center | right | plain |\n\
                 | :--- | :---: | ---: | --- |\n\
                 | hé\\|世界 | `a\\|b` | &vert; &amp; | [*é*](https://a.test) ~~old~~ |\n",
            &Theme::default(),
        );
        let table = lines[2].table.as_ref().unwrap();
        assert_eq!(
            table.alignments.as_ref(),
            [
                Alignment::Left,
                Alignment::Center,
                Alignment::Right,
                Alignment::None
            ]
        );
        assert_eq!(cell_texts(table), ["hé|世界", "a|b", "| &", "é old"]);
        assert!(table.cells[1][0].code);
        assert!(table.cells[3][0].italic);
        assert_eq!(table.cells[3][0].link.as_deref(), Some("https://a.test"));
        assert!(
            table.cells[3]
                .iter()
                .any(|span| span.text == "old" && span.strike)
        );
        assert!(table.cells.iter().flatten().all(|span| !span.bold));
    }

    #[test]
    fn missing_table_cells_stay_on_their_enclosing_source_row() {
        let lines = project(
            "| a | b | c |\n| --- | --- | --- |\n| | x |\n| only |\n| 1 | 2 | 3 | ignored |\n\nprose",
            &Theme::default(),
        );
        assert_eq!(cell_texts(lines[2].table.as_ref().unwrap()), ["", "x", ""]);
        assert_eq!(
            cell_texts(lines[3].table.as_ref().unwrap()),
            ["only", "", ""]
        );
        assert_eq!(
            cell_texts(lines[4].table.as_ref().unwrap()),
            ["1", "2", "3"]
        );
        assert!(lines[5].table.is_none());
        assert!(lines[6].table.is_none());
        assert_eq!(text(&lines[6]), "prose");
        assert_eq!(
            lines.iter().map(|line| line.source_row).collect::<Vec<_>>(),
            (0..7).collect::<Vec<_>>()
        );
    }

    #[test]
    fn quoted_crlf_tables_and_neighboring_tables_keep_distinct_ranges() {
        let lines = project(
            "> | hé | value |\r\n> | :--- | ---: |\r\n> | 世界 | |\r\n\r\n\
                 | next | table |\r\n| --- | :---: |\r\n\r\nfollowing\r\n",
            &Theme::default(),
        );
        for line in &lines[..3] {
            let table = line.table.as_ref().unwrap();
            assert_eq!((table.start, table.end), (0, 3));
            assert_eq!(line.kind, BlockKind::Quote);
        }
        assert_eq!(cell_texts(lines[2].table.as_ref().unwrap()), ["世界", ""]);
        assert!(lines[3].table.is_none());
        for line in &lines[4..6] {
            let table = line.table.as_ref().unwrap();
            assert_eq!((table.start, table.end), (4, 6));
            assert_eq!(
                table.alignments.as_ref(),
                [Alignment::None, Alignment::Center]
            );
        }
        assert!(lines[5].table.as_ref().unwrap().separator);
        assert!(lines[6..].iter().all(|line| line.table.is_none()));
        assert_eq!(text(&lines[7]), "following");
        assert_eq!(lines.len(), 9);
    }

    #[test]
    fn header_only_table_at_eof_includes_its_delimiter() {
        let lines = project("| heading |\n| --- |", &Theme::default());
        assert_eq!(lines.len(), 2);
        assert_eq!(cell_texts(lines[0].table.as_ref().unwrap()), ["heading"]);
        let separator = lines[1].table.as_ref().unwrap();
        assert_eq!((separator.start, separator.end), (0, 2));
        assert!(separator.separator);
        assert!(separator.cells.is_empty());
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;

    #[test]
    fn preview_offsets_skip_markup_and_keep_unicode_and_entities() {
        let source = "# **café** and [世界](target) &amp; `code`";
        let lines = project(source, &Theme::default());
        let rendered: String = lines[0]
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        for word in ["café", "and", "世界", "code"] {
            assert_eq!(
                lines[0].source_column(rendered.find(word).unwrap()),
                source.find(word).unwrap(),
                "{word}"
            );
        }
        assert_eq!(
            lines[0].source_column(rendered.find('&').unwrap()),
            source.find("&amp;").unwrap()
        );
    }

    #[test]
    fn list_marker_and_multiline_text_map_to_physical_source_rows() {
        let source = "- **first**\n  second café\n\n> [link](url)";
        let lines = project(source, &Theme::default());
        for (row, word) in [(0, "first"), (1, "café"), (3, "link")] {
            let rendered: String = lines[row]
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect();
            assert_eq!(
                lines[row].source_column(rendered.find(word).unwrap()),
                source.lines().nth(row).unwrap().find(word).unwrap()
            );
        }
    }
}
