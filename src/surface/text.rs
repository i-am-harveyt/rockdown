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
