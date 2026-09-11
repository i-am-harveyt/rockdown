# Rockdown

A lightweight, native Markdown workspace built with **Rust and GPUI**. Rockdown combines in-place Markdown rendering, Vim-style editing, a buffer-editable file explorer, and an embedded terminal—without a browser or Electron runtime.

## What it does

- **Live-preview Markdown:** headings, emphasis, code, lists, task lists, tables, quotes, wrapped prose, and local images.
- **Vim-style editing:** Normal, Insert, and Visual modes; motions, counts, operators, search, and undo/redo.
- **Multiple buffers:** switch between documents without losing unsaved text, cursor position, viewport, or undo history.
- **Right-hand file explorer:** rename, create, and stage deletions by editing filenames as buffer lines. Hide the dock or drag its left edge to resize it.
- **Embedded terminal:** a real PTY shell with color support, keyboard input, and resizing.
- **TOML configuration:** customize fonts, colors, dock dimensions, shell, and shortcuts.

Rockdown uses **line-based live preview** for `.md` files (case-insensitive) and untitled buffers: inactive lines render as Markdown, while the active line exposes its source syntax for editing. All other named files—including TOML, code, and extensionless files—show literal plain text, without Markdown styling, tables, or image previews. Preview mode follows save-as and explorer renames. Files remain plain UTF-8 text on disk.

## Build and run

### Requirements

- A recent Rust toolchain with Cargo and Rust 2024 edition support.
- Native development tools required by GPUI and its dependencies.
- On macOS, Xcode Command Line Tools. Install them with `xcode-select --install` if needed.
- On Windows, Windows 10 version 1809 or newer (or Windows 11), the Rust MSVC toolchain, and Visual Studio Build Tools with **Desktop development with C++** and a Windows 10/11 SDK. The SDK provides GPUI's `fxc.exe` shader compiler.

**Platform status:** development and native-window verification have been performed on macOS Apple Silicon. Windows support uses GPUI's native backend, ConPTY for the embedded terminal, and Windows filesystem operations for explorer commits. CI is configured to test and build Windows x64 and macOS; native Windows UI behavior has not been verified on this development machine. Linux builds and UI behavior also remain unverified here and may require additional GPUI system dependencies and an installed monospace font.

The macOS build enables GPUI's runtime shader compilation, so a separate Xcode Metal compiler toolchain is not required.

Release builds optimize for size (`opt-level = "s"`) with full link-time optimization, one codegen unit, and stripped symbols. Full LTO can increase build time; panic unwinding remains enabled. Markdown and syntax-highlighting dependencies enable only the features needed by the editor, retaining bundled grammars and themes.

From the repository root:

```sh
cargo build --release --locked

# Open a directory with an untitled editor buffer.
./target/release/rockdown /path/to/notes

# Open a document.
./target/release/rockdown /path/to/notes/note.md

# Use the current directory.
./target/release/rockdown
```

You can also build and launch in one command:

```sh
cargo run --release -- /path/to/notes
```

Passing a nonexistent filename starts an empty buffer at that path, provided its parent directory exists. The file is created when you save it.

### Windows quick start

Extract the Windows release archive, then launch `rockdown.exe`, or run it from PowerShell with a file or notes directory:

```powershell
.\rockdown.exe "$HOME\Documents\Notes"
```

To build from source, open **Developer PowerShell for Visual Studio** in the repository root:

```powershell
rustup default stable-x86_64-pc-windows-msvc
# Select the shader compiler from the SDK initialized by Developer PowerShell.
$env:GPUI_FXC_PATH = Join-Path $env:WindowsSdkVerBinPath 'x64\fxc.exe'
cargo build --release --locked
.\target\release\rockdown.exe "$HOME\Documents\Notes"
```

If `WindowsSdkVerBinPath` is unavailable, set `GPUI_FXC_PATH` to the installed SDK's `fxc.exe`, usually under `C:\Program Files (x86)\Windows Kits\10\bin\<SDK version>\x64\`. No Unix shell or WSL is required. Use a directory that already exists.

Windows defaults to **Consolas**, reads settings from `%APPDATA%\rockdown\config.toml`, and starts `%COMSPEC%` (normally Command Prompt) in the terminal. To use PowerShell, set `shell = 'powershell.exe'` or `shell = 'C:\Program Files\PowerShell\7\pwsh.exe'` in your TOML config. Single-quoted TOML strings keep Windows backslashes literal. The shell setting accepts an executable path, not arguments.

Use **Ctrl-C / Ctrl-V** for clipboard operations in the editor and explorer, and **Ctrl-Shift-V** to paste in the terminal. Terminal **Ctrl-C** remains an interrupt. Type `exit` to close Command Prompt or PowerShell.

### Command-line options

```text
rockdown [FILE|DIRECTORY] [--config PATH] [--check-config]
```

| Option | Purpose |
| --- | --- |
| `--config PATH` | Load an explicitly selected `.toml` configuration. |
| `--check-config` | Validate configuration and exit without opening a window. |
| `--help`, `-h` | Print usage information. |

## First note in one minute

1. Launch Rockdown in your notes directory.
2. The editor starts in **Normal mode**. Press `i` to enter **Insert mode** and start typing.
3. Press Return to insert a newline. Press `Esc` to return to Normal mode.
4. Type `:w note.md` and press Return to save your note.
5. Press `Ctrl-W`, then `l`, to focus the file explorer. Select a file with `j` or `k` and press Return to open it.
6. Use the buffer tabs or `Ctrl-PageUp` / `Ctrl-PageDown` to switch documents.
7. Press Ctrl + backtick to open the terminal dock.

Press **F1** or enter `:help` for the in-app keyboard guide. Press `Esc` to dismiss it.

## Focus and global shortcuts

`Cmd` refers to the macOS Command key. For sequences such as `Ctrl-W h`, press `Ctrl-W`, release it, then press `h`.

| Shortcut | Action |
| --- | --- |
| `Cmd-S` / `Ctrl-S` | Save the focused editor buffer, or commit explorer changes when the explorer is focused. |
| `Ctrl-E` / `Cmd-E` | Hide or show the file explorer. Showing it also focuses it. |
| `Ctrl-W h` | Focus the editor. |
| `Ctrl-W l` | Reveal and focus the explorer without toggling it closed. |
| `Ctrl-W j` | Reveal and focus the terminal. |
| `Cmd-1` / `Ctrl-1` | Focus the editor. |
| `Ctrl-Shift-V` | Paste from the clipboard in any pane. |
| Ctrl + backtick | Toggle the terminal dock. |
| `Ctrl-PageUp` / `Ctrl-PageDown` | Previous / next editor buffer. |
| `Cmd-W` / `Ctrl-Shift-W` | Close the current editor buffer, refusing unsaved changes. |
| `F1` | Toggle the keyboard guide. |

You can also click a pane to focus it. The **terminal icon** is at the bottom-left; the **folder** and **circled question-mark** icons at the bottom-right toggle Files and Help. Hover for a tooltip identifying the action; icons stay highlighted while their dock or guide is open. Footer buttons, buffer tabs, and tab-close controls have distinct hover and pressed highlights. The macOS title bar centers **Rockdown — {buffer name}** and updates when you switch or save a buffer under a new name.

## Markdown editing

### Modes and common keys

| Keys | Action |
| --- | --- |
| `i`, `a`, `I`, `A` | Insert before/after the cursor or at the beginning/end of the line. |
| `o`, `O` | Open a line below/above and enter Insert mode. |
| `Esc` | Return to Normal mode. |
| `v` | Enter Visual mode for character selections. |
| `h`, `j`, `k`, `l` or arrow keys | Move the cursor. |
| `w`, `b`, `e` | Move by words. |
| `0`, `$` | Move to the beginning/end of the line. |
| `gg`, `G` | Move to the beginning/end of the document. |
| `Ctrl-D`, `Ctrl-U` | Move cursor and viewport down/up by half a pane. A count sets the number of lines for subsequent half-page motions in that buffer. |
| `zz`, `zt` | Center the current line or place it at the top without moving the cursor. A count first selects that line, e.g. `40zz`. |
| `x`, `dd`, `dw`, `d$` | Delete a character, line, word, or through the end of the line. |
| `cc`, `cw` | Change a line or word and enter Insert mode. |
| `yy`, `p`, `P` | Yank a line; paste after/before the cursor. |
| `u`, `Ctrl-R` | Undo / redo. |
| `/text`, then Return | Search forward for literal text. |
| `n` | Find the next match. |

Counts work with supported motions and operators, such as `3j` or `2dd`. In Visual mode, use `y`, `d`, or `c` to yank, delete, or change the selection. `Ctrl-C` copies a visual selection to the system clipboard; `Ctrl-V` pastes clipboard text. On macOS, use `Cmd-C` / `Cmd-V` instead. `Ctrl-Shift-V` pastes on all platforms.

The editor and file explorer share the **system clipboard** for Vim operations: `y`/`yy` copy text, and `p`/`P` paste the current clipboard after/before the cursor. This also works across buffers and after explorer navigation or refresh. Linewise yanks paste as whole lines; characterwise yanks stay inline. Text copied from another application replaces the previous yank. As with Vim's unnamed register, delete/change operations also copy the removed text. Explorer yanks copy the displayed filename, not the file's contents.

**Return behavior:** in Insert mode, Return splits the line at the caret and moves the cursor to the new line. In Normal mode, it opens a line below and enters Insert mode. In the command line, Return executes the command.

### Whole-buffer substitution

From Normal mode, use `:%s/pattern/replacement/flags` in the editor or file explorer:

```vim
:%s/old/new/          " First match on each line
:%s/old/new/g         " All matches on each line
:%s/old/new/gi        " All matches, ignoring case
:%s#old/path#new/path#g
:%s/(word)/[\1]/g     " Capture references; & inserts the whole match
```

The comments above explain the examples; do not include them in the command. Patterns use **Rust regex syntax** (for example, `(group)`, `\d+`, and `^`/`$`), not Vim's regex dialect. Matching is per physical line. `g` replaces all matches per line; `i` ignores case, and `I` forces case-sensitive matching. Escape a delimiter with `\`; use `\&` for a literal ampersand, `\\` for a backslash, and `\r` or `\n` to insert a newline. The final delimiter is optional when no flags are supplied.

One `u` undoes the entire substitution; `Ctrl-R` redoes it. Invalid patterns, unsupported flags (including interactive `c` confirmation), and missing matches report an error without changing the buffer. An empty pattern is rejected rather than reusing a previous search. In the explorer, replacements stage filename edits; `:w` commits them with the usual safety checks.

### Saving, opening, and quitting

Enter these commands from Normal mode. Focus the editor first with `Ctrl-W h`; `:w` and `:e` have different meanings in the explorer.

| Command | Action |
| --- | --- |
| `:w` | Save the current document. |
| `:w filename.md` | Save under the given filename. |
| `:w! [filename.md]` | Explicitly overwrite a disk conflict or existing destination. |
| `:e filename.md` | Open an existing file, or activate its existing buffer. Other buffers keep their edits. |
| `:e` | Reload the current document, refusing unsaved changes. |
| `:e!` | Discard current edits and reload from disk. |
| `:q` | Close the window only if all editor buffers and the explorer are clean. |
| `:q!` | Close the window and discard all unsaved/staged changes. |
| `:wq` | Save the focused buffer, then close only if no other unsaved changes remain. |

Relative paths in `:e` and `:w` are resolved against the **current explorer directory**. Saving does not create missing parent directories.

Saves check for external file changes instead of silently overwriting them. If a conflict is reported, inspect the disk version before choosing `:e!` to reload or `:w!` to overwrite. Saving over a file owned by another open buffer is refused even with `:w!`.

Existing LF or CRLF line endings and UTF-8 BOMs are preserved when saving. Editing and clipboard paste use normalized newlines internally. New documents without an existing newline style use CRLF on Windows and LF elsewhere. Mixed-ending files are normalized to the first newline's style on save.

## Buffer management

An **editor buffer** is an open document, not a file explorer entry. Closing a buffer never deletes its file from disk.

Click a tab to select a document, or its **×** to close it. Closing an inactive tab leaves the selected document active. A `[+]` marker indicates unsaved changes; the close control refuses to discard them.

| Action | Commands |
| --- | --- |
| Previous buffer | `:bp`, `:bprevious`, `:previous-buffer` |
| Next buffer | `:bn`, `:bnext`, `:next-buffer` |
| Close current buffer | `:bd`, `:bdelete`, `:buffer-delete` |
| Discard edits and close | `:bd!`, `:bdelete!`, `:buffer-delete!` |

Navigation wraps in opening order. Opening an already-open file activates its existing buffer rather than creating a duplicate. Closing the final buffer leaves a fresh, empty untitled buffer.

Unsaved buffers remain protected even when inactive: switching is allowed, but a normal buffer close or window quit will not discard their edits.

## File explorer

The right dock is an **editable listing of filenames**. Its Vim editing commands stage filesystem operations; they do not change files until you commit with `:w`.

### Navigate and resize

- `Ctrl-W l` or `:ex`: reveal and focus the explorer.
- `j` / `k`: select an entry.
- Return in Normal mode: open a file or enter a directory.
- `-` in Normal mode: go to the parent directory.
- `Ctrl-E` / `Cmd-E`, or the bottom-right **folder icon**: toggle visibility.
- Drag the dock's **left edge** to resize it. Width is bounded to leave room for the editor.

Hiding the explorer preserves its staged edits and resized width. Resizing changes the current session's width; it does not write your configuration file.

### Edit filenames like a buffer

| Operation | Steps |
| --- | --- |
| Rename an entry | Select its line, press `cc`, type the replacement name, then `Esc`. |
| Create a file | Press `o`, type a new filename such as `note.md`, then `Esc`. |
| Create a directory | Add a new line whose name ends in `/`, such as `drafts/`. |
| Stage deletion | Select an entry and press `dd`. |
| Apply all staged changes | Enter `:w`. |
| Discard staged changes and reload | Enter `:e!`. |
| Refresh a clean listing | Enter `:e`. |

For example, to rename `draft.md` to `article.md`, select its line, use `cc` to replace the name, press `Esc`, then enter `:w`.

Navigation and ordinary reload refuse pending changes. Commit them, undo them, or explicitly discard them before navigating elsewhere. Duplicate destination names, path traversal, entry-type changes, and detected external filesystem changes are rejected.

**Deletion is recoverable:** committed deletions are moved into a transaction subdirectory under `.rockdown-trash` at the explorer's workspace root. They are not sent to the OS Trash. Recover them by moving the entries back using a terminal or file manager. Trash is hidden from the explorer listing; navigating outside the original workspace can establish a new trash root.

Editing the listing changes names and creates new empty files/directories—it is not a file-content copy interface. Explorer renames and deletions are also reconciled with all open editor buffers.

## Terminal dock

Click the bottom-left **terminal icon**, press Ctrl + backtick, or enter `:term` to toggle the bottom terminal dock. It starts a real shell in the explorer's directory **at the time the session is created**.

- Return executes the command.
- `Ctrl-C` interrupts a foreground command.
- `Ctrl-D` sends end-of-input in Unix shells; at an empty Unix shell prompt this usually exits the shell. In Windows shells, use `exit` instead.
- `Ctrl-Shift-V` (or `Cmd-V` on macOS) pastes, using bracketed paste when the terminal application enables it.
- `Ctrl-W h` / `Ctrl-W l` moves focus back to the editor / explorer.

Hiding the dock keeps the shell session running. When the shell exits—through `exit` or `Ctrl-D` at an empty prompt—the dock closes automatically. If the terminal had focus, focus returns to the editor; otherwise the current pane keeps focus. Opening the dock again starts a fresh session. `Ctrl-D` handled by a foreground program does not close the dock while the shell is still running. Navigating the explorer does not automatically change an already-running shell's working directory. Closing the application terminates its terminal session.

## Configuration

Rockdown loads configuration in this order:

1. The file explicitly supplied with `--config PATH`.
2. Otherwise, `config.toml` under `$XDG_CONFIG_HOME/rockdown/`.
3. If `XDG_CONFIG_HOME` is unset or empty, use `%APPDATA%\rockdown\` on Windows (falling back to `%USERPROFILE%\AppData\Roaming\rockdown\`), or `~/.config/rockdown/` on Unix.
4. If no config file exists, built-in defaults are used.

An invalid selected config produces an error on startup or reload. Project-local config is not loaded automatically.

### TOML example

```toml
font_family = "Menlo"
font_size = 16
line_height = 30
explorer_width = 300
terminal_height = 240
shell = "/bin/zsh"

[theme]
background = "#171b22"
panel = "#1e242e"
foreground = "#dce3ec"
muted = "#8995a7"
accent = "#9cc7b5"
```

A complete example, including the default shortcut map, is provided in [`examples/config.toml`](examples/config.toml).

### Available settings

| Setting | Default | Notes |
| --- | --- | --- |
| `font_family` | `Consolas` on Windows, `Menlo` elsewhere | Use a font installed on your system. |
| `font_size` | `15` | Range: 10–32. |
| `line_height` | `30` | At least `font_size + 4`, at most 64. |
| `explorer_width` | `290` | Configured range: 180–600; display width also respects window size. |
| `terminal_height` | `240` | Range: 100–600. |
| `shell` | Windows: `%COMSPEC%`, then `cmd.exe`; Unix: `$SHELL`, then `/bin/sh` | Shell executable, not a command string with arguments. |
| `theme` | Colors shown above | Six-digit RGB hex colors, with or without `#`. |
| `markdown` | Inherited colors and built-in heading sizes | Nested appearance settings described below. |
| `keys` | Built-in shortcuts | Maps GPUI keystrokes to action names. |

Unknown settings, unsupported action names, and invalid values are rejected.

### Markdown appearance

The optional `markdown` table customizes appearance. Existing configs need no changes: omitted tables and fields retain their defaults, including partial settings within a heading or color table.

| Table | Fields | Defaults and limits |
| --- | --- | --- |
| `markdown.colors` | `normal`, `bold`, `italic`, `bold_italic`, `code`, `link`, `strikethrough`, `quote` | Optional six-digit RGB hex colors, with or without `#`. |
| `markdown.h1` through `markdown.h6` | `font_size`, `color`, `underline` | Each level is independent. Optional `font_size`: 10–128; optional RGB `color`; `underline`: `false`. |
| `markdown.divider` | `color`, `thickness` | Optional RGB `color`; `thickness`: `1`, range 0.5–12. |

`normal` inherits `theme.foreground` when omitted. It sets the editor's base text color, including the active line's raw source and plain-text documents; explorer text continues to use the theme. `code` and `link` default to `theme.accent`. Other unset colors inherit the applicable heading or quote color, then `normal`. Heading and quote colors themselves inherit `normal`; divider color inherits `theme.muted`.

Color precedence, highest first, is **syntax highlighting → code → link → bold-italic → bold → italic → strikethrough → heading/quote → normal**. A missing optional override leaves the inherited color in place rather than masking a lower-priority color.

All six heading levels can override their size, color, and underline separately. An omitted heading size uses the built-in scaling relative to `font_size`. Larger heading sizes automatically expand rows so content is not clipped; you do not need to increase `line_height` to fit them. `underline = true` draws a full-width line below that heading's content. Heading underlines and horizontal rules share `markdown.divider.color` and `markdown.divider.thickness`.

For example, these TOML settings enlarge and underline first-level headings while leaving other sizes and colors inherited:

```toml
[markdown.colors]
bold = "#f2cf8f"
link = "#8fc8ed"

[markdown.h1]
font_size = 36
underline = true

[markdown.divider]
color = "#566575"
thickness = 1.5
```

Font sizes and thicknesses must be finite; invalid colors, out-of-range values, and unknown nested fields are rejected. Use `:config` after saving the selected config to reload appearance without restarting; row sizes are recalculated in every pane.

### Custom shortcuts

Supported action names are:

```text
save
paste
explorer
terminal
editor
help
buffer-delete
previous-buffer
next-buffer
```

`explorer` and `terminal` toggle their docks. `buffer-delete` is the safe close action; forced discard remains an explicit command such as `:bd!`.

**Defining `[keys]` in TOML replaces the entire default shortcut map; it does not merge individual entries.** Copy the full map from an example file and modify it if you want to retain the other defaults. Vim editing keys and colon commands remain available independently of that map.

Validate a config before launching:

```sh
./target/release/rockdown --check-config --config examples/config.toml
```

Run with an explicit config:

```sh
./target/release/rockdown /path/to/notes --config /path/to/config.toml
```

Use `:config` to reload settings in the app. A changed shell setting applies to the next terminal session, not the currently running one.

## Current boundaries

- Vim support is a focused set of editing commands, not a complete Vim or Neovim implementation.
- Markdown rendering is line-based live preview, not a separate rich-text document format.
- Local images render in place. Remote image URLs remain alt text; opening a document does not fetch them over the network.
- Files must be UTF-8. The explorer rejects filenames it cannot represent safely as individual text lines.
- There is no automatic document save or restoration of open buffers across application restarts.

## Development checks

From the repository root:

```sh
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

Tests cover Vim/Unicode editing, document save conflicts, buffer lifecycle, explorer operations and recovery, Markdown projection, configuration, keyboard normalization, and terminal behavior. Tests are not a substitute for native-window interaction checks when changing the UI.

After rebuilding, restart any already-running Rockdown instance to use the updated executable.

### Document dialogs

Use **Ctrl/Cmd-N** for a new document, **Ctrl/Cmd-O** to open a file,
**Ctrl/Cmd-S** to save, and **Ctrl/Cmd-Shift-S** for Save As. Untitled documents
open a native save dialog. These actions also appear in the native File menu
where the platform supports menus. Configure them using the `new`, `open`,
`save`, and `save-as` key actions. A dot beside a document name indicates unsaved
changes; the footer confirms saves and reports cancellation or conflicts.

Closing a tab offers Save / Discard / Cancel. Window close (including `:q`)
checks every dirty document, then explicitly asks about staged Files operations.
Cancelling any prompt keeps the window and its documents open. Earlier successful
saves remain saved; discarded buffers are retained until the entire close flow
is accepted. Saving staged Files changes applies renames, creations, and moves
to trash, just as `:w` in Files does.

Colon commands remain available. In Files, Ctrl/Cmd-S still applies staged
operations; Save As always targets the active document. Save As preserves file
conflict and buffer-ownership checks: choosing an existing unrelated file reports
a conflict rather than replacing it. Explicit `:w! path` is available for
intentional overwrites. `:q!` continues to explicitly discard everything.
