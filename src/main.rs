use anyhow::{Context, Result, bail};
use gpui::{prelude::*, *};
use rockdown::{config::Config, document::Document, documents::Documents, explorer::Explorer};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("rockdown: {error:#}");
        std::process::exit(1);
    }
}

#[derive(Debug, Default)]
struct Cli {
    paths: Vec<PathBuf>,
    config_path: Option<PathBuf>,
    check: bool,
    help: bool,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut cli = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--help" | "-h") => {
                    cli.help = true;
                    return Ok(cli);
                }
                Some("--config") => {
                    cli.config_path = Some(PathBuf::from(
                        args.next().context("--config requires a path")?,
                    ));
                }
                Some("--check-config") => cli.check = true,
                Some("--") => {
                    cli.paths.extend(args.map(PathBuf::from));
                    break;
                }
                Some(flag) if flag.starts_with('-') => bail!("Unknown argument: {flag}"),
                _ => cli.paths.push(PathBuf::from(arg)),
            }
        }
        Ok(cli)
    }

    fn prepare(&self, cwd: &Path) -> Result<(Documents, Explorer)> {
        let mut documents = Documents::new(Document::untitled(""));
        if self.paths.is_empty() {
            return Ok((documents, Explorer::open(cwd)?));
        }
        let scratch = documents.active_id();
        for path in &self.paths {
            if path.as_os_str().is_empty() {
                bail!("File path must not be empty");
            }
            let path = cwd.join(path);
            if path.is_dir() {
                if self.paths.len() == 1 {
                    return Ok((documents, Explorer::open(&path)?));
                }
                bail!(
                    "Directories must be opened alone, not alongside files: {}",
                    path.display()
                );
            }
            documents.open(&path)?;
        }
        // Drop the initial scratch buffer; deleting index zero activates the first file.
        documents.select(scratch)?;
        documents.delete(true)?;
        let directory = documents
            .current()
            .path
            .as_deref()
            .and_then(Path::parent)
            .context("Opened file has no parent directory")?;
        let explorer = Explorer::open(directory)?;
        Ok((documents, explorer))
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse(std::env::args_os().skip(1))?;
    if cli.help {
        println!(
            "Rockdown - native Vim-first Markdown workspace\n\nUsage: rockdown [FILE ... | DIRECTORY] [--config PATH] [--check-config]\n\nOpen files in order with the first active. Missing files start as named empty buffers.\nA directory must be the only path; no paths opens the current directory.\nUse -- before filenames beginning with a dash.\n\n--config PATH    Load TOML settings from an explicit file\n--check-config   Validate settings without opening a window\n\nDefault settings: $XDG_CONFIG_HOME/rockdown/config.toml\n(falls back to %APPDATA%\\rockdown\\config.toml on Windows,\nor ~/.config/rockdown/config.toml on Unix).\n\nIn the app: F1 help, Ctrl-E toggle files, Ctrl-` terminal, Ctrl-S save.\nEditor: i insert, Return new line, Esc normal, hjkl motion, :w [file].\nClipboard: Ctrl-C/V in Editor and Files (Cmd-C/V on macOS); Ctrl-Shift-V in all panes.\nBuffers: :bp previous, :bn next, :bd close, :bd! discard and close.\nCtrl-PageUp/PageDown cycle; Cmd-W or Ctrl-Shift-W closes the current buffer.\nExplorer: drag its left edge to resize; edit names, o create, dd trash, :w apply.\n:q closes with Save/Discard/Cancel prompts; :q! discards all.\nSee examples/config.toml for settings."
        );
        return Ok(());
    }
    let (config, config_path) = Config::load(cli.config_path.as_deref())?;
    if cli.check {
        println!(
            "Configuration valid: {}",
            config_path
                .as_ref()
                .map_or("built-in defaults".into(), |p| p.display().to_string())
        );
        return Ok(());
    }
    let (documents, explorer) = cli.prepare(&std::env::current_dir()?)?;
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
                    appears_transparent: cfg!(target_os = "macos"),
                    traffic_light_position: Some(point(px(14.), px(13.))),
                }),
                ..Default::default()
            },
            move |window, cx| {
                let workspace = cx.new(|cx| {
                    let mut workspace = rockdown::app::Workspace::new(
                        config,
                        config_path,
                        Document::untitled(""),
                        explorer,
                        window,
                        cx,
                    );
                    workspace.explorer_visible = documents.current().path.is_none();
                    workspace.documents = documents;
                    workspace.refresh_projection();
                    workspace
                });
                let weak = workspace.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    weak.update(cx, |app, cx| app.request_window_close(window, cx))
                        .unwrap_or(true)
                });
                workspace
            },
        );
        if let Err(error) = result {
            eprintln!("Opening window: {error:#}");
            cx.quit();
        }
        cx.set_menus(vec![Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New", rockdown::app::NewDocument),
                MenuItem::action("Open…", rockdown::app::OpenDocument),
                MenuItem::separator(),
                MenuItem::action("Save", rockdown::app::Save),
                MenuItem::action("Save As…", rockdown::app::SaveAs),
                MenuItem::separator(),
                MenuItem::action("Close Document", rockdown::app::BufferDelete),
            ],
        }]);
        cx.activate(true);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use std::{ffi::OsString, fs, path::PathBuf};

    fn parse(args: &[&str]) -> Cli {
        Cli::parse(args.iter().map(OsString::from)).unwrap()
    }

    #[test]
    fn startup_opens_ordered_files_deduplicates_aliases_and_keeps_first_active() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("README.md"), "# Existing\n").unwrap();
        fs::write(dir.path().join("sub/other.md"), "Other file").unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let cli = parse(&[
            "README.md",
            "test.md",
            "sub/other.md",
            "./README.md",
            "sub/../test.md",
        ]);
        let (mut documents, explorer) = cli.prepare(dir.path()).unwrap();
        let paths: Vec<_> = documents
            .entries()
            .iter()
            .map(|entry| entry.document.path.clone().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec![
                root.join("README.md"),
                root.join("test.md"),
                root.join("sub/other.md")
            ]
        );
        assert_eq!(
            documents.current().path.as_ref(),
            Some(&root.join("README.md"))
        );
        assert_eq!(documents.current().buffer.text(), "# Existing\n");
        assert_eq!(explorer.directory, root);
        assert!(!documents.dirty());
        documents.next();
        assert_eq!(
            documents.current().path.as_ref(),
            Some(&root.join("test.md"))
        );
        assert_eq!(documents.current().buffer.text(), "");
        assert!(!dir.path().join("test.md").exists());
        documents.next();
        assert_eq!(documents.current().buffer.text(), "Other file");
    }

    #[test]
    fn startup_loads_existing_second_file_instead_of_empty_buffer() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "Read me").unwrap();
        fs::write(dir.path().join("test.md"), "Already exists").unwrap();
        let (mut documents, _) = parse(&["README.md", "test.md"])
            .prepare(dir.path())
            .unwrap();
        assert_eq!(documents.current().buffer.text(), "Read me");
        documents.next();
        assert_eq!(documents.current().buffer.text(), "Already exists");
        assert!(!documents.dirty());
    }

    #[test]
    fn startup_preserves_current_directory_and_sole_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for (args, directory) in [(vec![], root.clone()), (vec!["sub"], root.join("sub"))] {
            let (documents, explorer) = parse(&args).prepare(dir.path()).unwrap();
            assert_eq!(explorer.directory, directory);
            assert_eq!(documents.entries().len(), 1);
            assert!(documents.current().path.is_none());
            assert_eq!(documents.current().buffer.text(), "");
            assert!(!documents.dirty());
        }
    }

    #[test]
    fn startup_preserves_single_existing_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("existing.md"), "Loaded").unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for (name, text) in [("existing.md", "Loaded"), ("missing.md", "")] {
            let (documents, explorer) = parse(&[name]).prepare(dir.path()).unwrap();
            assert_eq!(documents.entries().len(), 1);
            assert_eq!(documents.current().path.as_ref(), Some(&root.join(name)));
            assert_eq!(documents.current().buffer.text(), text);
            assert_eq!(explorer.directory, root);
            assert!(!documents.dirty());
        }
        assert!(!dir.path().join("missing.md").exists());
    }

    #[test]
    fn startup_rejects_directories_among_files_and_invalid_file_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("invalid.md"), [0xff]).unwrap();
        fs::write(dir.path().join("file.md"), "Text").unwrap();
        for args in [vec!["sub", "new.md"], vec!["new.md", "sub"]] {
            assert!(parse(&args).prepare(dir.path()).is_err());
        }
        for path in ["absent/new.md", "file.md/new.md", "invalid.md", ""] {
            assert!(parse(&[path]).prepare(dir.path()).is_err(), "{path:?}");
        }
        assert!(!dir.path().join("new.md").exists());
        assert!(!dir.path().join("absent").exists());
    }

    #[cfg(unix)]
    #[test]
    fn startup_deduplicates_symlinks_and_rejects_dangling_links_and_nonfiles() {
        use std::os::unix::{fs::symlink, net::UnixListener};
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.md"), "Text").unwrap();
        symlink("file.md", dir.path().join("alias.md")).unwrap();
        symlink(".", dir.path().join("alias-dir")).unwrap();
        let (documents, _) = parse(&["file.md", "alias.md", "new.md", "alias-dir/new.md"])
            .prepare(dir.path())
            .unwrap();
        assert_eq!(documents.entries().len(), 2);
        assert_eq!(documents.current().buffer.text(), "Text");
        assert!(!dir.path().join("new.md").exists());

        symlink("missing.md", dir.path().join("dangling.md")).unwrap();
        assert!(parse(&["dangling.md"]).prepare(dir.path()).is_err());
        let _socket = UnixListener::bind(dir.path().join("socket")).unwrap();
        assert!(parse(&["socket"]).prepare(dir.path()).is_err());
        assert!(!dir.path().join("missing.md").exists());
    }

    #[test]
    fn startup_parser_preserves_options_and_supports_option_terminator() {
        let cli = parse(&[
            "first.md",
            "--config",
            "settings.toml",
            "second.md",
            "--check-config",
        ]);
        assert_eq!(
            cli.paths,
            vec![PathBuf::from("first.md"), PathBuf::from("second.md")]
        );
        assert_eq!(cli.config_path, Some(PathBuf::from("settings.toml")));
        assert!(cli.check);
        assert!(parse(&["--help"]).help);
        assert!(parse(&["-h"]).help);
        assert_eq!(
            parse(&["--", "-note.md", "--check-config"]).paths,
            vec![PathBuf::from("-note.md"), PathBuf::from("--check-config")]
        );
        assert!(Cli::parse([OsString::from("--config")]).is_err());
        assert!(Cli::parse([OsString::from("--unknown")]).is_err());
    }
}
