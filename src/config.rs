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
    pub terminal_height: f32,
    pub shell: String,
    pub theme: Theme,
    pub markdown: MarkdownStyle,
    pub keys: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub background: String,
    pub panel: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
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

#[derive(Clone, Debug, Default, Deserialize)]
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
        Self {
            background: "#171b22".into(),
            panel: "#1e242e".into(),
            foreground: "#dce3ec".into(),
            muted: "#8995a7".into(),
            accent: "#9cc7b5".into(),
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: if cfg!(windows) { "Consolas" } else { "Menlo" }.into(),
            font_size: 15.,
            line_height: 30.,
            explorer_width: 290.,
            terminal_height: 240.,
            shell: std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" })
                .ok()
                .filter(|shell| !shell.trim().is_empty())
                .unwrap_or_else(|| if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into()),
            theme: Theme::default(),
            markdown: MarkdownStyle::default(),
            keys: BTreeMap::from([
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
                ("cmd-e".into(), "explorer".into()),
                ("ctrl-e".into(), "explorer".into()),
                ("ctrl-`".into(), "terminal".into()),
                ("cmd-1".into(), "editor".into()),
                ("ctrl-1".into(), "editor".into()),
                ("ctrl-pageup".into(), "previous-buffer".into()),
                ("ctrl-pagedown".into(), "next-buffer".into()),
                ("cmd-w".into(), "buffer-delete".into()),
                ("ctrl-shift-w".into(), "buffer-delete".into()),
                ("f1".into(), "help".into()),
            ]),
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
                Self::parse(path, &text).with_context(|| format!("Loading {}", path.display()))?
            }
        };
        config.validate()?;
        Ok((config, path))
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("toml") => Ok(toml::from_str(text)?),
            _ => bail!("Configuration must have a .toml extension"),
        }
    }

    pub fn validate(&self) -> Result<()> {
        for (name, value, min, max) in [
            ("font_size", self.font_size, 10., 32.),
            ("line_height", self.line_height, self.font_size + 4., 64.),
            ("explorer_width", self.explorer_width, 180., 600.),
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
                "explorer",
                "terminal",
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
