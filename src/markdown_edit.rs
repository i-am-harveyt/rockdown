//! Small source edits for Markdown typing; literal paste remains untouched.

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EnterEdit {
    Continue(String),
    Exit { from: usize, to: usize },
}

/// Inspect only the prefix before the caret. The caller excludes code blocks
/// and non-Markdown documents using the document's parsed block kinds.
pub(crate) fn enter_edit(line: &str, col: usize) -> Option<EnterEdit> {
    if !line.is_char_boundary(col) {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    let mut last_quote = None;
    while bytes.get(i) == Some(&b'>') {
        last_quote = Some(i);
        i += 1;
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
    }
    let list_start = i;
    let mut next_marker = None;
    if matches!(bytes.get(i), Some(b'-' | b'+' | b'*'))
        && matches!(bytes.get(i + 1), Some(b' ' | b'\t'))
    {
        next_marker = Some(line[i..i + 1].to_owned());
        i += 1;
    } else {
        let digits = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if (1..=9).contains(&(i - digits))
            && matches!(bytes.get(i), Some(b'.' | b')'))
            && matches!(bytes.get(i + 1), Some(b' ' | b'\t'))
        {
            let number: u64 = line[digits..i].parse().ok()?;
            next_marker = Some(format!("{}{}", number + 1, bytes[i] as char));
            i += 1;
        } else {
            i = list_start;
        }
    }
    let mut next = line[..list_start].to_owned();
    if let Some(marker) = &next_marker {
        next.push_str(marker);
        let spacing = i;
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        next.push_str(&line[spacing..i]);
        if bytes.get(i) == Some(&b'[')
            && matches!(bytes.get(i + 1), Some(b' ' | b'x' | b'X'))
            && bytes.get(i + 2) == Some(&b']')
            && (i + 3 == bytes.len() || matches!(bytes.get(i + 3), Some(b' ' | b'\t')))
        {
            i += 3;
            let spacing = i;
            while matches!(bytes.get(i), Some(b' ' | b'\t')) {
                i += 1;
            }
            next.push_str("[ ]");
            if spacing == i {
                next.push(' ');
            } else {
                next.push_str(&line[spacing..i]);
            }
        }
    } else if last_quote.is_none() {
        return None;
    }
    // Never duplicate a partially typed prefix, or move its remaining syntax.
    if col < i {
        return None;
    }
    if line[i..].trim().is_empty() {
        let from = if next_marker.is_some() {
            list_start
        } else {
            last_quote?
        };
        Some(EnterEdit::Exit {
            from,
            to: line.len(),
        })
    } else {
        Some(EnterEdit::Continue(format!("\n{next}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continues_markers_and_resets_tasks() {
        for (text, expected) in [
            ("- 世界", "\n- "),
            ("  + nested", "\n  + "),
            ("9. item", "\n10. "),
            ("2) item", "\n3) "),
            ("> quote", "\n> "),
            ("> > deep", "\n> > "),
            ("> - [X] done", "\n> - [ ] "),
            ("- [ ] task", "\n- [ ] "),
        ] {
            assert_eq!(
                enter_edit(text, text.len()),
                Some(EnterEdit::Continue(expected.into()))
            );
        }
    }

    #[test]
    fn exits_one_empty_container_without_inserting_a_line() {
        for (text, from) in [("- ", 0), ("> - [ ] ", 2), ("> > ", 2), ("  - ", 2)] {
            assert_eq!(
                enter_edit(text, text.len()),
                Some(EnterEdit::Exit {
                    from,
                    to: text.len()
                })
            );
        }
    }

    #[test]
    fn ordinary_text_and_partial_prefixes_are_literal() {
        for text in ["text", "---", "*emphasis*", "1234567890. long", "-no space"] {
            assert_eq!(enter_edit(text, text.len()), None);
        }
        assert_eq!(enter_edit("- [ ] task", 3), None);
        assert_eq!(enter_edit("- text", 0), None);
        assert_eq!(
            enter_edit("- first second", 7),
            Some(EnterEdit::Continue("\n- ".into()))
        );
    }
}
