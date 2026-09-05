use crate::surface::{Surface, SurfaceLayout};
use anyhow::{Result, bail};
use gpui::{prelude::*, *};
use rockdown::{
    config::{Config, parse_color},
    document::Document,
    explorer::Explorer,
    markdown::{self, RenderedLine},
    terminal::{Terminal, key_bytes},
    vim::{Buffer, Mode},
};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
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

pub(crate) struct Workspace {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub documents: rockdown::documents::Documents,
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
        let projection = markdown::project(&document.buffer.text());
        Self {
            config,
            config_path,
            documents: rockdown::documents::Documents::new(document),
            explorer,
            explorer_visible: true,
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
            message:
                "F1 help  ·  :w filename.md to save  ·  Ctrl-E toggle files  ·  Ctrl-` terminal"
                    .into(),
            help: false,
            marked: None,
            search: String::new(),
            window_prefix: false,
            follow_cursor: true,
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
        self.projection = markdown::project(&self.documents.current().buffer.text());
        self.row_heights[Pane::Editor.index()].clear();
    }
    fn change_document(
        &mut self,
        change: impl FnOnce(&mut rockdown::documents::Documents) -> Result<()>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let editor = Pane::Editor.index();
        let previous = self.documents.active_id();
        *self.documents.viewport_mut() = rockdown::documents::Viewport {
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
    pub(crate) fn start_explorer_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_resize = Some((event.position.x, self.explorer_width(window)));
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn resize_explorer(
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
    pub(crate) fn stop_explorer_resize(&mut self, cx: &mut Context<Self>) {
        if self.explorer_resize.take().is_some() {
            cx.notify();
        }
    }
    fn toggle_terminal(&mut self, cx: &mut Context<Self>) -> Result<()> {
        if self.terminal.is_none() || self.terminal.as_ref().is_some_and(Terminal::exited) {
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
                            !terminal.exited()
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
        let command = command.trim_start_matches(':').trim();
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
                self.config = config;
                self.config_path = path;
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
        let result = self.handle_key(event, window, cx);
        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => self.message = format!("{error:#}"),
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
        if let Some(action) = self.config.keys.iter().find_map(|(key, action)| {
            let binding = Keystroke::parse(key).ok()?;
            (binding.key == stroke.key && binding.modifiers == stroke.modifiers)
                .then(|| action.clone())
        }) {
            self.action(&action, window, cx)?;
            return Ok(true);
        }
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
        if stroke.modifiers.platform && stroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.type_text(&text);
            }
            return Ok(true);
        }
        if stroke.modifiers.platform && stroke.key == "c" && self.pane != Pane::Terminal {
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
        let before = self.buffer().revision();
        self.buffer_mut().key(&key);
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
        let layout = &self.layouts[pane.index()];
        let hit = layout.rows.iter().find(|row| {
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
    pub fn ensure_cursor_visible(&mut self, pane: Pane, rows: usize) {
        if pane == Pane::Terminal {
            return;
        }
        let count = if pane == Pane::Editor {
            self.documents.current().buffer.lines.len()
        } else {
            self.explorer.buffer.lines.len()
        };
        self.row_heights[pane.index()].resize(count, self.config.line_height);
        self.tops[pane.index()] = self.tops[pane.index()].min(count - 1);
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
    fn buffer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.documents.active_id();
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let panel = self.color(&self.config.theme.panel);
        div()
            .h(px(34.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(panel)
            .text_xs()
            .child(
                div()
                    .id("buffer-tabs")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .overflow_x_scroll()
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
                            "{}: {}{}",
                            id,
                            name,
                            if entry.document.buffer.dirty() {
                                " [+]"
                            } else {
                                ""
                            }
                        );
                        div()
                            .id(("buffer-tab", id))
                            .px_3()
                            .py_2()
                            .flex_shrink_0()
                            .cursor_pointer()
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
                    })),
            )
            .children(
                [
                    ("Prev", "previous-buffer"),
                    ("Next", "next-buffer"),
                    ("Close", "buffer-delete"),
                ]
                .into_iter()
                .map(|(label, action)| {
                    div()
                        .id(action)
                        .px_2()
                        .py_2()
                        .flex_shrink_0()
                        .cursor_pointer()
                        .text_color(accent)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Err(error) = this.action(action, window, cx) {
                                this.message = error.to_string();
                                cx.notify();
                            }
                        }))
                        .child(label)
                }),
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
        let path = self
            .documents
            .current()
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Untitled — :w filename.md".into());
        let title = format!(
            "{}{}",
            path,
            if self.documents.current().buffer.dirty() {
                "  [+]"
            } else {
                ""
            }
        );
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
        div().size_full().flex().flex_col().bg(background).text_color(foreground).font_family(self.config.font_family.clone()).text_size(px(self.config.font_size))
            .track_focus(&self.focus).on_key_down(cx.listener(Self::key_down))
            .on_mouse_move(cx.listener(Self::resize_explorer))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)))
            .when(self.explorer_resize.is_some(), |root| root.cursor(CursorStyle::ResizeLeftRight))
            .child(div().h(px(46.)).flex_shrink_0().px_4().flex().items_center().justify_between().bg(panel)
                .child(div().flex().gap_4().child(div().text_color(accent).font_weight(FontWeight::BOLD).child("ROCKDOWN")).child(div().text_sm().text_color(muted).child("Markdown workspace")))
                .child(div().flex().gap_4().children([(explorer_label, "explorer"),("Terminal", "terminal"),("Help", "help")].into_iter().map(|(label,action)| div().id(label).cursor_pointer().text_sm().text_color(accent).on_click(cx.listener(move |this,_,window,cx| { if let Err(e)=this.action(action,window,cx) { this.message=e.to_string(); cx.notify(); } })).child(label)))))
            .child(div().flex_1().min_h_0().flex()
                .child(div().flex_1().min_w_0().flex().flex_col()
                    .child(self.buffer_bar(cx))
                    .child(div().h(px(38.)).flex_shrink_0().px_4().flex().items_center().text_sm().text_color(muted).overflow_hidden().child(title))
                    .child(div().flex_1().min_h_0().child(self.surface(Pane::Editor,cx))))
                .when(self.explorer_visible, |body| body.child(div().w(px(explorer_width)).flex_shrink_0().flex().bg(panel)
                    .child(div().id("explorer-splitter").w(px(6.)).flex_shrink_0().h_full().cursor(CursorStyle::ResizeLeftRight)
                        .bg(if self.pane==Pane::Explorer {accent} else {background})
                        .hover(|style| style.bg(accent))
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::start_explorer_resize)))
                    .child(div().flex_1().min_w_0().flex().flex_col()
                        .child(div().h(px(38.)).flex_shrink_0().px_3().flex().items_center().text_sm().text_color(accent).child(if self.explorer.dirty() { "FILES  [+]  :w applies changes" } else { "FILES  ·  Enter open  ·  - parent" }))
                        .child(div().px_3().pb_2().text_xs().text_color(muted).overflow_hidden().child(self.explorer.directory.display().to_string()))
                        .child(div().flex_1().min_h_0().child(self.surface(Pane::Explorer,cx)))))))
            .when(self.terminal_visible, |root| root.child(div().h(px(self.config.terminal_height)).flex_shrink_0().flex().flex_col().border_t_1().border_color(accent).bg(background)
                .child(div().h(px(28.)).flex_shrink_0().px_4().text_sm().text_color(muted).child(if self.terminal.as_ref().is_some_and(Terminal::exited) { "TERMINAL · exited · toggle to restart" } else { "TERMINAL · Ctrl-` hide · Ctrl-W h/l change focus" }))
                .child(div().flex_1().min_h_0().child(self.surface(Pane::Terminal,cx)))))
            .when(self.help, |root| root.child(div().id("help-sheet").absolute().inset_0().m_8().p_6().bg(panel).border_1().border_color(accent).rounded_lg().overflow_y_scroll().flex().flex_col().gap_2()
                .child(div().text_xl().text_color(accent).child("Rockdown · keyboard guide"))
                .children(HELP.lines().map(|line| div().flex_shrink_0().text_sm().child(line.to_string())))
                .child(div().flex_shrink_0().text_sm().child("Prose wraps. Local images render inline; remote images stay linked alt text (no network requests)."))))
            .child(div().h(px(30.)).flex_shrink_0().px_3().flex().items_center().gap_3().bg(panel).text_sm()
                .child(div().text_color(accent).font_weight(FontWeight::BOLD).child(mode))
                .child(div().flex_1().overflow_hidden().child(status))
                .child(div().text_color(muted).child(location)))
    }
}

const HELP: &str = "Editing: i/a/I/A insert · o/O new line · Esc normal · v visual\nReturn splits at the caret in Insert mode; in Normal mode it opens a line below.\nh/j/k/l or arrows · w/b/e words · 0/$ line · gg/G document\nx delete · dd/dw/d$ delete · cc/cw change · yy yank · p/P paste\nu undo · Ctrl-R redo · counts: 3j, 2dd · /find then n repeat\n:w [filename] save · :w! overwrite conflict · :e[!] [file] reload/open\n:q close window · :q! discard all and close · :wq save and close\n\nBuffers: click tabs, or use Prev / Next / Close above the editor.\n:bp / :bprevious / :previous-buffer · Ctrl-PageUp\n:bn / :bnext / :next-buffer · Ctrl-PageDown\n:bd / :bdelete / :buffer-delete · Cmd-W or Ctrl-Shift-W\n:bd! discards unsaved changes. Deleting a buffer does not delete its file.\n:e filename opens or activates a buffer without discarding other edits.\n\nExplorer: Ctrl-E / Cmd-E hide/show · Ctrl-W l or :ex reveal and focus\nDrag the left edge to resize. Hiding preserves staged changes and width.\nEnter open · - parent · Edit filenames with Vim.\no creates a line; trailing / creates a directory.\ndd stages deletion. :w commits; :e! discards. Deletes go to .rockdown-trash.\n\nTerminal: Ctrl-` toggle · Ctrl-W h editor / l files / j terminal\nReturn executes the command. Ctrl-C interrupts, Ctrl-D exits. Cmd-V pastes.\n\nConfiguration: ~/.config/rockdown/config.toml or config.lua\n--config PATH selects an explicit file. :config reloads settings.\nTOML wins if both default files exist. Lua must return a settings table.\n\nInactive lines render Markdown; the cursor line exposes editable syntax.\nClick a line to edit. Mouse wheel scrolls. Cmd-S saves. Esc closes help.";

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
