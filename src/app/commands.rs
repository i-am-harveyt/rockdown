use super::{Pane, UiMode, Workspace, bind_config_keys};
use crate::{config::Config, explorer::Explorer, markdown::BlockKind, vim::InlineFormat};
use anyhow::{Result, bail};
use gpui::*;
use std::path::Path;
use unicode_segmentation::UnicodeSegmentation;

impl Workspace {
    /// Dispatch a named action from a keymap binding, surfacing failures in
    /// the status bar instead of dropping them.
    pub(super) fn run_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.action(action, window, cx) {
            self.set_message(format!("{error:#}"));
            cx.notify();
        }
    }
    fn action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if self.ui_mode == UiMode::Writer && action == "status-bar" {
            self.toggle_writer_chrome(window, cx);
            return Ok(());
        }
        if action == "ui-mode" {
            self.toggle_ui_mode_picker(window, cx);
            return Ok(());
        }
        if self.ui_mode_picker.is_some() {
            return Ok(());
        }
        if self.ui_mode == UiMode::Writer && action == "tab-bar" {
            self.toggle_writer_tabs(window, cx);
            return Ok(());
        }
        if self.writer_tabs_open
            && !matches!(
                action,
                "explorer"
                    | "terminal"
                    | "help"
                    | "outline"
                    | "themes"
                    | "editor"
                    | "previous-buffer"
                    | "next-buffer"
                    | "buffer-delete"
            )
        {
            return Ok(());
        }
        if self.ui_mode == UiMode::Writer
            && matches!(action, "help" | "outline" | "explorer" | "terminal")
        {
            self.writer_tabs_open = false;
            self.finish_theme_picker(false, cx);
            if action != "help" {
                self.help = false;
            }
            if action != "outline" {
                self.dismiss_outline();
            }
            if (self.pane == Pane::Explorer && action != "explorer")
                || (self.pane == Pane::Terminal && action != "terminal")
            {
                self.set_pane(Pane::Editor, cx);
            }
        }
        if self.help
            && !matches!(
                action,
                "help" | "themes" | "outline" | "tab-bar" | "status-bar"
            )
        {
            return Ok(());
        }
        if self.theme_picker.is_some() {
            if action == "themes" {
                self.finish_theme_picker(false, cx);
            }
            return Ok(());
        }
        if self.outline_picker.is_some()
            && !matches!(
                action,
                "outline"
                    | "paste"
                    | "help"
                    | "themes"
                    | "previous-buffer"
                    | "next-buffer"
                    | "save"
                    | "save-as"
            )
        {
            return Ok(());
        }
        match action {
            "new" => self.change_document(
                |documents| {
                    documents.new_document();
                    Ok(())
                },
                cx,
            )?,
            "open" => self.open_dialog(window, cx),
            "save-as" => self.save_dialog(self.documents.active_id(), true, None, window, cx),
            "save" => {
                if self.pane == Pane::Explorer {
                    self.save(None, false)?;
                } else {
                    self.save_dialog(self.documents.active_id(), false, None, window, cx);
                }
            }
            "explorer" => self.toggle_explorer(cx),
            "terminal" => self.toggle_terminal(cx)?,
            "editor" => {
                self.writer_tabs_open = false;
                self.set_pane(Pane::Editor, cx);
            }
            "help" => {
                self.dismiss_outline();
                self.help = !self.help;
            }
            "themes" => self.open_theme_picker(cx),
            "outline" => self.toggle_outline(cx),
            "tab-bar" => self.config.tab_bar_visible = !self.config.tab_bar_visible,
            "status-bar" => {
                self.config.status_bar_visible = !self.config.status_bar_visible;
                self.set_message(String::new());
            }
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
            "buffer-delete" => self.close_tab(self.documents.active_id(), window, cx),
            "paste" => self.paste_clipboard(window, cx),
            "undo" | "redo" => {
                if !self.buffer_history_available() {
                    return Ok(());
                }
                let buffer = self.buffer_mut();
                if action == "undo" {
                    buffer.undo();
                } else {
                    buffer.redo();
                }
                self.preferred_visual_x = None;
                self.viewport_alignment = None;
                self.follow_cursor = true;
                if self.pane == Pane::Editor {
                    self.refresh_projection();
                }
            }
            "bold" => self.format_selection(InlineFormat::Bold),
            "italic" => self.format_selection(InlineFormat::Italic),
            "underline" => self.format_selection(InlineFormat::Underline),
            "strikethrough" => self.format_selection(InlineFormat::Strikethrough),
            "inline-code" => self.format_selection(InlineFormat::Code),
            _ => {}
        }
        self.focus.focus(window);
        cx.notify();
        Ok(())
    }

    pub(super) fn buffer_history_available(&self) -> bool {
        self.pane != Pane::Terminal
            && self.command.is_none()
            && self.marked.is_none()
            && !self.dialog_pending
            && !self.help
            && self.theme_picker.is_none()
            && self.outline_picker.is_none()
            && self.ui_mode_picker.is_none()
            && !self.writer_tabs_open
    }

    fn format_selection(&mut self, format: InlineFormat) {
        if self.pane != Pane::Editor
            || !self.documents.current().is_markdown()
            || self.command.is_some()
            || self.marked.is_some()
            || self.dialog_pending
        {
            return;
        }
        let buffer = self.buffer();
        if self.projection.iter().any(|line| {
            line.kind == BlockKind::Code && buffer.selected_range(line.source_row).is_some()
        }) {
            return;
        }
        if self.buffer_mut().toggle_inline_format(format) {
            self.preferred_visual_x = None;
            self.viewport_alignment = None;
            self.follow_cursor = true;
            self.refresh_projection();
        }
    }
    pub(super) fn find_next(&mut self, first: bool) {
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
        self.set_message(format!("Pattern not found: {query}"));
    }
    pub(super) fn execute(
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
            self.set_message(format!(
                "{count} substitution{}",
                if count == 1 { "" } else { "s" }
            ));
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
                self.request_window_close(window, cx);
            }
            "q" => {
                self.request_window_close(window, cx);
            }
            "q!" => {
                if self.finish_recovery(cx) {
                    window.remove_window();
                }
            }
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
            "help" => {
                self.dismiss_outline();
                if self.ui_mode == UiMode::Writer {
                    self.set_pane(Pane::Editor, cx);
                }
                self.help = true;
            }
            "theme" | "themes" => self.open_theme_picker(cx),
            "outline" => self.toggle_outline(cx),
            "uimode" => self.toggle_ui_mode_picker(window, cx),
            "tabbar" if self.ui_mode == UiMode::Writer => {
                self.toggle_writer_tabs(window, cx);
            }
            "tabbar" => self.config.tab_bar_visible = !self.config.tab_bar_visible,
            "statusbar" if self.ui_mode == UiMode::Writer => self.toggle_writer_chrome(window, cx),
            "statusbar" => {
                self.config.status_bar_visible = !self.config.status_bar_visible;
                self.set_message(String::new());
            }
            "config" => {
                let (config, path) = Config::load(self.config_path.as_deref())?;
                bind_config_keys(&config, cx);
                self.config = config;
                self.config_path = path;
                self.refresh_projection();
                for heights in &mut self.row_heights {
                    heights.clear();
                }
                self.set_message("Configuration reloaded; new shell setting applies to the next terminal session".into());
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
}
