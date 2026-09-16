use super::{Buffer, Line, first_nonblank};

impl Buffer {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
