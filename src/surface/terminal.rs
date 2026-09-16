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
        let palette = app.config.theme.ansi_palette();
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
                let mut fg = terminal_color(cell.fgcolor(), foreground, &palette);
                let mut bg = terminal_color(cell.bgcolor(), background, &palette);
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

fn terminal_color(color: vt100::Color, default: Hsla, palette: &[u32; 16]) -> Hsla {
    let value = match color {
        vt100::Color::Default => return default,
        vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        vt100::Color::Idx(i) => {
            if i < 16 {
                palette[i as usize]
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

#[cfg(test)]
mod tests {
    use super::terminal_color;
    use crate::config::{Config, Theme, ThemePreset};
    use gpui::{Hsla, hsla, rgb};

    fn custom_theme() -> Theme {
        let directory = tempfile::tempdir().unwrap();
        let colorschemes = directory.path().join("colorscheme");
        std::fs::create_dir(&colorschemes).unwrap();
        let colors: Vec<_> = (0..16)
            .map(|index| format!("'#{:06x}'", 0x123450 + index))
            .collect();
        std::fs::write(
            colorschemes.join("custom.toml"),
            format!("[terminal]\nansi = [{}]", colors.join(", ")),
        )
        .unwrap();
        Config::parse(
            &directory.path().join("config.toml"),
            "[theme]\npreset = 'custom'",
        )
        .unwrap()
        .theme
    }

    #[test]
    fn custom_ansi_colors_reach_terminal_cells_on_either_background() {
        let mut parser = vt100::Parser::new(1, 16, 0);
        for index in 0..16 {
            parser.process(format!("\x1b[38;5;{index};48;5;{}mX", 15 - index).as_bytes());
        }
        let mut theme = custom_theme();
        for background in ["#171b22", "#faf9f6"] {
            theme.background = background.into();
            let palette = theme.ansi_palette();
            for index in 0..16 {
                let cell = parser.screen().cell(0, index).unwrap();
                let foreground: Hsla = rgb(0x123450 + u32::from(index)).into();
                let background: Hsla = rgb(0x123450 + u32::from(15 - index)).into();
                assert_eq!(
                    terminal_color(cell.fgcolor(), Hsla::default(), &palette),
                    foreground
                );
                assert_eq!(
                    terminal_color(cell.bgcolor(), Hsla::default(), &palette),
                    background
                );
            }
        }
    }

    #[test]
    fn ansi_colors_follow_each_preset_and_background_override() {
        for background in ["#171b22", "#faf9f6"] {
            let mut reds = Vec::new();
            for &preset in ThemePreset::ALL {
                let mut theme = preset.theme();
                theme.background = background.into();
                let palette = theme.ansi_palette();
                let red = terminal_color(vt100::Color::Idx(1), Hsla::default(), &palette);
                assert!(
                    !reds.contains(&red),
                    "{preset:?} should have its own palette"
                );
                reds.push(red);

                let dark = theme.is_dark();
                theme.background = if dark { "#faf9f6" } else { "#171b22" }.into();
                let alternate = theme.ansi_palette();
                for index in [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14] {
                    let original =
                        terminal_color(vt100::Color::Idx(index), Hsla::default(), &palette);
                    let changed =
                        terminal_color(vt100::Color::Idx(index), Hsla::default(), &alternate);
                    assert!(
                        if dark {
                            original.l > changed.l
                        } else {
                            original.l < changed.l
                        },
                        "{preset:?} ANSI {index} should adapt to background darkness"
                    );
                }
            }
        }
    }

    #[test]
    fn themes_preserve_default_truecolor_and_extended_indexes() {
        let default = hsla(0.23, 0.45, 0.67, 0.8);
        let indexed = [
            (16, 0x000000),
            (17, 0x00005f),
            (21, 0x0000ff),
            (51, 0x00ffff),
            (52, 0x5f0000),
            (196, 0xff0000),
            (231, 0xffffff),
            (232, 0x080808),
            (255, 0xeeeeee),
        ];
        for theme in ThemePreset::ALL
            .iter()
            .map(|preset| preset.theme())
            .chain([custom_theme()])
        {
            for background in ["#171b22", "#faf9f6"] {
                let mut theme = theme.clone();
                theme.background = background.into();
                let palette = theme.ansi_palette();
                assert_eq!(
                    terminal_color(vt100::Color::Default, default, &palette),
                    default
                );
                for (r, g, b) in [(0, 0, 0), (18, 52, 86), (255, 128, 1), (255, 255, 255)] {
                    let expected: Hsla = rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into();
                    assert_eq!(
                        terminal_color(vt100::Color::Rgb(r, g, b), default, &palette),
                        expected
                    );
                }
                for (index, value) in indexed {
                    let expected: Hsla = rgb(value).into();
                    assert_eq!(
                        terminal_color(vt100::Color::Idx(index), default, &palette),
                        expected
                    );
                }
            }
        }
    }
}
