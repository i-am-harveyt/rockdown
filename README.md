# Rockdown

A lightweight, native Markdown editor built with **Rust and GPUI**. Write in plain Markdown with in-place preview and Vim-style editing—without a browser or Electron runtime.

## Features

- **Live Markdown preview:** headings, emphasis, code, lists, tasks, tables, quotes, and local images. The active line shows its source syntax; other lines render in place.
- **Vim-style editing:** Normal, Insert, and Visual modes, motions, operators, search, and undo/redo. A focused subset, not a full Vim implementation.
- **Multiple documents:** tabs preserve unsaved text and editing history; local recovery restores drafts after a crash.
- **Dev and Writer layouts:** a docked workspace or a distraction-free writing view with floating controls.
- **Files and terminal:** edit filenames to stage filesystem operations, or open an embedded shell.
- **TOML configuration:** customize typography, themes, layout, and shortcuts.

Preview applies to `.md` files and untitled documents. Other files open as plain text. All documents remain UTF-8 text on disk.

## Install and run

**Platform status:** native UI verified on macOS Apple Silicon. CI and automated release archives currently cover Windows x64 only; native Windows UI and Linux builds remain unverified.

### Windows release

Extract the Windows release archive and launch `rockdown.exe`, or open a file or existing notes directory from PowerShell:

```powershell
.\rockdown.exe "$HOME\Documents\Notes"
```

### Build from source

Requires a Rust toolchain with **Rust 2024 edition** support and native development tools:

- **macOS:** install Xcode Command Line Tools with `xcode-select --install`. A separate Metal compiler toolchain is not required.
- **Windows:** Windows 10 version 1809 or newer, the Rust MSVC toolchain, and Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.
- **Linux:** additional GPUI system dependencies and an installed monospace font may be needed.

From the repository root:

```sh
cargo build --release --locked

./target/release/rockdown                     # Current directory
./target/release/rockdown /path/to/notes       # Notes directory
./target/release/rockdown note.md draft.md    # One or more documents
```

Missing files become empty buffers and are created when saved; their parent directories must exist. A directory must be the only positional path.

On Windows, build in **Developer PowerShell for Visual Studio**:

```powershell
rustup default stable-x86_64-pc-windows-msvc
$env:GPUI_FXC_PATH = Join-Path $env:WindowsSdkVerBinPath 'x64\fxc.exe'
cargo build --release --locked
.\target\release\rockdown.exe
```

If `WindowsSdkVerBinPath` is unavailable, set `GPUI_FXC_PATH` to your installed SDK's `fxc.exe`. macOS app packaging is available through [`scripts/bundle-macos.sh`](scripts/bundle-macos.sh).

## Getting started

1. Open your notes directory or a Markdown file.
2. The editor starts in **Normal mode**. Press `i` to enter Insert mode and type; press `Esc` to return to Normal mode.
3. Press **Cmd-S / Ctrl-S** to save. Untitled documents prompt for a filename.
4. Press **F1** or enter `:help` for the in-app keyboard and command reference.

### Essential shortcuts

`Cmd/Ctrl` means Cmd on macOS and Ctrl on Windows/Linux. For sequences such as `Ctrl-W h`, press `Ctrl-W`, release it, then press `h`.

| Shortcut | Action |
| --- | --- |
| `Cmd/Ctrl-N` / `Cmd/Ctrl-O` | New document / open file |
| `Cmd/Ctrl-S` / `Cmd/Ctrl-Shift-S` | Save / Save As |
| `Ctrl-PageUp` / `Ctrl-PageDown` | Previous / next document |
| `Cmd-W` / `Ctrl-Shift-W` | Close document on macOS / Windows and Linux |
| `Cmd/Ctrl-E` | Show or hide Files |
| Ctrl + backtick | Show or hide terminal |
| `Ctrl-W h` / `Ctrl-W l` / `Ctrl-W j` | Focus editor / Files / terminal |
| `Cmd/Ctrl-Shift-O` | Document outline |
| `Cmd/Ctrl-Shift-M` | Dev / Writer mode |
| `Cmd/Ctrl-Shift-T` | Theme picker |
| `F1` | Keyboard guide |

In Normal mode, `u` undoes and `Ctrl-R` redoes. Standard platform undo/redo shortcuts also work. Select text to apply formatting shortcuts such as **Cmd/Ctrl-B** for bold or **Cmd/Ctrl-I** for italic.

### Files and terminal

Files is an editable listing of filenames. Use `j` / `k` to select an entry, Return to open it, and `-` to go to its parent directory.

- `cc` renames an entry, `o` adds a file (or a directory if the name ends in `/`), and `dd` stages deletion.
- `:w` or **Cmd/Ctrl-S** in Files commits staged operations; `:e!` discards them.
- Committed deletions go to **`.rockdown-trash` at the workspace root**, not the OS Trash. Restore them with a file manager or terminal.

The terminal starts in the current Files directory. Hiding it keeps the shell running; `exit` ends the session. Use **Ctrl-Shift-V** to paste and **Ctrl-C** to interrupt a command.

### Images, saving, and recovery

- Paste images with **Cmd-V / Ctrl-Shift-V**, or drop local image files into a Markdown document. Images are copied into `assets/` beside the document; untitled documents first prompt to save. Remote images are not fetched.
- Undoing an image insertion removes the link, not the image file. Save As and renames do not relocate assets: move documents together with their asset directories.
- Saves detect external file changes rather than silently overwriting them. Inspect conflicts before using `:e!` to discard edits and reload, or `:w!` to force an overwrite.
- Local recovery checkpoints preserve drafts, **not automatic saves or backups**. Recent edits can be lost between checkpoints; undo history, terminal sessions, and staged Files operations do not survive a restart. Recovery files are local and unencrypted.
- Closing a dirty document prompts to Save / Discard / Cancel. `:q!` explicitly discards all unsaved and staged changes.

## Configuration

Start with [`examples/config.toml`](examples/config.toml), which includes appearance settings and the shortcut map.

Default location:

- **macOS / Linux:** `~/.config/rockdown/config.toml`
- **Windows:** `%APPDATA%\rockdown\config.toml`
- **With `XDG_CONFIG_HOME`:** `$XDG_CONFIG_HOME/rockdown/config.toml` takes precedence over those defaults.

Use `--config PATH` to select a different file. Without a config file, built-in defaults apply. Project-local configuration is not loaded automatically.

```toml
font_size = 16
writing_width = 820
markdown_line_numbers = false
image_assets_dir = "assets"

[theme]
preset = "paper" # "rockdown" for the built-in dark theme
```

Validate a configuration without opening a window:

```sh
./target/release/rockdown --check-config --config examples/config.toml
```

Use `:config` to reload settings in the app. Invalid settings report an error. **Defining `[keys]` replaces the entire default shortcut map**, so copy and adapt the example map if you want to keep other defaults.

For a custom theme, copy [`examples/nord.toml`](examples/nord.toml) to `colorscheme/nord.toml` beside your config and set `[theme] preset = "nord"`. Theme-picker choices apply only to the current session.

Use `--help` for command-line options and **F1** for the in-app reference.

## Development

```sh
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

UI changes also require native-window interaction checks. Restart any running instance after rebuilding.
