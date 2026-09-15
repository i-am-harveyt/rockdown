use std::{collections::HashMap, sync::LazyLock};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;

/// A parsed heading and its document-local fragment identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// Zero-based physical source row, including quote/list prefixes.
    pub row: usize,
    pub level: u8,
    pub title: String,
    /// Fragment identifier without the leading `#` or URL encoding.
    pub anchor: String,
}

// Keep Unicode letters, combining marks and numbers, plus GitHub-style word
// separators. Combining marks matter for both decomposed text and lowercasing.
static SLUG_PUNCTUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{M}\p{N}_\-\s]").unwrap());

/// Extract headings from the same Markdown dialect used by the editor projection.
/// Parser events exclude apparent headings in code and preserve headings nested
/// in containers, where projected line kinds may instead describe the container.
pub fn headings(source: &str) -> Vec<Heading> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut result = Vec::new();
    let mut current: Option<Heading> = None;
    let mut anchors = HashMap::new();
    let mut source_offset = 0;
    let mut source_row = 0;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                source_row += source[source_offset..range.start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                source_offset = range.start;
                current = Some(Heading {
                    row: source_row,
                    level: level as u8,
                    title: String::new(),
                    anchor: String::new(),
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(mut heading) = current.take() {
                    heading.anchor = unique_anchor(&heading.title, &mut anchors);
                    result.push(heading);
                }
            }
            // Link labels and image alt text arrive as ordinary inline events.
            // Inline HTML tags themselves are not part of the visible label.
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = current.as_mut() {
                    heading.title.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(heading) = current.as_mut() {
                    heading.title.push(' ');
                }
            }
            _ => {}
        }
    }
    result
}

fn unique_anchor(title: &str, anchors: &mut HashMap<String, usize>) -> String {
    let lowercase = title.to_lowercase();
    let stripped = SLUG_PUNCTUATION.replace_all(&lowercase, "");
    let base: String = stripped
        .chars()
        .map(|ch| if ch.is_whitespace() { '-' } else { ch })
        .collect();
    let anchor = if let Some(mut suffix) = anchors.get(&base).copied() {
        loop {
            suffix += 1;
            let candidate = format!("{base}-{suffix}");
            if !anchors.contains_key(&candidate) {
                *anchors.get_mut(&base).unwrap() = suffix;
                break candidate;
            }
        }
    } else {
        base
    };
    anchors.insert(anchor.clone(), 0);
    anchor
}

/// Resolve a local fragment to its heading's source row. Fragment identifiers
/// are case-sensitive; percent escapes are decoded once and `+` is not a space.
/// Empty fragments, invalid escapes/UTF-8 and unknown anchors do not resolve.
pub fn resolve_anchor(headings: &[Heading], fragment: &str) -> Option<usize> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    if fragment.is_empty() {
        return None;
    }
    if !fragment.contains('%') {
        return headings
            .iter()
            .find(|heading| heading.anchor == fragment)
            .map(|heading| heading.row);
    }

    let mut decoded = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = std::str::from_utf8(&decoded).ok()?;
    headings
        .iter()
        .find(|heading| heading.anchor == decoded)
        .map(|heading| heading.row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_maps_visible_labels_and_nested_multiline_headings() {
        let source = concat!(
            "Préface\r\n\r\n",
            "# A *bold* [link](https://example.com) ![alt `code`](image.png) &amp; <em>HTML</em> ###\r\n",
            "\r\n",
            "> ## Quoted ~~old~~\r\n\r\n",
            "- ### Listed\r\n\r\n",
            "Setext *first*\r\nsecond `line`\r\n===\r\n\r\n",
            "```md\r\n# Not a heading\r\n```\r\n\r\n",
            "    # Also code\r\n\r\n",
            "Escaped \\# and \\*literal\\*\r\n---\r\n",
        );
        let actual: Vec<_> = headings(source)
            .into_iter()
            .map(|heading| (heading.row, heading.level, heading.title))
            .collect();
        assert_eq!(
            actual,
            vec![
                (2, 1, "A bold link alt code & HTML".into()),
                (4, 2, "Quoted old".into()),
                (6, 3, "Listed".into()),
                (8, 1, "Setext first second line".into()),
                (18, 2, "Escaped # and *literal*".into()),
            ]
        );
    }

    #[test]
    fn anchors_disambiguate_duplicates_and_numeric_title_collisions() {
        let source = "# Foo\n# Foo-1\n# Foo\n# Foo\n# Foo-1\n# Foo-2\n";
        let headings = headings(source);
        assert_eq!(
            headings
                .iter()
                .map(|heading| heading.anchor.as_str())
                .collect::<Vec<_>>(),
            ["foo", "foo-1", "foo-2", "foo-3", "foo-1-1", "foo-2-1"]
        );
        assert_eq!(resolve_anchor(&headings, "#foo-2"), Some(2));
        assert_eq!(resolve_anchor(&headings, "foo-2-1"), Some(5));
    }

    #[test]
    fn unicode_fragments_decode_once_without_losing_combining_marks() {
        let headings = headings("# CAFÉ 中文!\n# İ E\u{301} _ok_ `under_score`\n# CAFÉ 中文!\n");
        assert_eq!(headings[0].anchor, "café-中文");
        assert_eq!(headings[1].anchor, "i\u{307}-e\u{301}-ok-under_score");
        assert_eq!(
            resolve_anchor(&headings, "#caf%C3%A9-%E4%B8%AD%E6%96%87"),
            Some(0)
        );
        assert_eq!(resolve_anchor(&headings, "#café-中文-1"), Some(2));
        assert_eq!(resolve_anchor(&headings, "#caf%25C3%25A9-中文"), None);
    }

    #[test]
    fn malformed_empty_and_unknown_fragments_do_not_resolve() {
        let headings = headings("# A B\n# !!!\n");
        for fragment in [
            "", "#", "#missing", "#A-B", "#a+b", "#a%20b", "#%", "#%2", "#%GG", "#%FF", "#%C0%AF",
        ] {
            assert_eq!(resolve_anchor(&headings, fragment), None, "{fragment:?}");
        }
        assert_eq!(resolve_anchor(&headings, "#a%2db"), Some(0));
    }
}
