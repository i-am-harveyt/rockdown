use crate::markdown::Span;
use gpui::*;

pub(super) fn run(text: &str, font: Font, color: Hsla) -> TextRun {
    TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Wrap when needed and return the effective body offset alongside the layout.
/// A marker wider than the column wraps normally instead of hanging offscreen.
pub(crate) fn wrap_body(
    line: &ShapedLine,
    runs: &[TextRun],
    width: Pixels,
    body_start: usize,
    window: &Window,
) -> Result<(Option<WrappedLine>, usize)> {
    if line.width <= width {
        return Ok((None, 0));
    }
    let body_start = if line.x_for_index(body_start) < width {
        body_start
    } else {
        0
    };
    let font_size = line.font_size;
    if body_start == 0 {
        let wrapped = window
            .text_system()
            .shape_text(line.text.clone(), font_size, runs, Some(width), None)?
            .pop();
        return Ok((wrapped, 0));
    }
    let mut skip = body_start;
    let body_runs: Vec<_> = runs
        .iter()
        .filter_map(|run| {
            let consumed = skip.min(run.len);
            skip -= consumed;
            (run.len > consumed).then(|| TextRun {
                len: run.len - consumed,
                ..run.clone()
            })
        })
        .collect();
    let wrapped = window
        .text_system()
        .shape_text(
            line.text[body_start..].to_owned().into(),
            font_size,
            &body_runs,
            Some((width - line.x_for_index(body_start)).max(px(1.))),
            None,
        )?
        .pop();
    let body_start = if wrapped.is_some() { body_start } else { 0 };
    Ok((wrapped, body_start))
}

pub(super) struct SpanStyle {
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) foreground: Hsla,
    pub(super) muted: Hsla,
    pub(super) panel: Hsla,
    pub(super) bold: Option<Hsla>,
    pub(super) italic: Option<Hsla>,
    pub(super) bold_italic: Option<Hsla>,
    pub(super) code: Hsla,
    pub(super) link: Hsla,
    pub(super) strikethrough: Option<Hsla>,
    pub(super) quote: Option<Hsla>,
}

/// Parse a `#rrggbb` color from the syntax highlighter into a render color.
fn parse_hex(hex: &str) -> Option<Hsla> {
    crate::config::parse_color(hex)
        .ok()
        .map(|value| rgb(value).into())
}

impl SpanStyle {
    pub(super) fn text(&self, spans: &[Span], base: Hsla) -> (String, Vec<TextRun>) {
        let mut text = String::new();
        let mut runs = Vec::with_capacity(spans.len());
        for span in spans {
            text.push_str(&span.text);
            let color = if span.code {
                self.code
            } else if span.link.is_some() {
                self.link
            } else {
                (span.bold && span.italic)
                    .then_some(self.bold_italic)
                    .flatten()
                    .or_else(|| span.bold.then_some(self.bold).flatten())
                    .or_else(|| span.italic.then_some(self.italic).flatten())
                    .or_else(|| span.strike.then_some(self.strikethrough).flatten())
                    .unwrap_or(base)
            };
            let mut style = run(&span.text, self.font.clone(), color);
            if span.bold {
                style.font.weight = FontWeight::BOLD;
            }
            if span.italic {
                style.font.style = FontStyle::Italic;
            }
            if let Some(hex) = &span.color {
                // Highlighted spans carry their own foreground; the block's
                // background quad already provides the panel tint.
                if let Some(tint) = parse_hex(hex) {
                    style.color = tint;
                }
            } else if span.code {
                style.background_color = Some(self.panel);
            }
            if span.link.is_some() || span.underline {
                style.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(style.color),
                    wavy: false,
                });
            }
            if span.strike {
                style.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(self.strikethrough.unwrap_or(self.muted)),
                });
            }
            runs.push(style);
        }
        (text, runs)
    }
}
