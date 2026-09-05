use crate::app::{Pane, Workspace};
use gpui::{prelude::*, *};
use rockdown::{markdown::BlockKind, vim::Mode};

#[derive(Default)]
pub(crate) struct SurfaceLayout {
    pub rows: Vec<HitRow>,
}
pub(crate) struct HitRow {
    pub source_row: usize,
    pub origin: Point<Pixels>,
    pub line: ShapedLine,
    pub raw: bool,
    pub height: Pixels,
}
struct DrawText {
    line: ShapedLine,
    wrapped: Option<WrappedLine>,
    origin: Point<Pixels>,
    height: Pixels,
}
#[derive(Default)]
pub(crate) struct Prepared {
    layout: SurfaceLayout,
    text: Vec<DrawText>,
    quads: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    images: Vec<AnyElement>,
}
pub(crate) struct Surface {
    pub workspace: Entity<Workspace>,
    pub pane: Pane,
}
impl IntoElement for Surface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

fn run(text: &str, font: Font, color: Hsla) -> TextRun {
    TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}
impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return Prepared::default();
        }
        let mut result = Prepared::default();
        let line_height = self.workspace.read(cx).config.line_height;
        let visible = (f32::from(bounds.size.height) / line_height)
            .floor()
            .max(1.) as usize;
        self.workspace
            .update(cx, |app, _| app.ensure_cursor_visible(self.pane, visible));
        if self.pane == Pane::Terminal {
            self.terminal(bounds, window, cx, &mut result);
            return result;
        }
        let app = self.workspace.read(cx);
        let foreground = app.color(&app.config.theme.foreground);
        let muted = app.color(&app.config.theme.muted);
        let accent = app.color(&app.config.theme.accent);
        let panel = app.color(&app.config.theme.panel);
        let font = font(app.config.font_family.clone());
        let buffer = if self.pane == Pane::Editor {
            &app.documents.current().buffer
        } else {
            &app.explorer.buffer
        };
        let top = app.tops[self.pane.index()];
        let gutter = if self.pane == Pane::Editor { 56. } else { 16. };
        let available = (bounds.size.width - px(gutter + 24.)).max(px(1.));
        let mut y = bounds.top() - px(app.scroll_offsets[self.pane.index()]);
        let mut images = Vec::new();
        for source_row in top..(top + visible + 1).min(buffer.lines.len()) {
            let active = source_row == buffer.row && app.pane == self.pane;
            let raw = active
                || self.pane == Pane::Explorer
                || buffer.selected_range(source_row).is_some();
            if y >= bounds.bottom() {
                break;
            }
            let mut font_size = app.config.font_size;
            let mut runs = Vec::new();
            let text = if raw {
                let text = buffer.lines[source_row].text.clone();
                runs.push(run(&text, font.clone(), foreground));
                text
            } else {
                let projection = &app.projection[source_row];
                match projection.kind {
                    BlockKind::Heading(level) => {
                        font_size =
                            (app.config.font_size + (7 - level) as f32 * 1.5).min(line_height - 3.)
                    }
                    BlockKind::Code => result.quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + px(gutter - 6.), y),
                            size(
                                (bounds.size.width - px(gutter)).max(px(0.)),
                                px(line_height),
                            ),
                        ),
                        panel,
                    )),
                    BlockKind::Quote => result.quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + px(gutter - 9.), y + px(3.)),
                            size(px(2.), px(line_height - 6.)),
                        ),
                        accent,
                    )),
                    BlockKind::Rule => result.quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + px(gutter), y + px(line_height / 2.)),
                            size((bounds.size.width - px(gutter + 16.)).max(px(0.)), px(1.)),
                        ),
                        muted,
                    )),
                    _ => {}
                }
                let mut text = String::new();
                for span in &projection.spans {
                    text.push_str(&span.text);
                    let mut style = run(
                        &span.text,
                        font.clone(),
                        if span.link.is_some() {
                            accent
                        } else {
                            foreground
                        },
                    );
                    if span.bold {
                        style.font.weight = FontWeight::BOLD;
                    }
                    if span.italic {
                        style.font.style = FontStyle::Italic;
                    }
                    if span.code {
                        style.background_color = Some(panel);
                        style.color = accent;
                    }
                    if span.link.is_some() {
                        style.underline = Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(accent),
                            wavy: false,
                        });
                    }
                    if span.strike {
                        style.strikethrough = Some(StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(muted),
                        });
                    }
                    runs.push(style);
                }
                text
            };
            let line = window
                .text_system()
                .shape_line(text.into(), px(font_size), &runs, None);
            let wrapped = if !raw && line.width > available {
                window
                    .text_system()
                    .shape_text(
                        line.text.clone(),
                        px(font_size),
                        &runs,
                        Some(available),
                        None,
                    )
                    .map(|mut lines| lines.pop())
                    .unwrap_or_else(|error| {
                        eprintln!("Wrapping text: {error}");
                        None
                    })
            } else {
                None
            };
            let mut row_height = px(line_height)
                * (wrapped
                    .as_ref()
                    .map_or(1, |line| line.wrap_boundaries.len() + 1) as f32);
            let caret = line.x_for_index(buffer.col.min(line.text.len()));
            let shift = if active && caret > available {
                caret - available
            } else {
                px(0.)
            };
            let origin = point(bounds.left() + px(gutter) - shift, y);
            if active {
                let mut highlight = accent;
                highlight.a = 0.055;
                result.quads.push(fill(
                    Bounds::new(
                        point(bounds.left(), y),
                        size(bounds.size.width, px(line_height)),
                    ),
                    highlight,
                ));
                let mut cursor_color = accent;
                cursor_color.a = if buffer.mode == Mode::Insert { 1. } else { 0.5 };
                let width = if buffer.mode == Mode::Insert {
                    px(2.)
                } else {
                    px(app.config.font_size * 0.6)
                };
                result.cursor = Some(fill(
                    Bounds::new(
                        point(origin.x + caret, y + px(3.)),
                        size(width, px(line_height - 6.)),
                    ),
                    cursor_color,
                ));
            }
            if let Some(range) = buffer.selected_range(source_row) {
                let mut color = accent;
                color.a = 0.25;
                let left = line.x_for_index(range.start);
                let right = line.x_for_index(range.end);
                result.quads.push(fill(
                    Bounds::new(
                        point(origin.x + left, y),
                        size((right - left).max(px(4.)), px(line_height)),
                    ),
                    color,
                ));
            }
            if self.pane == Pane::Editor {
                let number = format!("{:>4}", source_row + 1);
                let shaped = window.text_system().shape_line(
                    number.clone().into(),
                    px(app.config.font_size - 2.),
                    &[run(&number, font.clone(), muted)],
                    None,
                );
                result.text.push(DrawText {
                    line: shaped,
                    wrapped: None,
                    origin: point(bounds.left() + px(4.), y),
                    height: px(line_height),
                });
            }
            if !raw {
                let base = app
                    .documents
                    .current()
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .unwrap_or(&app.explorer.directory);
                for image in &app.projection[source_row].images {
                    // Remote images remain linked alt text: opening a document never phones home.
                    if image.contains("://") {
                        continue;
                    }
                    let height = px(160.);
                    images.push((
                        base.join(image),
                        point(origin.x, y + row_height),
                        available,
                        height,
                    ));
                    row_height += height;
                }
            }
            result.layout.rows.push(HitRow {
                source_row,
                origin,
                line: line.clone(),
                raw,
                height: row_height,
            });
            result.text.push(DrawText {
                line,
                wrapped,
                origin,
                height: px(line_height),
            });
            y += row_height;
        }
        let hidden_cursor = app.follow_cursor
            && app.pane == self.pane
            && !result
                .layout
                .rows
                .iter()
                .any(|row| row.source_row == buffer.row);
        if hidden_cursor {
            let row = buffer.row;
            self.workspace
                .update(cx, |app, _| app.tops[self.pane.index()] = row);
            return self.prepaint(None, None, bounds, &mut (), window, cx);
        }
        for (path, origin, width, height) in images {
            let mut image = img(path)
                .w(width)
                .h(height)
                .object_fit(ObjectFit::Contain)
                .with_fallback(|| {
                    div()
                        .size_full()
                        .text_sm()
                        .child("Image unavailable")
                        .into_any_element()
                })
                .into_any_element();
            image.layout_as_root(size(width.into(), height.into()), window, cx);
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                image.prepaint_at(origin, window, cx)
            });
            result.images.push(image);
        }
        result
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepared: &mut Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = self.workspace.read(cx);
        if app.pane == self.pane {
            window.handle_input(
                &app.focus,
                ElementInputHandler::new(bounds, self.workspace.clone()),
                cx,
            );
        }
        let focused = app.focus.is_focused(window);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepared.quads.drain(..) {
                window.paint_quad(quad);
            }
            for text in &prepared.text {
                let painted = if let Some(line) = &text.wrapped {
                    line.paint_background(
                        text.origin,
                        text.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .and_then(|_| {
                        line.paint(text.origin, text.height, TextAlign::Left, None, window, cx)
                    })
                } else {
                    text.line
                        .paint_background(text.origin, text.height, window, cx)
                        .and_then(|_| text.line.paint(text.origin, text.height, window, cx))
                };
                if let Err(error) = painted {
                    eprintln!("Text rendering: {error}");
                }
            }
            for image in &mut prepared.images {
                image.paint(window, cx);
            }
            if focused && let Some(cursor) = prepared.cursor.take() {
                window.paint_quad(cursor);
            }
        });
        self.workspace.update(cx, |app, _| {
            for row in &prepared.layout.rows {
                app.row_heights[self.pane.index()][row.source_row] = f32::from(row.height);
            }
            app.layouts[self.pane.index()] = std::mem::take(&mut prepared.layout);
        });
    }
}

impl Surface {
    fn terminal(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
        result: &mut Prepared,
    ) {
        let app = self.workspace.read(cx);
        let font = font(app.config.font_family.clone());
        let font_size = app.config.font_size;
        let height = px(font_size + 5.);
        let foreground = app.color(&app.config.theme.foreground);
        let background = app.color(&app.config.theme.background);
        let accent = app.color(&app.config.theme.accent);
        let probe = window.text_system().shape_line(
            "M".into(),
            px(font_size),
            &[run("M", font.clone(), foreground)],
            None,
        );
        let width = probe.width.max(px(1.));
        let rows = (f32::from(bounds.size.height) / f32::from(height))
            .floor()
            .max(1.) as u16;
        let cols = ((f32::from(bounds.size.width) - 16.) / f32::from(width))
            .floor()
            .max(1.) as u16;
        self.workspace.update(cx, |app, _| {
            if let Some(terminal) = &mut app.terminal
                && let Err(error) = terminal.resize(rows, cols)
            {
                app.message = error.to_string();
            }
        });
        let app = self.workspace.read(cx);
        let Some(terminal) = &app.terminal else {
            return;
        };
        let screen = terminal.screen();
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let origin = point(
                    bounds.left() + px(8.) + width * col as f32,
                    bounds.top() + height * row as f32,
                );
                let mut fg = terminal_color(cell.fgcolor(), foreground);
                let mut bg = terminal_color(cell.bgcolor(), background);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let cell_width = width * if cell.is_wide() { 2. } else { 1. };
                if bg != background {
                    result
                        .quads
                        .push(fill(Bounds::new(origin, size(cell_width, height)), bg));
                }
                if cell.contents().is_empty() {
                    continue;
                }
                let mut style = run(cell.contents(), font.clone(), fg);
                if cell.bold() {
                    style.font.weight = FontWeight::BOLD;
                }
                if cell.italic() {
                    style.font.style = FontStyle::Italic;
                }
                if cell.underline() {
                    style.underline = Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(fg),
                        wavy: false,
                    });
                }
                let line = window.text_system().shape_line(
                    cell.contents().to_string().into(),
                    px(font_size),
                    &[style],
                    None,
                );
                result.text.push(DrawText {
                    line,
                    wrapped: None,
                    origin,
                    height,
                });
            }
        }
        if app.pane == Pane::Terminal && !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            let mut color = accent;
            color.a = 0.5;
            result.cursor = Some(fill(
                Bounds::new(
                    point(
                        bounds.left() + px(8.) + width * col as f32,
                        bounds.top() + height * row as f32,
                    ),
                    size(width, height),
                ),
                color,
            ));
        }
    }
}
fn terminal_color(color: vt100::Color, default: Hsla) -> Hsla {
    let value = match color {
        vt100::Color::Default => return default,
        vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        vt100::Color::Idx(i) => {
            const ANSI: [u32; 16] = [
                0x20242b, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdce3ec,
                0x687385, 0xff8590, 0xb4e68e, 0xffd990, 0x89c8ff, 0xdda0ff, 0x85dbe5, 0xffffff,
            ];
            if i < 16 {
                ANSI[i as usize]
            } else if i >= 232 {
                let c = 8 + (i as u32 - 232) * 10;
                c << 16 | c << 8 | c
            } else {
                let n = i as u32 - 16;
                let levels = [0, 95, 135, 175, 215, 255];
                levels[(n / 36) as usize] << 16
                    | levels[((n / 6) % 6) as usize] << 8
                    | levels[(n % 6) as usize]
            }
        }
    };
    rgb(value).into()
}
