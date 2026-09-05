use crate::app::{Pane, Workspace};
use crate::{
    markdown::{BlockKind, RenderedLine, Span, TableRow},
    vim::Mode,
};
use gpui::{prelude::*, *};
use pulldown_cmark::Alignment;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

/// Rendered height of an inline image, in pixels.
const IMAGE_HEIGHT: Pixels = px(160.);

// Decoded bitmaps by path. Layout positions are computed by the renderer, so
// an image can never escape its row and paint over following content; the
// cache only saves repeated decoding of the same file.
static IMAGES: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<RenderImage>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn load_image(path: &Path) -> Option<Arc<RenderImage>> {
    let mut cache = IMAGES.lock().ok()?;
    if !cache.contains_key(path) {
        let decoded = std::fs::read(path)
            .ok()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|image| {
                let mut rgba = image.into_rgba8();
                // The sprite atlas expects BGRA bytes.
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
            });
        cache.insert(path.to_path_buf(), decoded);
    }
    cache.get(path).cloned().flatten()
}

#[derive(Default)]
pub struct SurfaceLayout {
    pub rows: Vec<HitRow>,
}
pub struct HitRow {
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
    clip: Option<Bounds<Pixels>>,
    align: TextAlign,
}
#[derive(Default)]
pub struct Prepared {
    layout: SurfaceLayout,
    text: Vec<DrawText>,
    quads: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    images: Vec<(Bounds<Pixels>, Arc<RenderImage>)>,
}
pub struct Surface {
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

struct SpanStyle {
    font: Font,
    font_size: Pixels,
    foreground: Hsla,
    muted: Hsla,
    accent: Hsla,
    panel: Hsla,
}

impl SpanStyle {
    fn text(&self, spans: &[Span]) -> (String, Vec<TextRun>) {
        let mut text = String::new();
        let mut runs = Vec::with_capacity(spans.len());
        for span in spans {
            text.push_str(&span.text);
            let mut style = run(
                &span.text,
                self.font.clone(),
                if span.link.is_some() {
                    self.accent
                } else {
                    self.foreground
                },
            );
            if span.bold {
                style.font.weight = FontWeight::BOLD;
            }
            if span.italic {
                style.font.style = FontStyle::Italic;
            }
            if span.code {
                style.background_color = Some(self.panel);
                style.color = self.accent;
            }
            if span.link.is_some() {
                style.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(self.accent),
                    wavy: false,
                });
            }
            if span.strike {
                style.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(self.muted),
                });
            }
            runs.push(style);
        }
        (text, runs)
    }
}

struct TableCell {
    text: SharedString,
    line: ShapedLine,
    runs: Vec<TextRun>,
    wrapped: Option<WrappedLine>,
}

// Cap large columns at a shared fair width rather than letting a long cell
// squeeze every short column. Surplus space is shared when everything fits.
fn column_widths(natural: &[Pixels], available: Pixels) -> Vec<Pixels> {
    if natural.is_empty() {
        return Vec::new();
    }
    let total = natural
        .iter()
        .copied()
        .fold(px(0.), |sum, width| sum + width);
    if total <= available {
        let extra = (available - total) / natural.len() as f32;
        return natural.iter().map(|width| *width + extra).collect();
    }
    let mut sorted = natural.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut remaining = available;
    let mut cap = px(0.);
    for (index, width) in sorted.iter().enumerate() {
        cap = remaining / (sorted.len() - index) as f32;
        if *width >= cap {
            break;
        }
        remaining -= *width;
    }
    natural.iter().map(|width| (*width).min(cap)).collect()
}

fn text_align(alignment: Alignment) -> TextAlign {
    match alignment {
        Alignment::Center => TextAlign::Center,
        Alignment::Right => TextAlign::Right,
        Alignment::Left | Alignment::None => TextAlign::Left,
    }
}
struct TableLayout {
    widths: Vec<Pixels>,
    padding: Pixels,
    border: Pixels,
    rows: Vec<Vec<TableCell>>,
}

impl TableLayout {
    fn new(
        projection: &[RenderedLine],
        table: &TableRow,
        available: Pixels,
        style: &SpanStyle,
        window: &mut Window,
    ) -> Self {
        let count = table.alignments.len();
        // Shrink padding and rules in extremely narrow panes; text is clipped
        // separately so even a glyph wider than its column cannot escape.
        let border = px(1.).min(available / (count + 1) as f32 / 2.);
        let padding = px(6.).min(available / count.max(1) as f32 / 8.);
        let content =
            (available - border * (count + 1) as f32 - padding * (2 * count) as f32).max(px(0.));
        let mut natural = vec![style.font_size * 2.; count];
        let mut rows: Vec<Vec<TableCell>> = projection[table.start..table.end]
            .iter()
            .map(|row| {
                row.table
                    .as_ref()
                    .map(|row| {
                        row.cells
                            .iter()
                            .enumerate()
                            .map(|(column, spans)| {
                                let (text, runs) = style.text(spans);
                                let line = window.text_system().shape_line(
                                    text.clone().into(),
                                    style.font_size,
                                    &runs,
                                    None,
                                );
                                natural[column] = natural[column].max(line.width);
                                TableCell {
                                    text: text.into(),
                                    line,
                                    runs,
                                    wrapped: None,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let widths = column_widths(&natural, content);
        // Wrapping needs the final column widths, so it runs after measuring.
        for row in &mut rows {
            for (column, cell) in row.iter_mut().enumerate() {
                if cell.line.width > widths[column] {
                    cell.wrapped = window
                        .text_system()
                        .shape_text(
                            cell.text.clone(),
                            style.font_size,
                            &cell.runs,
                            Some(widths[column]),
                            None,
                        )
                        .map(|mut lines| lines.pop())
                        .unwrap_or_else(|error| {
                            eprintln!("Wrapping text: {error}");
                            None
                        });
                }
            }
        }
        Self {
            widths,
            padding,
            border,
            rows,
        }
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
        let style = SpanStyle {
            font: font.clone(),
            font_size: px(app.config.font_size),
            foreground,
            muted,
            accent,
            panel,
        };
        let mut tables: HashMap<usize, TableLayout> = HashMap::new();
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
            if !raw && let Some(table) = app.projection[source_row].table.as_ref() {
                y += self.table_row(
                    app,
                    source_row,
                    table,
                    y,
                    bounds,
                    gutter,
                    available,
                    line_height,
                    &mut tables,
                    &style,
                    &mut result,
                    window,
                );
                continue;
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
                    clip: None,
                    align: TextAlign::Left,
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
                    let Some(bitmap) = load_image(&base.join(image)) else {
                        continue;
                    };
                    // Aspect-fit the bitmap into the row: taller images are
                    // capped at IMAGE_HEIGHT so they never cover later rows.
                    let bitmap_size = bitmap.size(0);
                    let ratio = bitmap_size.width.0 as f32 / bitmap_size.height.0 as f32;
                    let mut height = IMAGE_HEIGHT;
                    let mut width = height * ratio;
                    if width > available {
                        width = available;
                        height = width / ratio;
                    }
                    images.push((
                        Bounds::new(point(origin.x, y + row_height), size(width, height)),
                        bitmap,
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
                clip: None,
                align: TextAlign::Left,
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
        result.images = images;
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
                        text.align,
                        text.clip,
                        window,
                        cx,
                    )
                    .and_then(|_| {
                        line.paint(text.origin, text.height, text.align, text.clip, window, cx)
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
            for (bounds, image) in prepared.images.drain(..) {
                let _ = window.paint_image(bounds, Corners::default(), image, 0, false);
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
    /// Paint one physical table row as a cell grid. Column widths are shared by
    /// the whole table so boundaries stay stable while scrolling; each cell wraps
    /// independently and clips to its own column.
    #[allow(clippy::too_many_arguments)]
    fn table_row(
        &self,
        app: &Workspace,
        source_row: usize,
        table: &TableRow,
        y: Pixels,
        bounds: Bounds<Pixels>,
        gutter: f32,
        available: Pixels,
        line_height: f32,
        tables: &mut HashMap<usize, TableLayout>,
        style: &SpanStyle,
        result: &mut Prepared,
        window: &mut Window,
    ) -> Pixels {
        let left = bounds.left() + px(gutter);
        let layout = tables
            .entry(table.start)
            .or_insert_with(|| TableLayout::new(&app.projection, table, available, style, window));
        let index = source_row - table.start;
        let mut edge = left;
        let mut edges = Vec::with_capacity(layout.widths.len() + 1);
        for width in &layout.widths {
            edges.push(edge);
            edge += layout.border + 2. * layout.padding + *width + layout.border;
        }
        edges.push(edge);
        let width = edge + layout.border - left;
        let row = &layout.rows[index];
        let height = if row.is_empty() {
            px(line_height)
        } else {
            row.iter()
                .map(|cell| {
                    px(line_height)
                        * (cell
                            .wrapped
                            .as_ref()
                            .map_or(1, |line| line.wrap_boundaries.len() + 1)
                            as f32)
                })
                .fold(px(line_height), Pixels::max)
        };
        // Vertical rules run the full row height; the separator row carries the
        // horizontal rule, and the header row gains a background plus underline.
        for (x, e) in edges.iter().enumerate() {
            result.quads.push(fill(
                Bounds::new(point(*e, y), size(layout.border, height)),
                style.muted,
            ));
            if x + 1 < edges.len() {
                let width = layout.widths[x];
                let cell = Bounds::new(
                    point(*e + layout.border, y),
                    size(2. * layout.padding + width, height),
                );
                if row.is_empty() {
                    result.quads.push(fill(
                        Bounds::new(
                            point(*e, y + height / 2.),
                            size(
                                (layout.border + 2. * layout.padding + width + layout.border)
                                    .max(px(0.)),
                                layout.border,
                            ),
                        ),
                        style.muted,
                    ));
                } else if table.header {
                    result.quads.push(fill(cell, style.muted));
                }
            }
        }
        if table.header {
            result.quads.push(fill(
                Bounds::new(point(left, y), size(width, height + layout.border)),
                style.muted,
            ));
        }
        for (column, cell) in row.iter().enumerate() {
            let inner = layout.widths[column];
            let clip = Bounds::new(
                point(edges[column] + layout.border, y),
                size(2. * layout.padding + inner, height),
            );
            let align = table
                .alignments
                .get(column)
                .copied()
                .map_or(TextAlign::Left, text_align);
            let origin = point(
                clip.origin.x
                    + layout.padding
                    + if cell.wrapped.is_none() {
                        match align {
                            TextAlign::Left => px(0.),
                            TextAlign::Center => ((inner - cell.line.width) / 2.).max(px(0.)),
                            TextAlign::Right => (inner - cell.line.width).max(px(0.)),
                        }
                    } else {
                        px(0.)
                    },
                y,
            );
            result.text.push(DrawText {
                line: cell.line.clone(),
                wrapped: cell.wrapped.clone(),
                origin,
                height: px(line_height),
                clip: Some(clip),
                align,
            });
        }
        if self.pane == Pane::Editor {
            let number = format!("{:>4}", source_row + 1);
            let shaped = window.text_system().shape_line(
                number.clone().into(),
                px(app.config.font_size - 2.),
                &[run(&number, style.font.clone(), style.muted)],
                None,
            );
            result.text.push(DrawText {
                line: shaped,
                wrapped: None,
                origin: point(bounds.left() + px(4.), y),
                height: px(line_height),
                clip: None,
                align: TextAlign::Left,
            });
        }
        result.layout.rows.push(HitRow {
            source_row,
            origin: point(left, y),
            line: window
                .text_system()
                .shape_line("".into(), style.font_size, &[], None),
            raw: false,
            height,
        });
        height
    }

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
                    clip: None,
                    align: TextAlign::Left,
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
