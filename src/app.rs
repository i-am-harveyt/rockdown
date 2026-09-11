use crate::surface::{Surface, SurfaceLayout};
use crate::{
    config::{Config, parse_color},
    document::Document,
    explorer::Explorer,
    markdown::{self, RenderedLine},
    terminal::{Terminal, key_bytes},
    vim::{Buffer, Mode},
};
use anyhow::{Result, bail};
use gpui::{prelude::*, *};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    rockdown,
    [
        Save,
        Paste,
        ExplorerToggle,
        TerminalToggle,
        EditorPane,
        HelpToggle,
        PreviousBuffer,
        NextBuffer,
        BufferDelete
    ]
);

/// Bind the configured shortcut map through GPUI's keymap. Bound actions reach
/// the app even when macOS routes a key equivalent (like Cmd-V) through the
/// input context instead of delivering a plain key-down event.
pub fn bind_config_keys(config: &Config, cx: &mut App) {
    cx.bind_keys(config.keys.iter().filter_map(|(key, action)| {
        let action: Box<dyn Action> = match action.as_str() {
            "save" => Box::new(Save),
            "paste" => Box::new(Paste),
            "explorer" => Box::new(ExplorerToggle),
            "terminal" => Box::new(TerminalToggle),
            "editor" => Box::new(EditorPane),
            "help" => Box::new(HelpToggle),
            "previous-buffer" => Box::new(PreviousBuffer),
            "next-buffer" => Box::new(NextBuffer),
            "buffer-delete" => Box::new(BufferDelete),
            _ => return None,
        };
        KeyBinding::load(key, action, None, false, None, &DummyKeyboardMapper).ok()
    }));
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Editor,
    Explorer,
    Terminal,
}
impl Pane {
    pub fn index(self) -> usize {
        match self {
            Self::Editor => 0,
            Self::Explorer => 1,
            Self::Terminal => 2,
        }
    }
}

pub struct Workspace {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub documents: crate::documents::Documents,
    pub explorer: Explorer,
    pub explorer_visible: bool,
    explorer_resize: Option<(Pixels, f32)>,
    pub terminal: Option<Terminal>,
    pub terminal_visible: bool,
    terminal_task: Option<Task<()>>,
    pub pane: Pane,
    pub focus: FocusHandle,
    pub layouts: [SurfaceLayout; 3],
    pub tops: [usize; 3],
    pub scroll_offsets: [f32; 3],
    pub row_heights: [Vec<f32>; 3],
    pub projection: Vec<RenderedLine>,
    pub command: Option<String>,
    pub message: String,
    pub help: bool,
    pub marked: Option<Range<usize>>,
    search: String,
    window_prefix: bool,
    pub follow_cursor: bool,
    pane_heights: [f32; 3],
    viewport_alignment: Option<(Pane, bool)>,
}

impl Workspace {
    pub fn new(
        config: Config,
        config_path: Option<PathBuf>,
        document: Document,
        explorer: Explorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        bind_config_keys(&config, cx);
        let projection = if document.is_markdown() {
            markdown::project(&document.buffer.text())
        } else {
            Vec::new()
        };
        let explorer_visible = document.path.is_none();
        Self {
            config,
            config_path,
            documents: crate::documents::Documents::new(document),
            explorer,
            explorer_visible,
            explorer_resize: None,
            terminal: None,
            terminal_visible: false,
            terminal_task: None,
            pane: Pane::Editor,
            focus,
            layouts: Default::default(),
            tops: [0; 3],
            scroll_offsets: [0.; 3],
            row_heights: Default::default(),
            projection,
            command: None,
            message: if explorer_visible {
                "i to write  ·  :w filename.md to save  ·  F1 help"
            } else {
                "i to write  ·  :w to save  ·  F1 help"
            }
            .into(),
            help: false,
            marked: None,
            search: String::new(),
            window_prefix: false,
            follow_cursor: true,
            pane_heights: [0.; 3],
            viewport_alignment: None,
        }
    }
    pub fn color(&self, value: &str) -> Hsla {
        rgb(parse_color(value).expect("validated theme")).into()
    }
    pub fn buffer(&self) -> &Buffer {
        if self.pane == Pane::Explorer {
            &self.explorer.buffer
        } else {
            &self.documents.current().buffer
        }
    }
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        if self.pane == Pane::Explorer {
            &mut self.explorer.buffer
        } else {
            &mut self.documents.current_mut().buffer
        }
    }
    pub fn refresh_projection(&mut self) {
        let document = self.documents.current();
        self.projection = if document.is_markdown() {
            markdown::project(&document.buffer.text())
        } else {
            Vec::new()
        };
        self.row_heights[Pane::Editor.index()].clear();
    }
    fn change_document(
        &mut self,
        change: impl FnOnce(&mut crate::documents::Documents) -> Result<()>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let editor = Pane::Editor.index();
        let previous = self.documents.active_id();
        *self.documents.viewport_mut() = crate::documents::Viewport {
            top: self.tops[editor],
            offset: self.scroll_offsets[editor],
        };
        if let Err(error) = change(&mut self.documents) {
            self.documents.select(previous)?;
            return Err(error);
        }
        self.refresh_projection();
        self.layouts[editor] = SurfaceLayout::default();
        self.tops[editor] = self.documents.viewport().top;
        self.scroll_offsets[editor] = self.documents.viewport().offset;
        self.set_pane(Pane::Editor, cx);
        self.follow_cursor = false;
        self.message = format!(
            "Buffer {} · {} open",
            self.documents.active_id(),
            self.documents.entries().len()
        );
        Ok(())
    }
    pub fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.documents.dirty() || self.explorer.dirty() {
            self.message =
                "Unsaved changes. Save with :w, or discard all and close with :q!".into();
            cx.notify();
            false
        } else {
            true
        }
    }
    fn set_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.viewport_alignment = None;
        if pane == Pane::Explorer {
            self.explorer_visible = true;
        }
        self.pane = pane;
        self.command = None;
        self.marked = None;
        self.follow_cursor = true;
        cx.notify();
    }
    fn toggle_explorer(&mut self, cx: &mut Context<Self>) {
        self.explorer_resize = None;
        if self.explorer_visible {
            self.explorer_visible = false;
            if self.pane == Pane::Explorer {
                self.set_pane(Pane::Editor, cx);
            }
        } else {
            self.set_pane(Pane::Explorer, cx);
        }
        cx.notify();
    }
    fn explorer_width(&self, window: &Window) -> f32 {
        Self::clamp_explorer_width(self.config.explorer_width, window)
    }
    fn clamp_explorer_width(width: f32, window: &Window) -> f32 {
        let max = (f32::from(window.viewport_size().width) - 320.).clamp(0., 600.);
        width.clamp(180_f32.min(max), max)
    }
    pub fn start_explorer_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_resize = Some((event.position.x, self.explorer_width(window)));
        cx.stop_propagation();
        cx.notify();
    }
    pub fn resize_explorer(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start_x, start_width)) = self.explorer_resize else {
            return;
        };
        if !self.explorer_visible || !event.dragging() {
            self.stop_explorer_resize(cx);
            return;
        }
        self.config.explorer_width =
            Self::clamp_explorer_width(start_width + f32::from(start_x - event.position.x), window);
        cx.notify();
    }
    pub fn stop_explorer_resize(&mut self, cx: &mut Context<Self>) {
        if self.explorer_resize.take().is_some() {
            cx.notify();
        }
    }
    fn toggle_terminal(&mut self, cx: &mut Context<Self>) -> Result<()> {
        if self.terminal.is_none() {
            self.terminal = Some(Terminal::spawn(
                &self.explorer.directory,
                &self.config.shell,
                12,
                100,
            )?);
            self.terminal_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    Timer::after(Duration::from_millis(16)).await;
                    let running = this
                        .update(cx, |this, cx| {
                            let Some(terminal) = &mut this.terminal else {
                                return false;
                            };
                            if terminal.poll() {
                                if let Some(error) = terminal.error() {
                                    this.message = error.to_string();
                                }
                                if this.terminal_visible {
                                    cx.notify();
                                }
                            }
                            if terminal.exited() {
                                this.terminal_visible = false;
                                this.terminal = None;
                                if this.pane == Pane::Terminal {
                                    this.set_pane(Pane::Editor, cx);
                                }
                                cx.notify();
                                return false;
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !running {
                        break;
                    }
                }
            }));
            self.terminal_visible = true;
        } else {
            self.terminal_visible = !self.terminal_visible;
        }
        self.set_pane(
            if self.terminal_visible {
                Pane::Terminal
            } else {
                Pane::Editor
            },
            cx,
        );
        Ok(())
    }
    fn save(&mut self, path: Option<&Path>, force: bool) -> Result<()> {
        if self.pane == Pane::Explorer {
            if path.is_some() || force {
                bail!("Explorer commits use :w; :e! discards staged changes");
            }
            let report = self.explorer.commit()?;
            self.documents.reconcile(&report);
            self.refresh_projection();
            self.message = format!(
                "Explorer saved: {} created, {} renamed, {} moved to .rockdown-trash",
                report.created.len(),
                report.renamed.len(),
                report.deleted.len()
            );
        } else {
            let target = path.map(|p| {
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    self.explorer.directory.join(p)
                }
            });
            self.documents.save(target.as_deref(), force)?;
            self.refresh_projection();
            self.message = format!(
                "Saved {}",
                self.documents.current().path.as_ref().unwrap().display()
            );
            if !self.explorer.dirty() {
                self.explorer.reload()?;
            }
        }
        Ok(())
    }
    /// Dispatch a named action from a keymap binding, surfacing failures in
    /// the status bar instead of dropping them.
    fn run_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.action(action, window, cx) {
            self.message = format!("{error:#}");
            cx.notify();
        }
    }
    fn action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        match action {
            "save" => self.save(None, false)?,
            "explorer" => self.toggle_explorer(cx),
            "terminal" => self.toggle_terminal(cx)?,
            "editor" => self.set_pane(Pane::Editor, cx),
            "help" => self.help = !self.help,
            "previous-buffer" => self.change_document(
                |documents| {
                    documents.previous();
                    Ok(())
                },
                cx,
            )?,
            "next-buffer" => self.change_document(
                |documents| {
                    documents.next();
                    Ok(())
                },
                cx,
            )?,
            "buffer-delete" => self.change_document(|documents| documents.delete(false), cx)?,
            "paste" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.type_text(&text);
                }
            }
            _ => {}
        }
        self.focus.focus(window);
        cx.notify();
        Ok(())
    }
    fn find_next(&mut self, first: bool) {
        if self.search.is_empty() {
            return;
        }
        let query = self.search.clone();
        let buffer = self.buffer_mut();
        let row = buffer.row;
        let start = if first {
            buffer.col
        } else {
            buffer.lines[row].text[buffer.col..]
                .graphemes(true)
                .next()
                .map_or(buffer.col, |g| buffer.col + g.len())
        };
        for offset in 0..=buffer.lines.len() {
            let candidate = (row + offset) % buffer.lines.len();
            let text = &buffer.lines[candidate].text;
            let from = if offset == 0 {
                start.min(text.len())
            } else {
                0
            };
            if let Some(index) = text[from..].find(&query) {
                buffer.row = candidate;
                buffer.col = from + index;
                self.follow_cursor = true;
                return;
            }
        }
        self.message = format!("Pattern not found: {query}");
    }
    fn execute(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.follow_cursor = true;
        if let Some(query) = command.strip_prefix('/') {
            self.search = query.into();
            self.find_next(true);
            return Ok(());
        }
        let command = command.trim_start_matches(':').trim_start();
        if let Some(substitution) = command.strip_prefix("%s") {
            let count = self.buffer_mut().substitute(substitution)?;
            if self.pane == Pane::Editor {
                self.refresh_projection();
            }
            self.message = format!("{count} substitution{}", if count == 1 { "" } else { "s" });
            return Ok(());
        }
        let command = command.trim_end();
        let (verb, arg) = command
            .split_once(char::is_whitespace)
            .map_or((command, ""), |(a, b)| (a, b.trim()));
        match verb {
            "w" | "w!" => self.save((!arg.is_empty()).then(|| Path::new(arg)), verb == "w!")?,
            "wq" => {
                self.save((!arg.is_empty()).then(|| Path::new(arg)), false)?;
                if self.can_close(cx) {
                    window.remove_window();
                }
            }
            "q" => {
                if self.can_close(cx) {
                    window.remove_window();
                }
            }
            "q!" => window.remove_window(),
            "bp" | "bprevious" | "previous-buffer" => self.change_document(
                |documents| {
                    documents.previous();
                    Ok(())
                },
                cx,
            )?,
            "bn" | "bnext" | "next-buffer" => self.change_document(
                |documents| {
                    documents.next();
                    Ok(())
                },
                cx,
            )?,
            "bd" | "bdelete" | "bd!" | "bdelete!" | "buffer-delete" | "buffer-delete!" => {
                self.change_document(|documents| documents.delete(verb.ends_with('!')), cx)?;
            }
            "e" | "e!" => {
                if self.pane == Pane::Explorer {
                    if !arg.is_empty() {
                        bail!("Use Enter to navigate the explorer");
                    }
                    if verb == "e!" {
                        self.explorer = Explorer::open(&self.explorer.directory)?;
                    } else {
                        self.explorer.reload()?;
                    }
                } else {
                    let path = self.explorer.directory.join(arg);
                    self.change_document(
                        |documents| {
                            if arg.is_empty() {
                                documents.reload(verb == "e!")
                            } else {
                                documents.open(&path)?;
                                if verb == "e!" {
                                    documents.reload(true)?;
                                }
                                Ok(())
                            }
                        },
                        cx,
                    )?;
                }
                if self.pane == Pane::Explorer {
                    self.tops[Pane::Explorer.index()] = 0;
                    self.scroll_offsets[Pane::Explorer.index()] = 0.;
                }
            }
            "term" | "terminal" => self.toggle_terminal(cx)?,
            "ex" | "explorer" => self.set_pane(Pane::Explorer, cx),
            "help" => self.help = true,
            "config" => {
                let (config, path) = Config::load(self.config_path.as_deref())?;
                bind_config_keys(&config, cx);
                self.config = config;
                self.config_path = path;
                for heights in &mut self.row_heights {
                    heights.clear();
                }
                self.message = "Configuration reloaded; new shell setting applies to the next terminal session".into();
            }
            "" => {}
            number if number.parse::<usize>().is_ok() => {
                let n = number.parse::<usize>()?;
                let b = self.buffer_mut();
                b.row = n.saturating_sub(1).min(b.lines.len() - 1);
                b.col = 0;
            }
            _ => bail!("Unknown command: {verb}"),
        }
        Ok(())
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_cursor = true;
        self.viewport_alignment = None;
        let result = self.handle_key(event, window, cx);
        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => self.message = format!("{error:#}"),
        }
        if let Some((text, linewise)) = self.buffer_mut().take_yank() {
            cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                text,
                if linewise {
                    "rockdown:lines"
                } else {
                    "rockdown:characters"
                }
                .into(),
            ));
        }
        cx.stop_propagation();
        cx.notify();
    }
    fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool> {
        let stroke = &event.keystroke;
        // Configured shortcuts dispatch through GPUI keymap actions (see
        // bind_config_keys); this handler only covers the hardcoded keys.
        let key = crate::keyboard::key(stroke);
        if stroke.modifiers.platform && stroke.key == "q" {
            if self.can_close(cx) {
                window.remove_window();
            }
            return Ok(true);
        }
        if self.help {
            if stroke.key == "escape" || key == "q" {
                self.help = false;
            }
            return Ok(true);
        }
        let clipboard_shortcut =
            crate::keyboard::clipboard_shortcut(stroke, self.pane == Pane::Terminal);
        if clipboard_shortcut && stroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.type_text(&text);
            }
            return Ok(true);
        }
        if clipboard_shortcut && stroke.key == "c" && self.pane != Pane::Terminal {
            let buffer = self.buffer();
            let selected = buffer
                .lines
                .iter()
                .enumerate()
                .filter_map(|(row, line)| {
                    buffer.selected_range(row).map(|r| line.text[r].to_string())
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected));
            }
            return Ok(true);
        }
        if stroke.modifiers.control && stroke.key == "w" {
            self.window_prefix = true;
            return Ok(true);
        }
        if self.window_prefix {
            self.window_prefix = false;
            match key {
                "h" => self.set_pane(Pane::Editor, cx),
                "l" => self.set_pane(Pane::Explorer, cx),
                "j" => {
                    if !self.terminal_visible {
                        self.toggle_terminal(cx)?;
                    } else {
                        self.set_pane(Pane::Terminal, cx);
                    }
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.pane == Pane::Terminal {
            if stroke.modifiers.platform {
                return Ok(false);
            }
            if crate::keyboard::text(stroke).is_some() {
                return Ok(false);
            }
            let terminal = self.terminal.as_mut().unwrap();
            if let Some(bytes) = key_bytes(
                key,
                stroke.modifiers.control,
                stroke.modifiers.alt,
                terminal.screen().application_cursor(),
            ) {
                terminal.send(&bytes)?;
            }
            return Ok(true);
        }
        if self.command.is_some() {
            match key {
                "escape" => {
                    self.command = None;
                    self.marked = None;
                }
                "enter" => {
                    let command = self.command.take().unwrap();
                    self.marked = None;
                    self.execute(command, window, cx)?;
                }
                "backspace" => {
                    let command = self.command.as_mut().unwrap();
                    if command.len() > 1 {
                        let idx = command.grapheme_indices(true).next_back().unwrap().0;
                        command.truncate(idx);
                    } else {
                        self.command = None;
                    }
                }
                _ => return Ok(stroke.modifiers.control || stroke.modifiers.platform),
            }
            return Ok(true);
        }
        if self.pane == Pane::Editor
            && key == "enter"
            && !stroke.modifiers.control
            && !stroke.modifiers.platform
            && !stroke.modifiers.alt
        {
            let buffer = self.buffer_mut();
            match buffer.mode {
                Mode::Normal => {
                    buffer.key("o");
                }
                Mode::Insert => {
                    buffer.key("enter");
                }
                Mode::Visual => {
                    buffer.key("c");
                    buffer.key("enter");
                }
            }
            self.marked = None;
            self.refresh_projection();
            return Ok(true);
        }
        let insert = self.buffer().mode == Mode::Insert;
        if !insert && !stroke.modifiers.control && !stroke.modifiers.platform {
            if key == ":" || key == "/" {
                self.command = Some(key.into());
                return Ok(true);
            }
            if key == "n" {
                self.find_next(false);
                return Ok(true);
            }
            if self.pane == Pane::Explorer && (key == "enter" || key == "-") {
                if key == "-" {
                    self.explorer.parent()?;
                } else {
                    if let Some(path) = self.explorer.enter()? {
                        self.change_document(|documents| documents.open(&path), cx)?;
                    }
                }
                self.tops[Pane::Explorer.index()] = 0;
                return Ok(true);
            }
        }
        if stroke.modifiers.platform || stroke.modifiers.alt {
            return Ok(false);
        }
        if insert && crate::keyboard::text(stroke).is_some() {
            return Ok(false);
        }
        let key = if stroke.modifiers.control {
            format!("ctrl-{}", stroke.key)
        } else if key == " " {
            " ".into()
        } else {
            key.to_string()
        };
        self.marked = None;
        if self.buffer().mode == Mode::Normal && matches!(key.as_str(), "p" | "P") {
            let clipboard = cx.read_from_clipboard();
            let text = clipboard.as_ref().and_then(ClipboardItem::text);
            let linewise = match clipboard
                .as_ref()
                .and_then(ClipboardItem::metadata)
                .map(String::as_str)
            {
                Some("rockdown:lines") => true,
                Some("rockdown:characters") => false,
                _ => text.as_ref().is_some_and(|text| text.ends_with('\n')),
            };
            self.buffer_mut().set_clipboard(text, linewise);
        }
        let before = self.buffer().revision();
        self.buffer_mut().page_rows =
            (self.pane_heights[self.pane.index()] / self.config.line_height).max(1.) as usize;
        self.buffer_mut().key(&key);
        if let Some(motion) = self.buffer_mut().take_viewport_motion() {
            let index = self.pane.index();
            match motion {
                crate::vim::ViewportMotion::Center => {
                    self.viewport_alignment = Some((self.pane, true))
                }
                crate::vim::ViewportMotion::Top => {
                    self.viewport_alignment = Some((self.pane, false))
                }
                crate::vim::ViewportMotion::HalfPage { down, rows } => {
                    self.tops[index] = if down {
                        self.tops[index]
                            .saturating_add(rows)
                            .min(self.buffer().lines.len() - 1)
                    } else {
                        self.tops[index].saturating_sub(rows)
                    };
                }
            }
            self.follow_cursor = matches!(motion, crate::vim::ViewportMotion::HalfPage { .. });
        }
        if self.pane == Pane::Editor && self.documents.current().buffer.revision() != before {
            self.refresh_projection();
        }
        Ok(true)
    }
    fn type_text(&mut self, text: &str) {
        if let Some(command) = &mut self.command {
            command.push_str(&text.replace(['\r', '\n'], ""));
        } else if self.pane == Pane::Terminal {
            if let Some(terminal) = &mut self.terminal {
                let result = if terminal.screen().bracketed_paste() {
                    terminal.send(format!("\x1b[200~{text}\x1b[201~").as_bytes())
                } else {
                    terminal.send(text.as_bytes())
                };
                if let Err(error) = result {
                    self.message = error.to_string();
                }
            }
        } else {
            self.buffer_mut().insert_text(text);
            if self.pane == Pane::Editor {
                self.refresh_projection();
            }
        }
        self.follow_cursor = true;
    }
    pub fn mouse_down(
        &mut self,
        pane: Pane,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hit = self.layouts[pane.index()].rows.iter().find(|row| {
            event.position.y >= row.origin.y && event.position.y < row.origin.y + row.height
        });
        let location = hit.map(|row| {
            (
                row.source_row,
                if row.raw {
                    row.line
                        .closest_index_for_x(event.position.x - row.origin.x)
                } else {
                    0
                },
            )
        });
        self.set_pane(pane, cx);
        self.focus.focus(window);
        if pane != Pane::Terminal
            && let Some((row, col)) = location
        {
            self.buffer_mut().key("escape");
            let buffer = self.buffer_mut();
            buffer.row = row.min(buffer.lines.len() - 1);
            let text = &buffer.lines[buffer.row].text;
            buffer.col = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .take_while(|i| *i <= col)
                .last()
                .unwrap_or(0);
        }
        cx.notify();
    }
    pub fn scroll(&mut self, pane: Pane, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        self.viewport_alignment = None;
        if pane != Pane::Terminal {
            let delta: f32 = event
                .delta
                .pixel_delta(px(self.config.line_height))
                .y
                .into();
            let count = if pane == Pane::Editor {
                self.documents.current().buffer.lines.len()
            } else {
                self.explorer.buffer.lines.len()
            };
            let index = pane.index();
            self.row_heights[index].resize(count, self.config.line_height);
            let top = &mut self.tops[index];
            *top = (*top).min(count - 1);
            let offset = &mut self.scroll_offsets[index];
            *offset -= delta;
            while *offset >= self.row_heights[index][*top] && *top + 1 < count {
                *offset -= self.row_heights[index][*top];
                *top += 1;
            }
            while *offset < 0. && *top > 0 {
                *top -= 1;
                *offset += self.row_heights[index][*top];
            }
            *offset = offset.clamp(0., (self.row_heights[index][*top] - 1.).max(0.));
            self.follow_cursor = false;
            cx.notify();
        }
    }
    pub fn ensure_cursor_visible(&mut self, pane: Pane, height: f32) {
        if pane == Pane::Terminal {
            return;
        }
        self.pane_heights[pane.index()] = height;
        let rows = (height / self.config.line_height).max(1.) as usize;
        let count = if pane == Pane::Editor {
            self.documents.current().buffer.lines.len()
        } else {
            self.explorer.buffer.lines.len()
        };
        self.row_heights[pane.index()].resize(count, self.config.line_height);
        self.tops[pane.index()] = self.tops[pane.index()].min(count - 1);
        if let Some((target, center)) = self.viewport_alignment
            && target == pane
        {
            let index = pane.index();
            let mut top = self.buffer().row;
            let mut space = if center {
                (height - self.config.line_height).max(0.) / 2.
            } else {
                0.
            };
            while top > 0 && space > 0. {
                top -= 1;
                space -= self.row_heights[index][top];
            }
            self.tops[index] = top;
            self.scroll_offsets[index] = (-space).max(0.);
            return;
        }
        if self.follow_cursor && pane == self.pane && pane != Pane::Terminal {
            let row = self.buffer().row;
            self.scroll_offsets[pane.index()] = 0.;
            let top = &mut self.tops[pane.index()];
            if row < *top {
                *top = row;
            } else if row >= *top + rows {
                *top = row.saturating_sub(rows.saturating_sub(1));
            }
        }
    }

    pub fn viewport_is_aligned(&self, pane: Pane) -> bool {
        self.viewport_alignment
            .is_some_and(|(target, _)| target == pane)
    }
    fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.documents.active_id();
        let result = self.change_document(
            |documents| {
                documents.select(id)?;
                documents.delete(false)?;
                if id != active {
                    documents.select(active)?;
                }
                Ok(())
            },
            cx,
        );
        if let Err(error) = result {
            self.message = error.to_string();
        }
        self.focus.focus(window);
        cx.notify();
    }

    fn buffer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.documents.active_id();
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let panel = self.color(&self.config.theme.panel);
        let background = self.color(&self.config.theme.background);
        div()
            .id("buffer-tabs")
            .h(px(38.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .overflow_x_scroll()
            .bg(panel)
            .text_xs()
            .children(self.documents.entries().iter().map(|entry| {
                let id = entry.id;
                let name = entry
                    .document
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into());
                let label = format!(
                    "{}{}",
                    name,
                    if entry.document.buffer.dirty() {
                        " •"
                    } else {
                        ""
                    }
                );
                let path = entry
                    .document
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Untitled".into());
                div()
                    .id(("buffer-tab", id))
                    .tooltip(move |_, cx| {
                        cx.new(|_| ControlTooltip {
                            label: path.clone().into(),
                            background,
                            foreground: muted,
                            border: accent.opacity(0.3),
                        })
                        .into()
                    })
                    .h_full()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .bg(if id == active { background } else { panel })
                    .hover(|style| style.bg(accent.opacity(0.12)).text_color(accent))
                    .active(|style| style.bg(accent.opacity(0.22)))
                    .text_color(if id == active { accent } else { muted })
                    .border_b_2()
                    .border_color(if id == active { accent } else { panel })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Err(error) =
                            this.change_document(|documents| documents.select(id), cx)
                        {
                            this.message = error.to_string();
                        }
                        this.focus.focus(window);
                        cx.notify();
                    }))
                    .child(label)
                    .child(
                        div()
                            .id(("close-buffer", id))
                            .w(px(22.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(17.))
                            .text_color(muted)
                            .hover(|style| style.bg(accent.opacity(0.18)).text_color(accent))
                            .active(|style| style.bg(accent.opacity(0.3)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, window, cx);
                            }))
                            .child("×"),
                    )
            }))
    }

    fn dock_button(
        &self,
        label: &'static str,
        action: &'static str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        let background = self.color(&self.config.theme.background);
        let foreground = self.color(&self.config.theme.foreground);
        div()
            .id(action)
            .w(px(30.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .flex_shrink_0()
            .cursor_pointer()
            .bg(if selected {
                accent.opacity(0.1)
            } else {
                transparent_black()
            })
            .text_color(self.color(if selected {
                &self.config.theme.accent
            } else {
                &self.config.theme.muted
            }))
            .hover(|style| style.bg(accent.opacity(0.2)).text_color(accent))
            .active(|style| style.bg(accent.opacity(0.3)))
            .tooltip(move |_, cx| {
                cx.new(|_| ControlTooltip {
                    label: label.into(),
                    background,
                    foreground,
                    border: accent.opacity(0.3),
                })
                .into()
            })
            .on_click(cx.listener(move |this, _, window, cx| this.run_action(action, window, cx)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let p = |x, y| bounds.origin + point(px(x), px(y));
                        let mut path = PathBuilder::stroke(px(1.5));
                        match action {
                            "terminal" => {
                                path.move_to(p(2., 3.));
                                path.line_to(p(18., 3.));
                                path.line_to(p(18., 17.));
                                path.line_to(p(2., 17.));
                                path.close();
                                path.move_to(p(5., 7.));
                                path.line_to(p(8., 10.));
                                path.line_to(p(5., 13.));
                                path.move_to(p(10., 13.));
                                path.line_to(p(15., 13.));
                            }
                            "explorer" => {
                                path.move_to(p(2., 5.));
                                path.line_to(p(8., 5.));
                                path.line_to(p(10., 7.));
                                path.line_to(p(18., 7.));
                                path.line_to(p(18., 16.));
                                path.line_to(p(2., 16.));
                                path.close();
                            }
                            "help" => {
                                path.move_to(p(10., 2.));
                                path.cubic_bezier_to(p(18., 10.), p(14.4, 2.), p(18., 5.6));
                                path.cubic_bezier_to(p(10., 18.), p(18., 14.4), p(14.4, 18.));
                                path.cubic_bezier_to(p(2., 10.), p(5.6, 18.), p(2., 14.4));
                                path.cubic_bezier_to(p(10., 2.), p(2., 5.6), p(5.6, 2.));
                                path.close();
                                path.move_to(p(7.5, 7.));
                                path.cubic_bezier_to(p(12.5, 7.), p(7.5, 4.5), p(12.5, 4.5));
                                path.cubic_bezier_to(p(10., 11.5), p(12.5, 9.5), p(10., 9.));
                                path.move_to(p(10., 14.));
                                path.line_to(p(10., 15.));
                            }
                            _ => unreachable!("unknown dock control"),
                        }
                        window.paint_path(
                            path.build().expect("valid control icon"),
                            window.text_style().color,
                        );
                    },
                )
                .size(px(20.)),
            )
    }
    fn surface(&self, pane: Pane, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(match pane {
                Pane::Editor => "editor",
                Pane::Explorer => "explorer",
                Pane::Terminal => "terminal",
            })
            .size_full()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, window, cx| {
                    this.mouse_down(pane, event, window, cx)
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, event, _, cx| this.scroll(pane, event, cx)))
            .child(Surface {
                workspace: cx.entity(),
                pane,
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = self.color(&self.config.theme.background);
        let panel = self.color(&self.config.theme.panel);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let mode = if self.pane == Pane::Terminal {
            "TERMINAL"
        } else {
            match self.buffer().mode {
                Mode::Normal => "NORMAL",
                Mode::Insert => "INSERT",
                Mode::Visual => "VISUAL",
            }
        };
        let location = format!(
            "{}:{}",
            self.buffer().row + 1,
            self.buffer().lines[self.buffer().row].text[..self.buffer().col]
                .chars()
                .count()
                + 1
        );
        let status = self.command.clone().unwrap_or_else(|| self.message.clone());
        let explorer_width = self.explorer_width(window);
        let explorer_label = if self.explorer_visible {
            "Hide Files"
        } else {
            "Show Files"
        };
        let buffer_name = self
            .documents
            .current()
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let window_title = format!("Rockdown — {buffer_name}");
        window.set_window_title(&window_title);
        div().size_full().flex().flex_col().bg(background).text_color(foreground).font_family(self.config.font_family.clone()).text_size(px(self.config.font_size))
            .track_focus(&self.focus).on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &Save, window, cx| this.run_action("save", window, cx)))
            .on_action(cx.listener(|this, _: &Paste, window, cx| this.run_action("paste", window, cx)))
            .on_action(cx.listener(|this, _: &ExplorerToggle, window, cx| this.run_action("explorer", window, cx)))
            .on_action(cx.listener(|this, _: &TerminalToggle, window, cx| this.run_action("terminal", window, cx)))
            .on_action(cx.listener(|this, _: &EditorPane, window, cx| this.run_action("editor", window, cx)))
            .on_action(cx.listener(|this, _: &HelpToggle, window, cx| this.run_action("help", window, cx)))
            .on_action(cx.listener(|this, _: &PreviousBuffer, window, cx| this.run_action("previous-buffer", window, cx)))
            .on_action(cx.listener(|this, _: &NextBuffer, window, cx| this.run_action("next-buffer", window, cx)))
            .on_action(cx.listener(|this, _: &BufferDelete, window, cx| this.run_action("buffer-delete", window, cx)))
            .on_mouse_move(cx.listener(Self::resize_explorer))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)))
            .when(self.explorer_resize.is_some(), |root| root.cursor(CursorStyle::ResizeLeftRight))
            .when(cfg!(target_os = "macos"), |root| root.child(
                div().id("window-title").h(px(32.)).flex_shrink_0()
                    .px(px(80.)).flex().items_center().justify_center()
                    .bg(panel).text_size(px(12.)).text_color(muted)
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })
                    .child(div().overflow_hidden().text_ellipsis().child(window_title))
            ))
            .child(div().flex_1().min_h_0().flex()
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(self.buffer_bar(cx))
                    .child(div().flex_1().min_h_0().flex().justify_center()
                        .child(div().w_full().min_w_0().h_full()
                            .when(self.documents.current().is_markdown(), |column| column.max_w(px(self.config.writing_width)).py_4())
                            .child(self.surface(Pane::Editor,cx)))))
                .when(self.explorer_visible, |body| body.child(div().w(px(explorer_width)).flex_shrink_0().flex().bg(panel)
                    .child(div().id("explorer-splitter").w(px(6.)).flex_shrink_0().h_full().cursor(CursorStyle::ResizeLeftRight)
                        .bg(if self.pane==Pane::Explorer {accent} else {background})
                        .hover(|style| style.bg(accent))
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::start_explorer_resize)))
                    .child(div().flex_1().min_w_0().flex().flex_col()
                        .child(div().h(px(38.)).flex_shrink_0().px_3().flex().items_center().text_xs().text_color(muted).child(if self.explorer.dirty() { "Files  •" } else { "Files" }))
                        .child(div().px_3().pb_2().text_xs().text_color(muted).overflow_hidden().child(self.explorer.directory.display().to_string()))
                        .child(div().flex_1().min_h_0().child(self.surface(Pane::Explorer,cx)))))))
            .when(self.terminal_visible, |root| root.child(div().h(px(self.config.terminal_height)).flex_shrink_0().flex().flex_col().border_t_1().border_color(accent).bg(background)
                .child(div().h(px(28.)).flex_shrink_0().px_4().text_sm().text_color(muted).child("TERMINAL · Ctrl-` hide · Ctrl-W h/l change focus"))
                .child(div().flex_1().min_h_0().child(self.surface(Pane::Terminal,cx)))))
            .when(self.help, |root| root.child(div().id("help-sheet").absolute().inset_0().m_8().p_6().bg(panel).border_1().border_color(accent).rounded_lg().overflow_y_scroll().flex().flex_col().gap_2()
                .child(div().text_xl().text_color(accent).child("Rockdown · keyboard guide"))
                .child(div().flex_shrink_0().text_sm().child("Editor and Files share the system clipboard: y copies, p/P paste. Deletes and changes also copy their removed text."))
                .child(div().flex_shrink_0().text_sm().child("Ctrl-D / Ctrl-U: half-page down / up. zz: center current line. zt: current line at top."))
                .child(div().flex_shrink_0().text_sm().child(r":%s/pattern/replacement/[giI]: whole-buffer substitution. Rust regex; & = match, \1 = capture. u undoes all replacements."))
                .children(HELP.lines().map(|line| div().flex_shrink_0().text_sm().child(line.to_string())))
                .child(div().flex_shrink_0().text_sm().child("Prose wraps. Local images render inline; remote images stay linked alt text (no network requests)."))))
            .child(div().h(px(34.)).flex_shrink_0().px_2().flex().items_center().gap_3().bg(panel).text_xs()
                .child(div().flex_shrink_0().text_color(accent).font_weight(FontWeight::BOLD).child(mode))
                .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(status))
                .child(div().flex_shrink_0().text_color(muted).child(location))
                .when(self.terminal_visible, |footer| footer.child(self.dock_button("Hide Terminal", "terminal", true, cx)))
                .child(self.dock_button(explorer_label, "explorer", self.explorer_visible, cx))
                .child(self.dock_button(if self.help { "Hide Help" } else { "Show Help" }, "help", self.help, cx)))
    }
}

struct ControlTooltip {
    label: SharedString,
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
}

impl Render for ControlTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .text_color(self.foreground)
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

const HELP: &str = r#"Editing: i/a/I/A insert · o/O new line · Esc normal · v visual
Return splits at the caret in Insert mode; in Normal mode it opens a line below.
h/j/k/l or arrows · w/b/e words · 0/$ line · gg/G document
x delete · dd/dw/d$ delete · cc/cw change · yy yank · p/P paste
u undo · Ctrl-R redo · counts: 3j, 2dd · /find then n repeat
Clipboard: Ctrl-C/V in Editor and Files (Cmd-C/V on macOS).
Ctrl-Shift-V pastes in every pane; terminal paste respects bracketed-paste mode.
Images render at 60% of the pane width; ![alt](pic.png "40%") or "320px" resizes.
:w [filename] save · :w! overwrite conflict · :e[!] [file] reload/open
:q close window · :q! discard all and close · :wq save and close

Buffers: click a tab to select; click its × to close. Unsaved changes are protected.
:bp / :bprevious / :previous-buffer · Ctrl-PageUp
:bn / :bnext / :next-buffer · Ctrl-PageDown
:bd / :bdelete / :buffer-delete · Cmd-W or Ctrl-Shift-W
:bd! discards unsaved changes. Deleting a buffer does not delete its file.
:e filename opens or activates a buffer without discarding other edits.

Explorer: Ctrl-E / Cmd-E hide/show · Ctrl-W l or :ex reveal and focus
Drag the left edge to resize. Hiding preserves staged changes and width.
Enter open · - parent · Edit filenames with Vim.
o creates a line; trailing / creates a directory.
dd stages deletion. :w commits; :e! discards. Deletes go to .rockdown-trash.

Terminal: Ctrl-` toggle · Ctrl-W h editor / l files / j terminal
Return executes the command. Ctrl-C interrupts. Type exit to close the shell.
Ctrl-D sends EOF in Unix shells. Ctrl-Shift-V (or Cmd-V on macOS) pastes.

Configuration: Windows %APPDATA%\rockdown\config.toml; Unix ~/.config/rockdown/config.toml
XDG_CONFIG_HOME overrides the base directory on either platform.
--config PATH selects an explicit file. :config reloads settings.
markdown.colors: normal/bold/italic/bold_italic/code/link/strikethrough/quote.
markdown.h1 through h6: font_size, color, underline.
markdown.divider: color, thickness (also used by heading underlines).

Preview applies to .md filenames (case-insensitive) and untitled buffers only.
Other filenames show literal text. In Markdown, inactive lines render;
the cursor line exposes editable syntax. Save-as updates the preview type.
Click a line to edit. Mouse wheel scrolls. Ctrl-S saves. Esc closes help."#;

fn utf8_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (offset, c) in text.char_indices() {
        if units + c.len_utf16() > utf16 {
            return offset;
        }
        units += c.len_utf16();
    }
    text.len()
}
fn utf16_offset(text: &str, utf8: usize) -> usize {
    text[..utf8.min(text.len())].encode_utf16().count()
}

impl Workspace {
    fn input_text(&self) -> &str {
        if self.pane == Pane::Terminal {
            ""
        } else if let Some(command) = &self.command {
            command
        } else {
            &self.buffer().lines[self.buffer().row].text
        }
    }
    fn input_col(&self) -> usize {
        if self.pane == Pane::Terminal {
            0
        } else {
            self.command.as_ref().map_or(self.buffer().col, String::len)
        }
    }
    fn replace_input(&mut self, range: Option<Range<usize>>, text: &str) {
        if self.pane == Pane::Terminal {
            if let Some(terminal) = &mut self.terminal
                && let Err(error) = terminal.send(text.as_bytes())
            {
                self.message = error.to_string();
            }
            return;
        }
        let range = range.or_else(|| self.marked.clone());
        if let Some(command) = &mut self.command {
            let range = range
                .map(|r| utf8_offset(command, r.start)..utf8_offset(command, r.end))
                .unwrap_or(command.len()..command.len());
            command.replace_range(range, text);
        } else if self.buffer().mode == Mode::Insert {
            if let Some(range) = range {
                let current = self.input_text();
                let start = utf8_offset(current, range.start);
                let end = utf8_offset(current, range.end);
                // IME ranges are UTF-16; extend to full graphemes before editing.
                let start = current
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(current.len()))
                    .take_while(|i| *i <= start)
                    .last()
                    .unwrap_or(0);
                let end = current
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .find(|i| *i >= end)
                    .unwrap_or(current.len());
                let count = current[start..end].graphemes(true).count();
                self.buffer_mut().col = start;
                for _ in 0..count {
                    self.buffer_mut().key("delete");
                }
            }
            self.type_text(text);
        }
        self.marked = None;
    }
}
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.input_text();
        let start = utf8_offset(text, range.start);
        let end = utf8_offset(text, range.end).max(start);
        *actual = Some(utf16_offset(text, start)..utf16_offset(text, end));
        Some(text[start..end].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.help
            || (self.command.is_none()
                && self.pane != Pane::Terminal
                && self.buffer().mode != Mode::Insert)
        {
            return None;
        }
        let col = utf16_offset(self.input_text(), self.input_col());
        Some(UTF16Selection {
            range: col..col,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone()
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_input(range, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range.as_ref().or(self.marked.as_ref()).map_or_else(
            || utf16_offset(self.input_text(), self.input_col()),
            |r| r.start,
        );
        self.replace_input(range, text);
        self.marked = (!text.is_empty()).then_some(start..start + text.encode_utf16().count());
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        let start = utf8_offset(self.input_text(), range.start);
        Some(
            Bounds::new(
                point(row.origin.x + row.line.x_for_index(start), row.origin.y),
                size(px(2.), px(self.config.line_height)),
            )
            .intersect(&bounds),
        )
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        Some(utf16_offset(
            self.input_text(),
            row.line.closest_index_for_x(position.x - row.origin.x),
        ))
    }
}
