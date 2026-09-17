use super::text::{SpanStyle, run};
use super::{DrawText, HitRow, LinkHit, Prepared, Surface};
use crate::app::{Pane, Workspace};
use crate::markdown::{RenderedLine, TableRow};
use gpui::*;
use pulldown_cmark::Alignment;
use std::collections::HashMap;

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
pub(super) struct TableLayout {
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
                                let (text, runs) = style.text(spans, style.foreground);
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

impl Surface {
    /// Paint one physical table row as a cell grid. Column widths are shared by
    /// the whole table so boundaries stay stable while scrolling; each cell wraps
    /// independently and clips to its own column.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn table_row(
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
            let mut span_start = 0;
            for span in &table.cells[column] {
                let span_end = span_start + span.text.len();
                if let Some(url) = &span.link {
                    let starts = std::iter::once(0).chain(cell.wrapped.iter().flat_map(|line| {
                        line.wrap_boundaries.iter().map(|boundary| {
                            line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
                                .index
                        })
                    }));
                    let mut starts = starts.peekable();
                    let mut visual_row = 0;
                    while let Some(start) = starts.next() {
                        let end = starts.peek().copied().unwrap_or(cell.text.len());
                        let first = span_start.max(start);
                        let last = span_end.min(end);
                        if first < last {
                            let left = cell.line.x_for_index(start);
                            let width = cell.line.x_for_index(end) - left;
                            let shift = if cell.wrapped.is_some() {
                                match align {
                                    TextAlign::Left => px(0.),
                                    TextAlign::Center => (clip.size.width - width) / 2.,
                                    TextAlign::Right => clip.size.width - width,
                                }
                            } else {
                                px(0.)
                            };
                            let hit = Bounds::new(
                                origin
                                    + point(
                                        shift + cell.line.x_for_index(first) - left,
                                        px(line_height) * visual_row as f32,
                                    ),
                                size(
                                    cell.line.x_for_index(last) - cell.line.x_for_index(first),
                                    px(line_height),
                                ),
                            )
                            .intersect(&clip)
                            .intersect(&bounds);
                            if hit.size.width > px(0.) && hit.size.height > px(0.) {
                                result.layout.links.push(LinkHit {
                                    bounds: hit,
                                    url: url.clone(),
                                });
                            }
                        }
                        visual_row += 1;
                    }
                }
                span_start = span_end;
            }
            result.text.push(DrawText {
                line: cell.line.clone(),
                wrapped: cell.wrapped.clone(),
                origin,
                height: px(line_height),
                clip: Some(clip),
                align,
            });
        }
        if self.pane == Pane::Editor && app.config.markdown_line_numbers {
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
            wrapped: None,
            body_start: 0,
            line_height: px(line_height),
            height,
        });
        height
    }
}
