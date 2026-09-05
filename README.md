# Rockdown

A lightweight, native Markdown workspace built with **Rust and GPUI**. Rockdown combines in-place Markdown rendering, Vim-style editing, a buffer-editable file explorer, and an embedded terminal—without a browser or Electron runtime.

## What it does

- **Live-preview Markdown:** headings, emphasis, code, lists, task lists, tables, quotes, wrapped prose, and local images.
- **Vim-style editing:** Normal, Insert, and Visual modes; motions, counts, operators, search, and undo/redo.
- **Multiple buffers:** switch between documents without losing unsaved text, cursor position, viewport, or undo history.
- **Right-hand file explorer:** rename, create, and stage deletions by editing filenames as buffer lines. Hide the dock or drag its left edge to resize it.
- **Embedded terminal:** a real PTY shell with color support, keyboard input, and resizing.
- **TOML or Lua configuration:** customize fonts, colors, dock dimensions, shell, and shortcuts.

Rockdown uses **line-based live preview**, rather than a separate preview pane: inactive lines render as Markdown, while the active line exposes its source syntax for editing. Files remain plain UTF-8 text on disk.

## Build and run

### Requirements

- A recent Rust toolchain with Cargo and Rust 2024 edition support.
- Native development tools required by GPUI and its dependencies.
- On macOS, Xcode Command Line Tools. Install them with `xcode-select --install` if needed.

**Platform status:** development and native-window verification have been performed on macOS Apple Silicon. The code includes Linux PTY and filesystem support, but Linux builds and UI behavior have not been verified here. Linux may require additional GPUI system dependencies and an installed monospace font. Windows is not currently supported by the embedded terminal and explorer commit implementation.

The macOS build enables GPUI's runtime shader compilation, so a separate Xcode Metal compiler toolchain is not required. Lua is built through the vendored dependency; a separate Lua installation is not required.

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

### Command-line options

```text
rockdown [FILE|DIRECTORY] [--config PATH] [--check-config]
```

| Option | Purpose |
| --- | --- |
| `--config PATH` | Load an explicitly selected `.toml` or `.lua` configuration. |
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
| `Cmd-1` | Focus the editor. |
| Ctrl + backtick | Toggle the terminal dock. |
| `Ctrl-PageUp` / `Ctrl-PageDown` | Previous / next editor buffer. |
| `Cmd-W` / `Ctrl-Shift-W` | Close the current editor buffer, refusing unsaved changes. |
| `F1` | Toggle the keyboard guide. |

You can also click a pane to focus it. **Terminal** is at the bottom-left; **Hide Files / Show Files** and **Help** are at the bottom-right. The macOS title bar centers **Rockdown — {buffer name}** and updates when you switch or save a buffer under a new name.

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
| `x`, `dd`, `dw`, `d$` | Delete a character, line, word, or through the end of the line. |
| `cc`, `cw` | Change a line or word and enter Insert mode. |
| `yy`, `p`, `P` | Yank a line; paste after/before the cursor. |
| `u`, `Ctrl-R` | Undo / redo. |
| `/text`, then Return | Search forward for literal text. |
| `n` | Find the next match. |

Counts work with supported motions and operators, such as `3j` or `2dd`. In Visual mode, use `y`, `d`, or `c` to yank, delete, or change the selection. `Cmd-C` copies a visual selection to the system clipboard; `Cmd-V` pastes clipboard text.

The editor and file explorer share the **system clipboard** for Vim operations: `y`/`yy` copy text, and `p`/`P` paste the current clipboard after/before the cursor. This also works across buffers and after explorer navigation or refresh. Linewise yanks paste as whole lines; characterwise yanks stay inline. Text copied from another application replaces the previous yank. As with Vim's unnamed register, delete/change operations also copy the removed text. Explorer yanks copy the displayed filename, not the file's contents.

**Return behavior:** in Insert mode, Return splits the line at the caret and moves the cursor to the new line. In Normal mode, it opens a line below and enters Insert mode. In the command line, Return executes the command.

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
- `Ctrl-E` / `Cmd-E`, or **Hide Files / Show Files**: toggle visibility.
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

Press Ctrl + backtick, or enter `:term`, to toggle the bottom terminal dock. It starts a real shell in the explorer's directory **at the time the session is created**.

- Return executes the command.
- `Ctrl-C` interrupts a foreground command.
- `Ctrl-D` sends end-of-input; at an empty shell prompt this usually exits the shell.
- `Cmd-V` pastes, using bracketed paste when the terminal application enables it.
- `Ctrl-W h` / `Ctrl-W l` moves focus back to the editor / explorer.

Hiding the dock keeps the shell session running. Toggling an exited terminal starts a new session. Navigating the explorer does not automatically change an already-running shell's working directory. Closing the application terminates its terminal session.

## Configuration

Rockdown loads configuration in this order:

1. The file explicitly supplied with `--config PATH`.
2. Otherwise, `config.toml` or `config.lua` under `$XDG_CONFIG_HOME/rockdown/`.
3. If `XDG_CONFIG_HOME` is unset, that directory is `~/.config/rockdown/`.
4. If neither default file exists, built-in defaults are used.

**TOML wins when both default files exist.** An invalid selected config produces an error rather than falling back to the other file.

Project-local config is not loaded automatically. Lua configuration executes code and must return a settings table; only load Lua files you trust.

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

### Equivalent Lua configuration

```lua
return {
  font_family = "Menlo",
  font_size = 16,
  line_height = 30,
  explorer_width = 300,
  terminal_height = 240,
  shell = "/bin/zsh",
  theme = {
    background = "#171b22",
    panel = "#1e242e",
    foreground = "#dce3ec",
    muted = "#8995a7",
    accent = "#9cc7b5",
  },
}
```

Complete examples, including the default shortcut map, are provided in [`examples/config.toml`](examples/config.toml) and [`examples/config.lua`](examples/config.lua).

### Available settings

| Setting | Default | Notes |
| --- | --- | --- |
| `font_family` | `Menlo` | Use a font installed on your system. |
| `font_size` | `15` | Range: 10–32. |
| `line_height` | `30` | At least `font_size + 4`, at most 64. |
| `explorer_width` | `290` | Configured range: 180–600; display width also respects window size. |
| `terminal_height` | `240` | Range: 100–600. |
| `shell` | `$SHELL`, then `/bin/sh` | Shell executable, not a command string with arguments. |
| `theme` | Colors shown above | Six-digit RGB hex colors, with or without `#`. |
| `keys` | Built-in shortcuts | Maps GPUI keystrokes to action names. |

Unknown settings, unsupported action names, and invalid values are rejected.

### Custom shortcuts

Supported action names are:

```text
save
explorer
terminal
editor
help
buffer-delete
previous-buffer
next-buffer
```

`explorer` and `terminal` toggle their docks. `buffer-delete` is the safe close action; forced discard remains an explicit command such as `:bd!`.

**Defining `[keys]` in TOML or `keys = { ... }` in Lua replaces the entire default shortcut map; it does not merge individual entries.** Copy the full map from an example file and modify it if you want to retain the other defaults. Vim editing keys and colon commands remain available independently of that map.

Validate a config before launching:

```sh
./target/release/rockdown --check-config --config examples/config.toml
./target/release/rockdown --check-config --config examples/config.lua
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

![](R0009272.JPG "800px")