use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub explorer_width: f32,
    /// Maximum Markdown writing-column width, including its gutters.
    pub writing_width: f32,
    /// Show source line numbers in Markdown documents.
    pub markdown_line_numbers: bool,
    pub tab_bar_visible: bool,
    pub status_bar_visible: bool,
    /// Portable document-relative directory for imported image files.
    pub image_assets_dir: String,
    pub terminal_height: f32,
    pub shell: String,
    pub theme: Theme,
    pub markdown: MarkdownStyle,
    pub keys: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreset {
    #[default]
    Rockdown,
    Paper,
}

impl ThemePreset {
    pub const ALL: &'static [Self] = &[Self::Rockdown, Self::Paper];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rockdown => "Rockdown",
            Self::Paper => "Paper",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Rockdown => "rockdown",
            Self::Paper => "paper",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|preset| preset.id() == id)
    }

    pub fn theme(self) -> Theme {
        let [background, panel, foreground, muted, accent] = match self {
            Self::Rockdown => ["#171b22", "#1e242e", "#dce3ec", "#8995a7", "#9cc7b5"],
            Self::Paper => ["#faf9f6", "#eeede9", "#292929", "#6b6b67", "#356d61"],
        };
        Theme {
            preset: self.id().into(),
            name: self.name().into(),
            background: background.into(),
            panel: panel.into(),
            foreground: foreground.into(),
            muted: muted.into(),
            accent: accent.into(),
            base: self,
            syntax: [None; 10],
            ansi: None,
            markdown: ThemeMarkdown::default(),
        }
    }

    fn syntax_palette(self, dark: bool) -> [u32; 10] {
        match (self, dark) {
            (Self::Rockdown, true) => [
                0xdce3ec, 0x8995a7, 0xe09a9a, 0xd8af85, 0xd8cd9b, 0x9cc7b5, 0x95c5ca, 0x9cbbe0,
                0xc3a8dc, 0xc5a390,
            ],
            (Self::Rockdown, false) => [
                0x29313c, 0x586779, 0x983c46, 0x895322, 0x766018, 0x356d61, 0x286b72, 0x365f91,
                0x72508e, 0x79513c,
            ],
            (Self::Paper, true) => [
                0xe8e6df, 0xa8a69e, 0xd99286, 0xd3ac7f, 0xc9bf89, 0x94b8a2, 0x8db8b4, 0x94adc7,
                0xb3a0bd, 0xbea18c,
            ],
            (Self::Paper, false) => [
                0x292929, 0x6b6b67, 0x983f35, 0x825724, 0x756323, 0x356d61, 0x326a68, 0x3f6086,
                0x75547e, 0x78583e,
            ],
        }
    }

    fn ansi_palette(self, dark: bool) -> [u32; 16] {
        match (self, dark) {
            (Self::Rockdown, true) => [
                0x20242b, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdce3ec,
                0x687385, 0xff8590, 0xb4e68e, 0xffd990, 0x89c8ff, 0xdda0ff, 0x85dbe5, 0xffffff,
            ],
            (Self::Rockdown, false) => [
                0x20242b, 0xa83240, 0x426b28, 0x805a12, 0x2465a0, 0x85439b, 0x21727d, 0xdce3ec,
                0x687385, 0xbc3445, 0x477827, 0x90671a, 0x276eae, 0x9449ad, 0x237e89, 0xffffff,
            ],
            (Self::Paper, true) => [
                0x292929, 0xd88078, 0x9db787, 0xd6b66e, 0x8caecb, 0xba98b9, 0x89bcb0, 0xeeede9,
                0x777773, 0xe9968f, 0xb0ca9a, 0xe8c982, 0x9fc2df, 0xcdaacb, 0x9cd0c3, 0xfaf9f6,
            ],
            (Self::Paper, false) => [
                0x292929, 0x9b3b35, 0x4b6b38, 0x84671f, 0x365f85, 0x795177, 0x356d61, 0xeeede9,
                0x6b6b67, 0xad413b, 0x557b3d, 0x927325, 0x3e6e99, 0x8b5c88, 0x3c7e70, 0xfaf9f6,
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub preset: String,
    pub name: String,
    pub background: String,
    pub panel: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    base: ThemePreset,
    syntax: [Option<u32>; 10],
    ansi: Option<[u32; 16]>,
    markdown: ThemeMarkdown,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeOverlay {
    preset: Option<String>,
    background: Option<String>,
    panel: Option<String>,
    foreground: Option<String>,
    muted: Option<String>,
    accent: Option<String>,
}

impl ThemeOverlay {
    fn apply(self, theme: &mut Theme) {
        for (target, value) in [
            (&mut theme.background, self.background),
            (&mut theme.panel, self.panel),
            (&mut theme.foreground, self.foreground),
            (&mut theme.muted, self.muted),
            (&mut theme.accent, self.accent),
        ] {
            if let Some(value) = value {
                *target = value;
            }
        }
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let overlay = ThemeOverlay::deserialize(deserializer)?;
        let id = overlay.preset.as_deref().unwrap_or("rockdown");
        let mut theme = ThemePreset::from_id(id)
            .ok_or_else(|| {
                serde::de::Error::custom("Custom themes require Config::parse or Config::load")
            })?
            .theme();
        overlay.apply(&mut theme);
        Ok(theme)
    }
}

impl Theme {
    pub fn is_dark(&self) -> bool {
        let rgb = parse_color(&self.background).expect("validated theme");
        let linear = |shift: u32| {
            let channel = ((rgb >> shift) & 0xffu32) as f64 / 255.;
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        // Relative luminance where black and white have equal contrast.
        0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0) < 0.179
    }

    /// Foreground, comment, variable, constant, type, string, escape, function,
    /// keyword, and embedded-source colors, in that order.
    pub fn syntax_palette(&self) -> [u32; 10] {
        let inherited = self.base.syntax_palette(self.is_dark());
        std::array::from_fn(|index| self.syntax[index].unwrap_or(inherited[index]))
    }

    pub fn ansi_palette(&self) -> [u32; 16] {
        self.ansi
            .unwrap_or_else(|| self.base.ansi_palette(self.is_dark()))
    }

    /// Replace colors, including clearing old overrides, without changing typography.
    pub fn apply_markdown_colors(&self, markdown: &mut MarkdownStyle) {
        markdown.colors.clone_from(&self.markdown.colors);
        for (heading, color) in [
            (&mut markdown.h1, &self.markdown.h1.color),
            (&mut markdown.h2, &self.markdown.h2.color),
            (&mut markdown.h3, &self.markdown.h3.color),
            (&mut markdown.h4, &self.markdown.h4.color),
            (&mut markdown.h5, &self.markdown.h5.color),
            (&mut markdown.h6, &self.markdown.h6.color),
        ] {
            heading.color.clone_from(color);
        }
        markdown
            .divider
            .color
            .clone_from(&self.markdown.divider.color);
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeFile {
    name: Option<String>,
    base: ThemePreset,
    theme: ThemeOverlay,
    syntax: SyntaxColors,
    terminal: TerminalColors,
    markdown: ThemeMarkdown,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
struct PaletteColor(u32);

impl TryFrom<String> for PaletteColor {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        parse_color(&value).map(Self)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SyntaxColors {
    foreground: Option<PaletteColor>,
    comment: Option<PaletteColor>,
    variable: Option<PaletteColor>,
    constant: Option<PaletteColor>,
    #[serde(rename = "type")]
    type_color: Option<PaletteColor>,
    string: Option<PaletteColor>,
    escape: Option<PaletteColor>,
    function: Option<PaletteColor>,
    keyword: Option<PaletteColor>,
    embedded: Option<PaletteColor>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TerminalColors {
    ansi: Option<Vec<PaletteColor>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct ThemeMarkdown {
    colors: MarkdownColors,
    h1: MarkdownColor,
    h2: MarkdownColor,
    h3: MarkdownColor,
    h4: MarkdownColor,
    h5: MarkdownColor,
    h6: MarkdownColor,
    divider: MarkdownColor,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct MarkdownColor {
    color: Option<String>,
}

impl ThemeFile {
    fn load(path: &Path, id: &str) -> Result<Theme> {
        let load = || -> Result<Theme> {
            validate_theme_id(id)?;
            if ThemePreset::from_id(id).is_some() {
                bail!("Colorscheme ID '{id}' is reserved for a built-in theme");
            }
            let text = std::fs::read_to_string(path).context("Reading colorscheme")?;
            let file: Self = toml::from_str(&text)?;
            if file.theme.preset.is_some() {
                bail!("Colorschemes use 'base', not 'theme.preset'");
            }
            let mut theme = file.base.theme();
            theme.preset = id.into();
            theme.name = file.name.unwrap_or_else(|| id.into());
            file.theme.apply(&mut theme);
            let syntax = file.syntax;
            theme.syntax = [
                syntax.foreground,
                syntax.comment,
                syntax.variable,
                syntax.constant,
                syntax.type_color,
                syntax.string,
                syntax.escape,
                syntax.function,
                syntax.keyword,
                syntax.embedded,
            ]
            .map(|color| color.map(|color| color.0));
            theme.ansi = file
                .terminal
                .ansi
                .map(|colors| -> Result<_> {
                    let colors: [PaletteColor; 16] =
                        colors.try_into().map_err(|colors: Vec<_>| {
                            anyhow::anyhow!(
                                "terminal.ansi requires exactly 16 colors, got {}",
                                colors.len()
                            )
                        })?;
                    Ok(colors.map(|color| color.0))
                })
                .transpose()?;
            theme.markdown = file.markdown;
            for (name, color) in [
                ("theme.background", &theme.background),
                ("theme.panel", &theme.panel),
                ("theme.foreground", &theme.foreground),
                ("theme.muted", &theme.muted),
                ("theme.accent", &theme.accent),
            ] {
                parse_color(color).with_context(|| name)?;
            }
            let colors = &theme.markdown.colors;
            for (name, color) in [
                ("markdown.colors.normal", &colors.normal),
                ("markdown.colors.bold", &colors.bold),
                ("markdown.colors.italic", &colors.italic),
                ("markdown.colors.bold_italic", &colors.bold_italic),
                ("markdown.colors.code", &colors.code),
                ("markdown.colors.link", &colors.link),
                ("markdown.colors.strikethrough", &colors.strikethrough),
                ("markdown.colors.quote", &colors.quote),
                ("markdown.h1.color", &theme.markdown.h1.color),
                ("markdown.h2.color", &theme.markdown.h2.color),
                ("markdown.h3.color", &theme.markdown.h3.color),
                ("markdown.h4.color", &theme.markdown.h4.color),
                ("markdown.h5.color", &theme.markdown.h5.color),
                ("markdown.h6.color", &theme.markdown.h6.color),
                ("markdown.divider.color", &theme.markdown.divider.color),
            ] {
                if let Some(color) = color {
                    parse_color(color).with_context(|| name)?;
                }
            }
            Ok(theme)
        };
        load().with_context(|| format!("Loading colorscheme {}", path.display()))
    }
}

fn validate_theme_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains(['/', '\\', ':'])
        || id.chars().any(char::is_control)
    {
        bail!("Invalid colorscheme ID '{id}': expected a filename stem");
    }
    Ok(())
}

fn colorscheme_directory(config_path: Option<&Path>) -> Option<PathBuf> {
    match config_path {
        Some(path) => Some(path.parent().unwrap_or(Path::new("")).join("colorscheme")),
        None => {
            config_base(|name| std::env::var_os(name)).map(|base| base.join("rockdown/colorscheme"))
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarkdownStyle {
    pub colors: MarkdownColors,
    pub h1: HeadingStyle,
    pub h2: HeadingStyle,
    pub h3: HeadingStyle,
    pub h4: HeadingStyle,
    pub h5: HeadingStyle,
    pub h6: HeadingStyle,
    pub divider: DividerStyle,
}

impl MarkdownStyle {
    pub fn heading(&self, level: u8) -> &HeadingStyle {
        match level.clamp(1, 6) {
            1 => &self.h1,
            2 => &self.h2,
            3 => &self.h3,
            4 => &self.h4,
            5 => &self.h5,
            _ => &self.h6,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MarkdownColors {
    pub normal: Option<String>,
    pub bold: Option<String>,
    pub italic: Option<String>,
    pub bold_italic: Option<String>,
    pub code: Option<String>,
    pub link: Option<String>,
    pub strikethrough: Option<String>,
    pub quote: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeadingStyle {
    pub font_size: Option<f32>,
    pub color: Option<String>,
    pub underline: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DividerStyle {
    pub color: Option<String>,
    pub thickness: f32,
}

impl Default for DividerStyle {
    fn default() -> Self {
        Self {
            color: None,
            thickness: 1.,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        ThemePreset::default().theme()
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: if cfg!(windows) { "Consolas" } else { "Menlo" }.into(),
            font_size: 15.,
            line_height: 30.,
            explorer_width: 290.,
            writing_width: 820.,
            markdown_line_numbers: false,
            tab_bar_visible: true,
            status_bar_visible: true,
            image_assets_dir: "assets".into(),
            terminal_height: 240.,
            shell: std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" })
                .ok()
                .filter(|shell| !shell.trim().is_empty())
                .unwrap_or_else(|| if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into()),
            theme: Theme::default(),
            markdown: MarkdownStyle::default(),
            keys: [
                ("cmd-n".into(), "new".into()),
                ("ctrl-n".into(), "new".into()),
                ("cmd-o".into(), "open".into()),
                ("ctrl-o".into(), "open".into()),
                ("cmd-shift-s".into(), "save-as".into()),
                ("ctrl-shift-s".into(), "save-as".into()),
                ("cmd-s".into(), "save".into()),
                ("ctrl-s".into(), "save".into()),
                ("cmd-v".into(), "paste".into()),
                ("ctrl-shift-v".into(), "paste".into()),
                (
                    if cfg!(target_os = "macos") {
                        "cmd-z"
                    } else {
                        "ctrl-z"
                    }
                    .into(),
                    "undo".into(),
                ),
                (
                    if cfg!(target_os = "macos") {
                        "cmd-shift-z"
                    } else {
                        "ctrl-shift-z"
                    }
                    .into(),
                    "redo".into(),
                ),
                ("cmd-e".into(), "explorer".into()),
                ("ctrl-e".into(), "explorer".into()),
                ("cmd-shift-t".into(), "themes".into()),
                ("ctrl-shift-t".into(), "themes".into()),
                ("cmd-shift-m".into(), "ui-mode".into()),
                ("ctrl-shift-m".into(), "ui-mode".into()),
                ("cmd-shift-o".into(), "outline".into()),
                ("ctrl-shift-o".into(), "outline".into()),
                ("cmd-alt-t".into(), "tab-bar".into()),
                ("ctrl-alt-t".into(), "tab-bar".into()),
                ("cmd-alt-s".into(), "status-bar".into()),
                ("ctrl-alt-s".into(), "status-bar".into()),
                ("ctrl-`".into(), "terminal".into()),
                ("cmd-1".into(), "editor".into()),
                ("ctrl-1".into(), "editor".into()),
                ("ctrl-pageup".into(), "previous-buffer".into()),
                ("ctrl-pagedown".into(), "next-buffer".into()),
                ("cmd-w".into(), "buffer-delete".into()),
                ("ctrl-shift-w".into(), "buffer-delete".into()),
                ("f1".into(), "help".into()),
            ]
            .into_iter()
            .chain((!cfg!(target_os = "macos")).then(|| ("ctrl-y".into(), "redo".into())))
            .collect(),
        }
    }
}

impl Config {
    /// Only user-level config is auto-loaded.
    pub fn load(explicit: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let path = if let Some(path) = explicit {
            Some(path.to_path_buf())
        } else {
            let base = config_base(|name| std::env::var_os(name)).map(|base| base.join("rockdown"));
            base.and_then(|base| {
                let candidate = base.join("config.toml");
                candidate.is_file().then_some(candidate)
            })
        };
        let config = match &path {
            None => Self::default(),
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("Reading {}", path.display()))?;
                Self::parse(path, &text)?
            }
        };
        config.validate().with_context(|| match &path {
            Some(path) => format!("Loading {}", path.display()),
            None => "Validating default configuration".into(),
        })?;
        Ok((config, path))
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        let parse = || -> Result<Self> {
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                bail!("Configuration must have a .toml extension");
            }
            let mut table: toml::Table = toml::from_str(text)?;
            let overlay: ThemeOverlay = table
                .remove("theme")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            let mut config: Self = toml::Value::Table(table).try_into()?;
            let id = overlay.preset.as_deref().unwrap_or("rockdown");
            validate_theme_id(id)?;
            let mut theme = match ThemePreset::from_id(id) {
                Some(preset) => preset.theme(),
                None => {
                    let directory =
                        colorscheme_directory(Some(path)).expect("explicit config path");
                    ThemeFile::load(&directory.join(format!("{id}.toml")), id)?
                }
            };
            overlay.apply(&mut theme);
            // Config colors override only explicitly supplied values; typography
            // belongs to the config and is never inherited from a colorscheme.
            let colors = &mut config.markdown.colors;
            let inherited = &theme.markdown;
            for (color, fallback) in [
                (&mut colors.normal, &inherited.colors.normal),
                (&mut colors.bold, &inherited.colors.bold),
                (&mut colors.italic, &inherited.colors.italic),
                (&mut colors.bold_italic, &inherited.colors.bold_italic),
                (&mut colors.code, &inherited.colors.code),
                (&mut colors.link, &inherited.colors.link),
                (&mut colors.strikethrough, &inherited.colors.strikethrough),
                (&mut colors.quote, &inherited.colors.quote),
                (&mut config.markdown.h1.color, &inherited.h1.color),
                (&mut config.markdown.h2.color, &inherited.h2.color),
                (&mut config.markdown.h3.color, &inherited.h3.color),
                (&mut config.markdown.h4.color, &inherited.h4.color),
                (&mut config.markdown.h5.color, &inherited.h5.color),
                (&mut config.markdown.h6.color, &inherited.h6.color),
                (&mut config.markdown.divider.color, &inherited.divider.color),
            ] {
                if color.is_none() {
                    color.clone_from(fallback);
                }
            }
            config.theme = theme;
            Ok(config)
        };
        parse().with_context(|| format!("Loading {}", path.display()))
    }

    /// Discover fresh colorschemes next to the config, even if it does not exist.
    pub fn colorschemes(config_path: Option<&Path>) -> Result<Vec<Theme>> {
        let mut themes: Vec<_> = ThemePreset::ALL
            .iter()
            .map(|preset| preset.theme())
            .collect();
        let Some(directory) = colorscheme_directory(config_path) else {
            return Ok(themes);
        };
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(themes),
            Err(error) => {
                return Err(error).with_context(|| format!("Reading {}", directory.display()));
            }
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| format!("Reading {}", directory.display()))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
                let id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .with_context(|| format!("Invalid colorscheme filename {}", path.display()))?
                    .to_owned();
                files.push((id, path));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        for (id, path) in files {
            themes.push(ThemeFile::load(&path, &id)?);
        }
        Ok(themes)
    }

    pub fn validate(&self) -> Result<()> {
        crate::image_assets::validate_assets_dir(&self.image_assets_dir)?;
        for (name, value, min, max) in [
            ("font_size", self.font_size, 10., 32.),
            ("line_height", self.line_height, self.font_size + 4., 64.),
            ("explorer_width", self.explorer_width, 180., 600.),
            ("writing_width", self.writing_width, 320., 1600.),
            ("terminal_height", self.terminal_height, 100., 600.),
        ] {
            if !value.is_finite() || value < min || value > max {
                bail!("{name} must be between {min} and {max}");
            }
        }
        if self.font_family.trim().is_empty() || self.shell.trim().is_empty() {
            bail!("font_family and shell must not be empty");
        }
        for color in [
            &self.theme.background,
            &self.theme.panel,
            &self.theme.foreground,
            &self.theme.muted,
            &self.theme.accent,
        ] {
            parse_color(color)?;
        }
        for (name, color) in [
            ("normal", &self.markdown.colors.normal),
            ("bold", &self.markdown.colors.bold),
            ("italic", &self.markdown.colors.italic),
            ("bold_italic", &self.markdown.colors.bold_italic),
            ("code", &self.markdown.colors.code),
            ("link", &self.markdown.colors.link),
            ("strikethrough", &self.markdown.colors.strikethrough),
            ("quote", &self.markdown.colors.quote),
        ] {
            if let Some(color) = color {
                parse_color(color).with_context(|| format!("markdown.colors.{name}"))?;
            }
        }
        for level in 1..=6 {
            let heading = self.markdown.heading(level);
            if let Some(size) = heading.font_size
                && (!size.is_finite() || !(10. ..=128.).contains(&size))
            {
                bail!("markdown.h{level}.font_size must be between 10 and 128");
            }
            if let Some(color) = &heading.color {
                parse_color(color).with_context(|| format!("markdown.h{level}.color"))?;
            }
        }
        if let Some(color) = &self.markdown.divider.color {
            parse_color(color).context("markdown.divider.color")?;
        }
        let thickness = self.markdown.divider.thickness;
        if !thickness.is_finite() || !(0.5..=12.).contains(&thickness) {
            bail!("markdown.divider.thickness must be between 0.5 and 12");
        }
        for (key, action) in &self.keys {
            gpui::Keystroke::parse(key).with_context(|| format!("Invalid shortcut {key}"))?;
            if ![
                "save",
                "new",
                "open",
                "save-as",
                "paste",
                "undo",
                "redo",
                "explorer",
                "terminal",
                "themes",
                "ui-mode",
                "outline",
                "tab-bar",
                "status-bar",
                "editor",
                "help",
                "buffer-delete",
                "previous-buffer",
                "next-buffer",
            ]
            .contains(&action.as_str())
            {
                bail!("Unknown shortcut action: {action}");
            }
        }
        Ok(())
    }
}

fn config_base(mut env: impl FnMut(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let mut get = |name| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    get("XDG_CONFIG_HOME").or_else(|| {
        if cfg!(windows) {
            get("APPDATA").or_else(|| get("USERPROFILE").map(|home| home.join("AppData/Roaming")))
        } else {
            get("HOME").map(|home| home.join(".config"))
        }
    })
}

pub fn parse_color(value: &str) -> Result<u32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Colors must be six-digit hexadecimal RGB values");
    }
    u32::from_str_radix(value, 16).context("Invalid RGB color")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_asset_directory_configuration_stays_document_relative() {
        let config = Config::parse(Path::new("c.toml"), "image_assets_dir = 'media/图片'").unwrap();
        config.validate().unwrap();
        for directory in ["", "../assets", "/tmp/assets", "C:/assets"] {
            let config = Config {
                image_assets_dir: directory.into(),
                ..Default::default()
            };
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn theme_presets_supply_valid_palettes() {
        for &preset in ThemePreset::ALL {
            let config = Config::parse(
                Path::new("c.toml"),
                &format!("[theme]\npreset = '{}'", preset.id()),
            )
            .unwrap();
            config.validate().unwrap();
            assert_eq!(config.theme, preset.theme());
            assert_eq!(
                config.theme.is_dark(),
                !matches!(preset, ThemePreset::Paper)
            );
        }
    }

    #[test]
    fn explicit_theme_colors_override_presets_in_either_order() {
        for fields in [
            "preset = 'paper'\naccent = '#123456'",
            "accent = '#123456'\npreset = 'paper'",
        ] {
            let config = Config::parse(Path::new("c.toml"), &format!("[theme]\n{fields}")).unwrap();
            assert_eq!(
                config.theme,
                Theme {
                    accent: "#123456".into(),
                    ..ThemePreset::Paper.theme()
                }
            );
        }
    }

    #[test]
    fn legacy_custom_themes_keep_unspecified_rockdown_colors() {
        let config = Config::parse(
            Path::new("c.toml"),
            "[theme]\nbackground = '#ffffff'\nforeground = '#111111'\nmuted = '#555555'\npanel = '#eeeeee'",
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(
            config.theme,
            Theme {
                background: "#ffffff".into(),
                foreground: "#111111".into(),
                muted: "#555555".into(),
                panel: "#eeeeee".into(),
                ..Theme::default()
            }
        );
        assert_eq!(
            Config::parse(Path::new("c.toml"), "").unwrap().theme,
            Theme::default()
        );
    }

    #[test]
    fn unknown_theme_inputs_are_rejected() {
        for fields in [
            "preset = 'unknown'",
            "preset = 'solarized_light'",
            "preset = 1",
            "preset = 'nord'\naccnet = '#123456'",
            "accnet = '#123456'",
        ] {
            assert!(Config::parse(Path::new("c.toml"), &format!("[theme]\n{fields}")).is_err());
        }
    }

    #[test]
    fn syntax_brightness_follows_background_overrides_not_preset() {
        for (preset, background, dark) in [
            ("rockdown", "#ffffff", false),
            ("paper", "#000000", true),
            ("rockdown", "#00ff00", false),
            ("paper", "#0000ff", true),
        ] {
            let config = Config::parse(
                Path::new("c.toml"),
                &format!("[theme]\npreset = '{preset}'\nbackground = '{background}'"),
            )
            .unwrap();
            assert_eq!(config.theme.is_dark(), dark);
        }
    }

    #[test]
    fn colorscheme_discovery_is_sorted_and_reloads_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let ids = |themes: Vec<Theme>| {
            themes
                .into_iter()
                .map(|theme| theme.preset)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(Config::colorschemes(Some(&config_path)).unwrap()),
            ["rockdown", "paper"]
        );
        let directory = root.path().join("colorscheme");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("zeta.toml"), "name = 'First label'").unwrap();
        std::fs::write(
            directory.join("alpha.toml"),
            "name = 'Last label'\nbase = 'paper'",
        )
        .unwrap();
        std::fs::write(directory.join("ignored.txt"), "not toml").unwrap();
        let themes = Config::colorschemes(Some(&config_path)).unwrap();
        assert_eq!(themes[2].name, "Last label");
        assert_eq!(themes[2].background, ThemePreset::Paper.theme().background);
        assert_eq!(ids(themes), ["rockdown", "paper", "alpha", "zeta"]);

        std::fs::write(&config_path, "[theme]\npreset = 'alpha'").unwrap();
        let first = Config::load(Some(&config_path)).unwrap().0;
        std::fs::write(
            directory.join("alpha.toml"),
            "name = 'Updated'\n[theme]\naccent = '#112233'",
        )
        .unwrap();
        let second = Config::load(Some(&config_path)).unwrap().0;
        assert_ne!(first.theme.accent, second.theme.accent);
        assert_eq!(second.theme.name, "Updated");
        assert_eq!(second.theme.accent, "#112233");
        assert_eq!(
            Config::colorschemes(Some(&config_path)).unwrap()[2],
            second.theme
        );
        std::fs::remove_file(directory.join("alpha.toml")).unwrap();
        assert_eq!(
            ids(Config::colorschemes(Some(&config_path)).unwrap()),
            ["rockdown", "paper", "zeta"]
        );
        let error = Config::load(Some(&config_path)).unwrap_err();
        assert!(format!("{error:#}").contains("alpha.toml"));
    }

    #[test]
    fn custom_colors_merge_with_explicit_config_and_adapt_inherited_palettes() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let directory = root.path().join("colorscheme");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("custom.toml"),
            r##"
name = "Custom display name"
base = "paper"
[theme]
background = "#000000"
accent = "#123456"
[syntax]
keyword = "#654321"
[markdown.colors]
normal = "#101010"
bold = "#202020"
italic = "#303030"
bold_italic = "#404040"
code = "#505050"
link = "#606060"
strikethrough = "#707070"
quote = "#808080"
[markdown.h1]
color = "#909090"
[markdown.h2]
color = "#a0a0a0"
[markdown.divider]
color = "#b0b0b0"
"##,
        )
        .unwrap();
        let config = Config::parse(
            &config_path,
            r##"
[theme]
preset = "custom"
background = "#ffffff"
accent = "#356d61"
[markdown.colors]
normal = "#abcdef"
[markdown.h1]
font_size = 42
underline = true
[markdown.h2]
color = "#fedcba"
[markdown.divider]
thickness = 3
"##,
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.theme.preset, "custom");
        assert_eq!(config.theme.name, "Custom display name");
        assert_eq!(config.theme.accent, "#356d61");
        assert_eq!(config.markdown.colors.normal.as_deref(), Some("#abcdef"));
        assert_eq!(config.markdown.colors.bold.as_deref(), Some("#202020"));
        assert_eq!(config.markdown.colors.italic.as_deref(), Some("#303030"));
        assert_eq!(
            config.markdown.colors.bold_italic.as_deref(),
            Some("#404040")
        );
        assert_eq!(config.markdown.colors.code.as_deref(), Some("#505050"));
        assert_eq!(config.markdown.colors.link.as_deref(), Some("#606060"));
        assert_eq!(
            config.markdown.colors.strikethrough.as_deref(),
            Some("#707070")
        );
        assert_eq!(config.markdown.colors.quote.as_deref(), Some("#808080"));
        assert_eq!(config.markdown.h1.color.as_deref(), Some("#909090"));
        assert_eq!(config.markdown.h1.font_size, Some(42.));
        assert!(config.markdown.h1.underline);
        assert_eq!(config.markdown.h2.color.as_deref(), Some("#fedcba"));
        assert_eq!(config.markdown.divider.color.as_deref(), Some("#b0b0b0"));
        assert_eq!(config.markdown.divider.thickness, 3.);
        let mut expected = ThemePreset::Paper.theme().syntax_palette();
        expected[8] = 0x654321;
        assert_eq!(config.theme.syntax_palette(), expected);
        assert_eq!(
            config.theme.ansi_palette(),
            ThemePreset::Paper.theme().ansi_palette()
        );
        let raw = Config::colorschemes(Some(&config_path))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(raw.syntax_palette()[8], 0x654321);
        assert_ne!(raw.syntax_palette()[0], config.theme.syntax_palette()[0]);
        assert_ne!(raw.ansi_palette(), config.theme.ansi_palette());

        let mut markdown = config.markdown.clone();
        raw.apply_markdown_colors(&mut markdown);
        assert_eq!(markdown.colors.normal.as_deref(), Some("#101010"));
        assert_eq!(markdown.h2.color.as_deref(), Some("#a0a0a0"));
        ThemePreset::Rockdown
            .theme()
            .apply_markdown_colors(&mut markdown);
        assert_eq!(markdown.colors, MarkdownColors::default());
        assert!(markdown.h1.color.is_none());
        assert!(markdown.h2.color.is_none());
        assert!(markdown.divider.color.is_none());
        assert_eq!(markdown.h1.font_size, Some(42.));
        assert!(markdown.h1.underline);
        assert_eq!(markdown.divider.thickness, 3.);
    }

    #[test]
    fn explicit_syntax_and_terminal_palettes_keep_role_order_and_values() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let directory = root.path().join("colorscheme");
        std::fs::create_dir(&directory).unwrap();
        let ansi = (0..16)
            .map(|value| format!("'#{value:06x}'"))
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(
            directory.join("custom.toml"),
            format!(
                r##"
[syntax]
foreground = "#000010"
comment = "#000011"
variable = "#000012"
constant = "#000013"
type = "#000014"
string = "#000015"
escape = "#000016"
function = "#000017"
keyword = "#000018"
embedded = "#000019"
[terminal]
ansi = [{ansi}]
"##
            ),
        )
        .unwrap();
        for background in ["#ffffff", "#000000"] {
            let config = Config::parse(
                &config_path,
                &format!("[theme]\npreset = 'custom'\nbackground = '{background}'"),
            )
            .unwrap();
            assert_eq!(
                config.theme.syntax_palette(),
                std::array::from_fn(|index| 0x10 + index as u32)
            );
            assert_eq!(
                config.theme.ansi_palette(),
                std::array::from_fn(|index| index as u32)
            );
        }
    }

    #[test]
    fn colorscheme_errors_identify_files_and_reject_invalid_schema() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let directory = root.path().join("colorscheme");
        std::fs::create_dir(&directory).unwrap();
        let selected = "[theme]\npreset = 'custom'";
        let file = directory.join("custom.toml");
        assert!(
            format!("{:#}", Config::parse(&config_path, selected).unwrap_err())
                .contains("custom.toml")
        );
        for contents in [
            "not valid toml",
            "unknown = true",
            "base = 'nord'",
            "theme.preset = 'paper'",
            "theme.accnet = '#123456'",
            "theme.background = '#bad'",
            "syntax.keywrod = '#123456'",
            "syntax.keyword = 'red'",
            "terminal.unknown = []",
            "terminal.ansi = ['#123456']",
            "markdown.unknown = true",
            "markdown.colors.bould = '#123456'",
            "markdown.colors.link = 'red'",
            "markdown.h1.font_size = 40",
            "markdown.h2.underline = true",
            "markdown.h3.color = 'red'",
            "markdown.divider.thickness = 2",
            "markdown.divider.color = 'red'",
        ] {
            std::fs::write(&file, contents).unwrap();
            let error = Config::parse(&config_path, selected).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains(&config_path.display().to_string()),
                "{message}"
            );
            assert!(message.contains(&file.display().to_string()), "{message}");
            let error = Config::colorschemes(Some(&config_path)).unwrap_err();
            assert!(format!("{error:#}").contains(&file.display().to_string()));
        }
        for count in [15, 17] {
            let colors = vec!["'#123456'"; count].join(",");
            std::fs::write(&file, format!("[terminal]\nansi = [{colors}]")).unwrap();
            assert!(Config::colorschemes(Some(&config_path)).is_err());
        }
        let mut colors = vec!["'#123456'"; 16];
        colors[7] = "'invalid'";
        std::fs::write(&file, format!("[terminal]\nansi = [{}]", colors.join(","))).unwrap();
        assert!(Config::colorschemes(Some(&config_path)).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        std::fs::write(&directory, "not a directory").unwrap();
        let error = Config::colorschemes(Some(&config_path)).unwrap_err();
        assert!(format!("{error:#}").contains(&directory.display().to_string()));
    }

    #[test]
    fn colorscheme_ids_cannot_traverse_directories_or_replace_builtins() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        let directory = root.path().join("colorscheme");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(root.path().join("outside.toml"), "").unwrap();
        for id in [
            "",
            ".",
            "..",
            "../outside",
            "nested/other",
            r"nested\other",
            "/absolute",
            "C:drive",
        ] {
            let error =
                Config::parse(&config_path, &format!("[theme]\npreset = '{id}'")).unwrap_err();
            assert!(format!("{error:#}").contains("Invalid colorscheme ID"));
        }
        for preset in ThemePreset::ALL {
            let file = directory.join(format!("{}.toml", preset.id()));
            std::fs::write(&file, "name = 'Impostor'").unwrap();
            let error = Config::colorschemes(Some(&config_path)).unwrap_err();
            assert!(format!("{error:#}").contains("reserved"));
            assert!(format!("{error:#}").contains(&file.display().to_string()));
            let config = Config::parse(
                &config_path,
                &format!("[theme]\npreset = '{}'", preset.id()),
            )
            .unwrap();
            assert_eq!(config.theme, preset.theme());
            std::fs::remove_file(file).unwrap();
        }
    }

    #[test]
    fn config_location_uses_native_directories_and_honors_xdg() {
        let env = BTreeMap::from([
            ("XDG_CONFIG_HOME", "custom"),
            ("APPDATA", "roaming"),
            ("USERPROFILE", "profile"),
            ("HOME", "home"),
        ]);
        assert_eq!(
            config_base(|name| env.get(name).map(Into::into)),
            Some(PathBuf::from("custom"))
        );
        assert_eq!(
            config_base(|name| {
                (name != "XDG_CONFIG_HOME")
                    .then(|| env.get(name).map(Into::into))
                    .flatten()
            }),
            Some(PathBuf::from(if cfg!(windows) {
                "roaming"
            } else {
                "home/.config"
            }))
        );
        assert_eq!(
            config_base(|name| match name {
                "XDG_CONFIG_HOME" | "APPDATA" => Some("".into()),
                "USERPROFILE" | "HOME" => Some("home".into()),
                _ => None,
            }),
            Some(PathBuf::from(if cfg!(windows) {
                "home/AppData/Roaming"
            } else {
                "home/.config"
            }))
        );
        assert!(config_base(|_| None).is_none());
        let config = Config::default();
        config.validate().unwrap();
        assert_eq!(config.keys["ctrl-shift-v"], "paste");
        assert_eq!(config.keys["ctrl-1"], "editor");
        if cfg!(windows) {
            assert_eq!(config.font_family, "Consolas");
        }
    }

    #[test]
    fn parses_and_validates_toml_configuration() {
        let config = Config::parse(
            Path::new("c.toml"),
            r##"
font_size = 18
[keys]
'ctrl-t' = 'terminal'
[markdown.colors]
normal = '#123456'
bold = '#abcdef'
italic = '#234567'
bold_italic = '#345678'
code = '#456789'
link = '#56789a'
strikethrough = '#6789ab'
quote = '#789abc'
[markdown.h1]
font_size = 48
underline = true
[markdown.h6]
color = '#89abcd'
[markdown.divider]
thickness = 2.5
"##,
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.font_size, 18.0);
        assert_eq!(config.keys.get("ctrl-t"), Some(&"terminal".to_string()));
        let colors = &config.markdown.colors;
        assert_eq!(
            [
                colors.normal.as_deref(),
                colors.bold.as_deref(),
                colors.italic.as_deref(),
                colors.bold_italic.as_deref(),
                colors.code.as_deref(),
                colors.link.as_deref(),
                colors.strikethrough.as_deref(),
                colors.quote.as_deref(),
            ],
            [
                Some("#123456"),
                Some("#abcdef"),
                Some("#234567"),
                Some("#345678"),
                Some("#456789"),
                Some("#56789a"),
                Some("#6789ab"),
                Some("#789abc"),
            ]
        );
        assert_eq!(config.markdown.heading(1).font_size, Some(48.));
        assert!(config.markdown.heading(1).underline);
        assert!(config.markdown.heading(1).color.is_none());
        assert_eq!(config.markdown.heading(6).color.as_deref(), Some("#89abcd"));
        for level in 2..=6 {
            assert!(config.markdown.heading(level).font_size.is_none());
            assert!(!config.markdown.heading(level).underline);
        }
        assert!(config.markdown.divider.color.is_none());
        assert_eq!(config.markdown.divider.thickness, 2.5);
    }
    #[test]
    fn writing_layout_defaults_and_limits() {
        let defaults = Config::parse(Path::new("c.toml"), "").unwrap();
        assert_eq!(defaults.writing_width, 820.);
        assert!(!defaults.markdown_line_numbers);
        for width in [320., 820., 1600.] {
            let config = Config::parse(
                Path::new("c.toml"),
                &format!("writing_width = {width}\nmarkdown_line_numbers = true"),
            )
            .unwrap();
            config.validate().unwrap();
            assert!(config.markdown_line_numbers);
        }
        for value in ["319.9", "1600.1", "nan", "inf"] {
            let config =
                Config::parse(Path::new("c.toml"), &format!("writing_width = {value}")).unwrap();
            assert!(
                config
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("writing_width")
            );
        }
        assert!(Config::parse(Path::new("c.toml"), "markdown_line_numbers = 'true'").is_err());
        Config::parse(
            Path::new("config.toml"),
            include_str!("../examples/config.toml"),
        )
        .unwrap()
        .validate()
        .unwrap();
    }

    #[test]
    fn invalid_configuration_is_not_silently_ignored() {
        assert!(Config::parse(Path::new("c.toml"), "font_sze = 18").is_err());
        let config = Config {
            font_size: f32::NAN,
            ..Config::default()
        };
        assert!(config.validate().is_err());
        assert!(Config::parse(Path::new("c.lua"), "font_size = 18").is_err());
    }

    #[test]
    fn invalid_markdown_styles_are_rejected() {
        for (field, value) in [
            ("markdown.colors.normal", "'+12345'"),
            ("markdown.colors.bold", "'#12345'"),
            ("markdown.colors.italic", "'#gggggg'"),
            ("markdown.colors.bold_italic", "'red'"),
            ("markdown.colors.code", "'#12345678'"),
            ("markdown.colors.link", "'#12 456'"),
            ("markdown.colors.strikethrough", "''"),
            ("markdown.colors.quote", "'##123456'"),
            ("markdown.divider.color", "'#bad'"),
            ("markdown.divider.thickness", "nan"),
            ("markdown.divider.thickness", "0.49"),
            ("markdown.divider.thickness", "12.01"),
        ] {
            let config = Config::parse(Path::new("c.toml"), &format!("{field} = {value}")).unwrap();
            let error = config.validate().unwrap_err();
            assert!(error.to_string().contains(field), "{error:#}");
        }
        for level in 1..=6 {
            for (property, value) in [
                ("font_size", "nan"),
                ("font_size", "inf"),
                ("font_size", "9.99"),
                ("font_size", "128.01"),
                ("color", "'invalid'"),
            ] {
                let field = format!("markdown.h{level}.{property}");
                let config =
                    Config::parse(Path::new("c.toml"), &format!("{field} = {value}")).unwrap();
                let error = config.validate().unwrap_err();
                assert!(error.to_string().contains(&field), "{error:#}");
            }
        }
        for toml in [
            "markdown.unknown = true",
            "markdown.colors.bould = '#123456'",
            "markdown.h1.size = 32",
            "markdown.divider.width = 2",
            "markdown.h2.underline = 'true'",
        ] {
            assert!(Config::parse(Path::new("c.toml"), toml).is_err());
        }
        for (size, thickness) in [(10., 0.5), (128., 12.)] {
            let config = Config::parse(
                Path::new("c.toml"),
                &format!(
                    "markdown.h1.font_size = {size}\nmarkdown.divider.thickness = {thickness}"
                ),
            )
            .unwrap();
            config.validate().unwrap();
        }
    }
}
