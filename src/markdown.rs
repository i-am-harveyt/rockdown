use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

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
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedLine {
    pub source_row: usize,
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    pub images: Vec<String>,
}

#[derive(Default)]
struct Style {
    strong: usize,
    emphasis: usize,
    strike: usize,
    code: usize,
    header: usize,
    links: Vec<String>,
}

impl Style {
    fn append(&self, line: &mut RenderedLine, text: &str) {
        if text.is_empty() {
            return;
        }
        let bold = self.strong > 0 || self.header > 0;
        let italic = self.emphasis > 0;
        let code = self.code > 0;
        let strike = self.strike > 0;
        let link = self.links.last();
        if let Some(last) = line.spans.last_mut()
            && last.bold == bold
            && last.italic == italic
            && last.code == code
            && last.strike == strike
            && last.link.as_ref() == link
        {
            last.text.push_str(text);
        } else {
            line.spans.push(Span {
                text: text.to_owned(),
                bold,
                italic,
                code,
                strike,
                link: link.cloned(),
            });
        }
    }
}

#[derive(Clone, Copy)]
enum Container {
    Quote,
    Item(usize),
}

/// Project a single full-document parse onto physical source lines. Delimiter-only
/// and trailing empty lines deliberately remain present, so a click always refers
/// to the same row in the editable buffer. Offsets are UTF-8 byte offsets.
pub fn project(source: &str) -> Vec<RenderedLine> {
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
            kind: BlockKind::Paragraph,
            spans: Vec::new(),
            images: Vec::new(),
        })
        .collect();
    // Difference arrays classify nested containers in one final linear pass,
    // rather than revisiting all their rows for every level of nesting.
    let mut quotes = vec![0isize; starts.len() + 1];
    let mut lists = vec![0isize; starts.len() + 1];
    let mut style = Style::default();
    let mut containers = Vec::new();
    let mut ordinals: Vec<Option<u64>> = Vec::new();
    let mut cell_index = 0usize;
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        let row = source_row(&starts, range.start);
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    set_kind(&mut lines, &starts, &range, BlockKind::Heading(level as u8));
                }
                Tag::BlockQuote(_) => {
                    cover(&mut quotes, &starts, &range);
                    containers.push(Container::Quote);
                }
                Tag::CodeBlock(_) => {
                    set_kind(&mut lines, &starts, &range, BlockKind::Code);
                    style.code += 1;
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
                }
                Tag::Emphasis => style.emphasis += 1,
                Tag::Strong => style.strong += 1,
                Tag::Strikethrough => style.strike += 1,
                Tag::Image { dest_url, .. } => {
                    lines[row].images.push(dest_url.to_string());
                    style.links.push(dest_url.into_string());
                }
                Tag::Link { dest_url, .. } => {
                    style.links.push(dest_url.into_string());
                }
                Tag::TableHead => {
                    style.header += 1;
                    cell_index = 0;
                }
                Tag::TableRow => cell_index = 0,
                Tag::TableCell => {
                    if cell_index > 0 {
                        style.append(&mut lines[row], " │ ");
                    }
                    cell_index += 1;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::CodeBlock => style.code -= 1,
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
                TagEnd::TableHead => style.header -= 1,
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                append_text(&mut lines, row, &text, &style);
            }
            Event::Code(text) => {
                style.code += 1;
                let raw = &source[range];
                if raw.contains('\n') {
                    append_multiline_code(&mut lines, row, raw, &containers, &style);
                } else {
                    style.append(&mut lines[row], &text);
                }
                style.code -= 1;
            }
            Event::Rule => lines[row].kind = BlockKind::Rule,
            Event::TaskListMarker(checked) => {
                style.append(&mut lines[row], if checked { "[x] " } else { "[ ] " });
            }
            // Breaks already have distinct physical rows. Adding a space here
            // would change the visual line ending and can leak inline styling.
            Event::SoftBreak | Event::HardBreak => {}
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                append_text(&mut lines, row, &text, &style);
            }
            Event::FootnoteReference(label) => style.append(&mut lines[row], &label),
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

fn append_text(lines: &mut [RenderedLine], row: usize, text: &str, style: &Style) {
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

    fn text(line: &RenderedLine) -> String {
        line.spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn multiline_styles_keep_unicode_and_physical_rows() {
        let lines = project("**hé *世界*\nencore**\n\n");
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
        let lines = project("```rust\n**literal**\n\n世界\n```\nafter\n");
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
    fn multiline_code_keeps_container_prefixes_out_of_content() {
        let lines = project("> - before ` hé\n>   世界 ` after\n");
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
        );
        assert_eq!(text(&lines[0]), "name │ value");
        assert!(lines[0].spans.iter().all(|span| span.bold));
        assert!(lines[1].spans.is_empty());
        assert_eq!(text(&lines[2]), "A │ &");
        let link = &lines[2].spans[0];
        assert!(link.bold);
        assert_eq!(link.link.as_deref(), Some("https://a.test"));
        assert!(lines[3].spans.is_empty());
        assert_eq!(text(&lines[4]), "alt");
        assert!(lines[4].spans[0].italic);
        assert_eq!(lines[4].spans[0].link.as_deref(), Some("image.png"));
        assert!(lines[5].spans.is_empty());
    }
}
