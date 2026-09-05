mod app;
mod keyboard;
mod surface;

use anyhow::{Context, Result, bail};
use gpui::{prelude::*, *};
use rockdown::{config::Config, document::Document, explorer::Explorer};
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("rockdown: {error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut path = None;
    let mut config_path = None;
    let mut check = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "Rockdown — native Vim-first Markdown workspace\n\nUsage: rockdown [FILE|DIRECTORY] [--config PATH] [--check-config]\n\n--config PATH    Load TOML or Lua settings from an explicit file\n--check-config   Validate settings without opening a window\n\nDefault settings: $XDG_CONFIG_HOME/rockdown/config.toml or config.lua\n(falls back to ~/.config; TOML wins if both exist). Lua returns a table.\n\nIn the app: F1 help, Ctrl-E toggle files, Ctrl-` terminal, Cmd-S save.\nEditor: i insert, Return new line, Esc normal, hjkl motion, :w [file].\nBuffers: :bp previous, :bn next, :bd close, :bd! discard and close.\nCtrl-PageUp/PageDown cycle; Cmd-W or Ctrl-Shift-W closes the current buffer.\nExplorer: drag its left edge to resize; edit names, o create, dd trash, :w apply.\n:q closes the window only when every buffer is saved; :q! discards all.\nSee examples/config.toml and examples/config.lua for settings."
                );
                return Ok(());
            }
            Some("--config") => {
                config_path = Some(PathBuf::from(
                    args.next().context("--config requires a path")?,
                ))
            }
            Some("--check-config") => check = true,
            Some(flag) if flag.starts_with('-') => bail!("Unknown argument: {flag}"),
            _ => {
                if path.is_some() {
                    bail!("Expected one file or directory");
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    let (config, config_path) = Config::load(config_path.as_deref())?;
    if check {
        println!(
            "Configuration valid: {}",
            config_path
                .as_ref()
                .map_or("built-in defaults".into(), |p| p.display().to_string())
        );
        return Ok(());
    }
    let path = path.unwrap_or(std::env::current_dir()?);
    let (document, directory) = if path.is_dir() {
        (Document::untitled(""), std::fs::canonicalize(&path)?)
    } else if path.exists() {
        let doc = Document::open(&path)?;
        let directory = doc.path.as_ref().unwrap().parent().unwrap().to_path_buf();
        (doc, directory)
    } else {
        let parent = std::fs::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new(".")),
        )?;
        let mut doc = Document::untitled("");
        doc.path = Some(parent.join(path.file_name().context("Missing filename")?));
        (doc, parent)
    };
    let explorer = Explorer::open(&directory)?;
    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1180.), px(800.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Rockdown".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let workspace = cx.new(|cx| {
                    app::Workspace::new(config, config_path, document, explorer, window, cx)
                });
                let weak = workspace.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    weak.update(cx, |app, cx| app.can_close(cx)).unwrap_or(true)
                });
                workspace
            },
        );
        if let Err(error) = result {
            eprintln!("Opening window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}
