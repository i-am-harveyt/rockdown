use anyhow::{Context, Result, bail};
use mlua::LuaSerdeExt;
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
            font_family: "Menlo".into(),
            font_size: 15.,
            line_height: 30.,
            explorer_width: 290.,
            terminal_height: 240.,
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            theme: Theme::default(),
            keys: BTreeMap::from([
                ("cmd-s".into(), "save".into()),
                ("ctrl-s".into(), "save".into()),
                ("cmd-v".into(), "paste".into()),
                ("cmd-e".into(), "explorer".into()),
                ("ctrl-e".into(), "explorer".into()),
                ("ctrl-`".into(), "terminal".into()),
                ("cmd-1".into(), "editor".into()),
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
    /// Only user-level config is auto-loaded. Project-local Lua must be explicitly selected.
    pub fn load(explicit: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let path = if let Some(path) = explicit {
            Some(path.to_path_buf())
        } else {
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                })
                .map(|base| base.join("rockdown"));
            base.and_then(|base| {
                [base.join("config.toml"), base.join("config.lua")]
                    .into_iter()
                    .find(|p| p.is_file())
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
            Some("lua") => {
                let lua = mlua::Lua::new();
                let value: mlua::Value = lua
                    .load(text)
                    .set_name(path.to_string_lossy())
                    .eval()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                lua.from_value(value)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
            }
            _ => bail!("Configuration must have a .toml or .lua extension"),
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
        for (key, action) in &self.keys {
            gpui::Keystroke::parse(key).with_context(|| format!("Invalid shortcut {key}"))?;
            if ![
                "save",
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

pub fn parse_color(value: &str) -> Result<u32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 {
        bail!("Theme colors must be six-digit hexadecimal RGB values");
    }
    u32::from_str_radix(value, 16).context("Invalid RGB color")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_formats_produce_equivalent_settings() {
        let toml = Config::parse(
            Path::new("c.toml"),
            "font_size = 18\n[keys]\n'ctrl-t' = 'terminal'",
        )
        .unwrap();
        let lua = Config::parse(
            Path::new("c.lua"),
            "return { font_size = 18, keys = { ['ctrl-t'] = 'terminal' } }",
        )
        .unwrap();
        assert_eq!(toml.font_size, lua.font_size);
        assert_eq!(toml.keys, lua.keys);
        toml.validate().unwrap();
        lua.validate().unwrap();
    }
    #[test]
    fn invalid_configuration_is_not_silently_ignored() {
        assert!(Config::parse(Path::new("c.toml"), "font_sze = 18").is_err());
        let config = Config {
            font_size: f32::NAN,
            ..Config::default()
        };
        assert!(config.validate().is_err());
        assert!(Config::parse(Path::new("c.lua"), "return {").is_err());
    }
}
