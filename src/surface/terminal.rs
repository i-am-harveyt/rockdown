use super::text::run;
use super::{DrawText, Prepared, Surface};
use crate::app::Pane;
use gpui::*;

impl Surface {
    pub(super) fn terminal(
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
