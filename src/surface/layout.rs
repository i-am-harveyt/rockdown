use super::images::{begin_layout, load_image, track_visible};
use super::table::TableLayout;
use super::text::{SpanStyle, run};
use super::{DrawText, HitRow, Prepared, Surface};
use crate::app::Pane;
use crate::image_assets::local_image_path;
use crate::markdown::BlockKind;
use crate::vim::Mode;
use gpui::*;
use std::collections::HashMap;

/// Default inline-image width as a share of the editor pane. Overridable per
/// image with the title syntax `![alt](path "40%")` or `"320px"`.
const DEFAULT_IMAGE_WIDTH: f32 = 0.6;

impl HitRow {
    fn starts(&self) -> Vec<usize> {
        std::iter::once(0)
            .chain(self.wrapped.iter().flat_map(|line| {
                line.wrap_boundaries.iter().map(|boundary| {
                    self.body_start
                        + line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
                            .index
                })
            }))
            .collect()
    }

    pub fn position_for_index(&self, index: usize) -> Point<Pixels> {
        if let Some(wrapped) = &self.wrapped {
            let layout = &wrapped.unwrapped_layout;
            let boundary_index = |boundary: &WrapBoundary| {
                layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
            };
            let visual_row = wrapped
                .wrap_boundaries
                .partition_point(|boundary| self.body_start + boundary_index(boundary) <= index);
            if visual_row > 0 {
                let start = boundary_index(&wrapped.wrap_boundaries[visual_row - 1]);
                return self.origin
                    + point(
                        self.line.x_for_index(self.body_start)
                            + layout.x_for_index(index - self.body_start)
                            - layout.x_for_index(start),
                        self.line_height * visual_row as f32,
                    );
            }
        }
        self.origin + point(self.line.x_for_index(index), px(0.))
    }

    pub fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if let Some(line) = &self.wrapped {
            let mut local = position - self.origin;
            let indent = self.line.x_for_index(self.body_start);
            if local.y < self.line_height && local.x < indent {
                return self.line.closest_index_for_x(local.x).min(self.body_start);
            }
            local.x -= indent;
            local.y = local
                .y
                .max(px(0.))
                .min(self.line_height * (line.wrap_boundaries.len() as f32 + 0.99));
            self.body_start
                + line
                    .closest_index_for_position(local, self.line_height)
                    .unwrap_or_else(|index| index)
        } else {
            self.line.closest_index_for_x(position.x - self.origin.x)
        }
    }
}

impl Surface {
    pub(super) fn prepare(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return Prepared::default();
        }
        begin_layout(&self.workspace, self.pane, cx);
        loop {
            let mut result = Prepared::default();
            let line_height = self.workspace.read(cx).config.line_height;
            let visible = (f32::from(bounds.size.height) / line_height)
                .floor()
                .max(1.) as usize;
            self.workspace.update(cx, |app, _| {
                app.ensure_cursor_visible(self.pane, f32::from(bounds.size.height))
            });
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
            let markdown = &app.config.markdown;
            let color = |value: &Option<String>| value.as_deref().map(|hex| app.color(hex));
            let divider_color = color(&markdown.divider.color).unwrap_or(muted);
            let divider_thickness = px(markdown.divider.thickness);
            let buffer = if self.pane == Pane::Editor {
                &app.documents.current().buffer
            } else {
                &app.explorer.buffer
            };
            let top = app.tops[self.pane.index()];
            let show_line_numbers = self.pane == Pane::Editor
                && (!app.documents.current().is_markdown() || app.config.markdown_line_numbers);
            let gutter = if show_line_numbers {
                56.
            } else if self.pane == Pane::Editor {
                24.
            } else {
                16.
            };
            let available = (bounds.size.width - px(gutter + 24.)).max(px(1.));
            result.layout.text_width = available;
            let style = SpanStyle {
                font: font.clone(),
                font_size: px(app.config.font_size),
                foreground: color(&markdown.colors.normal).unwrap_or(foreground),
                muted,
                panel,
                bold: color(&markdown.colors.bold),
                italic: color(&markdown.colors.italic),
                bold_italic: color(&markdown.colors.bold_italic),
                code: color(&markdown.colors.code).unwrap_or(accent),
                link: color(&markdown.colors.link).unwrap_or(accent),
                strikethrough: Some(color(&markdown.colors.strikethrough).unwrap_or(muted)),
                quote: Some(color(&markdown.colors.quote).unwrap_or(muted)),
            };
            let mut tables: HashMap<usize, TableLayout> = HashMap::new();
            let mut y = bounds.top() - px(app.scroll_offsets[self.pane.index()]);
            let mut images = Vec::new();
            for source_row in top..(top + visible + 1).min(buffer.lines.len()) {
                let active = source_row == buffer.row && app.pane == self.pane;
                let raw = active
                    || self.pane == Pane::Explorer
                    || app.projection.is_empty()
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
                let mut text_line_height = line_height;
                let mut heading_underline = false;
                let mut runs = Vec::new();
                let text = if raw {
                    let text = buffer.lines[source_row].text.clone();
                    runs.push(run(
                        &text,
                        font.clone(),
                        if self.pane == Pane::Editor {
                            style.foreground
                        } else {
                            foreground
                        },
                    ));
                    text
                } else {
                    let projection = &app.projection[source_row];
                    let mut base = style.foreground;
                    match projection.kind {
                        BlockKind::Heading(level) => {
                            let heading = markdown.heading(level);
                            font_size = heading.font_size.unwrap_or_else(|| {
                                (app.config.font_size + (7 - level) as f32 * 1.5)
                                    .min(line_height - 3.)
                            });
                            if heading.font_size.is_some() {
                                text_line_height = line_height.max(font_size + 4.);
                            }
                            base = color(&heading.color).unwrap_or(accent);
                            heading_underline = heading.underline;
                        }
                        BlockKind::Quote => {
                            base = style.quote.unwrap_or(base);
                            result.quads.push(fill(
                                Bounds::new(
                                    point(bounds.left() + px(gutter - 9.), y + px(3.)),
                                    size(px(2.), px(line_height - 6.)),
                                ),
                                accent,
                            ));
                        }
                        BlockKind::Rule => result.quads.push(fill(
                            Bounds::new(
                                point(bounds.left() + px(gutter), y + px(line_height / 2.)),
                                size(
                                    (bounds.size.width - px(gutter + 16.)).max(px(0.)),
                                    divider_thickness,
                                ),
                            ),
                            divider_color,
                        )),
                        _ => {}
                    }
                    let (built, span_runs) = style.text(&projection.spans, base);
                    runs = span_runs;
                    built
                };
                let line = window
                    .text_system()
                    .shape_line(text.into(), px(font_size), &runs, None);
                let wrap = !raw
                    || (self.pane == Pane::Editor
                        && app
                            .projection
                            .get(source_row)
                            .is_some_and(|row| row.table.is_none() && row.kind != BlockKind::Code));
                let body_start = if wrap {
                    app.projection
                        .get(source_row)
                        .and_then(|row| row.list_content)
                        .map_or(0, |(source, display)| if raw { source } else { display })
                        .min(line.text.len())
                } else {
                    0
                };
                let (wrapped, body_start) = if wrap {
                    super::wrap_body(&line, &runs, available, body_start, window).unwrap_or_else(
                        |error| {
                            eprintln!("Wrapping text: {error}");
                            (None, 0)
                        },
                    )
                } else {
                    (None, 0)
                };
                let mut row_height = px(text_line_height)
                    * (wrapped
                        .as_ref()
                        .map_or(1, |line| line.wrap_boundaries.len() + 1)
                        as f32);
                if !raw && app.projection[source_row].kind == BlockKind::Code {
                    result.quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + px(gutter - 6.), y),
                            size((bounds.size.width - px(gutter)).max(px(0.)), row_height),
                        ),
                        panel,
                    ));
                }
                if heading_underline {
                    result.quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + px(gutter), y + row_height + px(2.)),
                            size(
                                (bounds.size.width - px(gutter + 16.)).max(px(0.)),
                                divider_thickness,
                            ),
                        ),
                        divider_color,
                    ));
                    row_height += divider_thickness + px(4.);
                }
                let caret = line.x_for_index(buffer.col.min(line.text.len()));
                let shift = if active && !wrap && caret > available {
                    caret - available
                } else {
                    px(0.)
                };
                let origin = point(bounds.left() + px(gutter) - shift, y);
                let hit = HitRow {
                    source_row,
                    origin,
                    line: line.clone(),
                    raw,
                    wrapped: wrapped.clone(),
                    body_start,
                    line_height: px(text_line_height),
                    height: row_height,
                };
                if active {
                    let mut highlight = accent;
                    highlight.a = 0.055;
                    result.quads.push(fill(
                        Bounds::new(point(bounds.left(), y), size(bounds.size.width, row_height)),
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
                            hit.position_for_index(buffer.col) + point(px(0.), px(3.)),
                            size(width, px(line_height - 6.)),
                        ),
                        cursor_color,
                    ));
                }
                if let Some(range) = buffer.selected_range(source_row) {
                    let mut color = accent;
                    color.a = 0.25;
                    if buffer.mode == Mode::VisualLine {
                        result.quads.push(fill(
                            Bounds::new(
                                point(bounds.left() + px(gutter), y),
                                size((bounds.size.width - px(gutter)).max(px(0.)), row_height),
                            ),
                            color,
                        ));
                    } else {
                        let starts = hit.starts();
                        for (visual_row, start) in starts.iter().copied().enumerate() {
                            let end = starts
                                .get(visual_row + 1)
                                .copied()
                                .unwrap_or(line.text.len());
                            let first = range.start.max(start);
                            let last = range.end.min(end);
                            if first < last {
                                let left = hit.position_for_index(first).x - origin.x;
                                let right = left + line.x_for_index(last) - line.x_for_index(first);
                                result.quads.push(fill(
                                    Bounds::new(
                                        origin
                                            + point(left, px(text_line_height) * visual_row as f32),
                                        size((right - left).max(px(4.)), px(text_line_height)),
                                    ),
                                    color,
                                ));
                            }
                        }
                    }
                }
                if show_line_numbers {
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
                        let Some(path) = local_image_path(base, &image.url) else {
                            continue;
                        };
                        track_visible(self.workspace.entity_id(), &path, cx);
                        let bitmap = match load_image(&path, cx) {
                            Ok(bitmap) => bitmap,
                            Err(message) => {
                                let label = format!("{message}: {}", image.url);
                                let runs = [run(&label, font.clone(), muted)];
                                let shaped = window.text_system().shape_line(
                                    label.into(),
                                    px(app.config.font_size),
                                    &runs,
                                    None,
                                );
                                let wrapped = window
                                    .text_system()
                                    .shape_text(
                                        shaped.text.clone(),
                                        px(app.config.font_size),
                                        &runs,
                                        Some(available),
                                        None,
                                    )
                                    .map(|mut lines| lines.pop())
                                    .unwrap_or_else(|error| {
                                        eprintln!("Wrapping image error: {error}");
                                        None
                                    });
                                let height = px(line_height)
                                    * wrapped
                                        .as_ref()
                                        .map_or(1, |line| line.wrap_boundaries.len() + 1)
                                        as f32;
                                result.text.push(DrawText {
                                    line: shaped,
                                    wrapped,
                                    origin: point(origin.x, y + row_height),
                                    height: px(line_height),
                                    clip: None,
                                    align: TextAlign::Left,
                                });
                                row_height += height;
                                continue;
                            }
                        };
                        // Width-driven sizing: default 60% of the pane width, with a
                        // per-image override from the title (`![alt](pic.png "40%")`
                        // or `"320px"`). Height follows the bitmap's aspect ratio,
                        // clamped so a wide image never exceeds the pane.
                        let bitmap_size = bitmap.size(0);
                        let ratio = bitmap_size.width.0 as f32 / bitmap_size.height.0 as f32;
                        let width = match image.width {
                            Some(crate::markdown::ImageWidth::Fraction(share)) => available * share,
                            Some(crate::markdown::ImageWidth::Points(points)) => px(points),
                            None => available * DEFAULT_IMAGE_WIDTH,
                        }
                        .min(available);
                        let height = width / ratio;
                        images.push((
                            Bounds::new(
                                point(origin.x + (available - width) / 2., y + row_height),
                                size(width, height),
                            ),
                            bitmap,
                        ));
                        row_height += height;
                    }
                }
                result.layout.rows.push(HitRow {
                    height: row_height,
                    ..hit
                });
                let mut text_origin = origin;
                if wrapped.is_some() && body_start > 0 {
                    let mut remaining = body_start;
                    let prefix_runs: Vec<_> = runs
                        .iter()
                        .filter_map(|run| {
                            let len = remaining.min(run.len);
                            remaining -= len;
                            (len > 0).then(|| TextRun { len, ..run.clone() })
                        })
                        .collect();
                    result.text.push(DrawText {
                        line: window.text_system().shape_line(
                            line.text[..body_start].to_owned().into(),
                            px(font_size),
                            &prefix_runs,
                            None,
                        ),
                        wrapped: None,
                        origin,
                        height: px(text_line_height),
                        clip: None,
                        align: TextAlign::Left,
                    });
                    text_origin.x += line.x_for_index(body_start);
                }
                result.text.push(DrawText {
                    line,
                    wrapped,
                    origin: text_origin,
                    height: px(text_line_height),
                    clip: None,
                    align: TextAlign::Left,
                });
                y += row_height;
            }
            if app.follow_cursor
                && app.pane == self.pane
                && !app.viewport_is_aligned(self.pane)
                && let Some(cursor) = &result.cursor
            {
                let delta = if cursor.bounds.bottom() > bounds.bottom() {
                    cursor.bounds.bottom() - bounds.bottom()
                } else if cursor.bounds.top() < bounds.top() {
                    cursor.bounds.top() - bounds.top()
                } else {
                    px(0.)
                };
                if delta.abs() > px(0.5) {
                    self.workspace.update(cx, |app, _| {
                        let offset = &mut app.scroll_offsets[self.pane.index()];
                        *offset += f32::from(delta);
                    });
                    continue;
                }
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
                continue;
            }
            // Restored offsets were measured at a potentially different window
            // width or document version. Validate against the real first row,
            // including wrapping and images, rather than a guessed line height.
            if self.pane == Pane::Editor && app.viewport_needs_measurement {
                let height = result.layout.rows.first().map(|row| f32::from(row.height));
                let realign = self.workspace.update(cx, |app, _| {
                    app.viewport_needs_measurement = false;
                    let offset = &mut app.scroll_offsets[Pane::Editor.index()];
                    if height.is_some_and(|height| *offset >= height) {
                        *offset = 0.;
                        true
                    } else {
                        false
                    }
                });
                if realign {
                    continue;
                }
            }
            result.images = images;
            let realign = self.workspace.update(cx, |app, _| {
                if !app.viewport_is_aligned(self.pane) {
                    return false;
                }
                let index = self.pane.index();
                let previous = (app.tops[index], app.scroll_offsets[index]);
                for row in &result.layout.rows {
                    app.row_heights[index][row.source_row] = f32::from(row.height);
                }
                app.ensure_cursor_visible(self.pane, f32::from(bounds.size.height));
                previous.0 != app.tops[index]
                    || (previous.1 - app.scroll_offsets[index]).abs() > 0.5
            });
            if realign {
                continue;
            }
            return result;
        }
    }
}
