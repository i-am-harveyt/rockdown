use super::{Pane, UiMode, Workspace, help::HELP};
use crate::{markdown, terminal::key_bytes, vim::Mode};
use anyhow::Result;
use gpui::*;
use unicode_segmentation::UnicodeSegmentation;

impl Workspace {
    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.theme_picker.is_none()
            && self.outline_picker.is_none()
            && self.ui_mode_picker.is_none()
            && !self.writer_tabs_open
        {
            self.follow_cursor = true;
            self.viewport_alignment = None;
        }
        let result = self.handle_key(event, window, cx);
        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => self.set_message(format!("{error:#}")),
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
            self.request_window_close(window, cx);
            return Ok(true);
        }
        if let Some(selected) = self.ui_mode_picker {
            match key {
                "escape" => self.ui_mode_picker = None,
                "enter" => self.set_ui_mode(selected, window, cx),
                "up" | "down" | "j" | "k" | "tab" => {
                    self.ui_mode_picker = Some(match selected {
                        UiMode::Dev => UiMode::Writer,
                        UiMode::Writer => UiMode::Dev,
                    });
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.writer_tabs_open {
            if key == "escape" {
                self.writer_tabs_open = false;
            }
            return Ok(true);
        }
        let clipboard_shortcut =
            crate::keyboard::clipboard_shortcut(stroke, self.pane == Pane::Terminal);
        if clipboard_shortcut && stroke.key == "v" && !self.help && self.theme_picker.is_none() {
            self.paste_clipboard(window, cx);
            return Ok(true);
        }
        if self.outline_picker.is_some() {
            if self.marked.is_some() {
                return Ok(false);
            }
            match key {
                "escape" => self.dismiss_outline(),
                "enter" => {
                    let picker = self.outline_picker.as_ref().unwrap();
                    if let Some(&index) = picker.matches.get(picker.selected) {
                        self.jump_to_heading(self.headings[index].row, window, cx);
                    }
                }
                "up" | "down" => {
                    let picker = self.outline_picker.as_mut().unwrap();
                    let count = picker.matches.len();
                    if count > 0 {
                        picker.selected = if key == "up" {
                            (picker.selected + count - 1) % count
                        } else {
                            (picker.selected + 1) % count
                        };
                        picker.scroll.scroll_to_item(picker.selected);
                    }
                }
                "left" | "right" | "home" | "end" | "backspace" | "delete" => {
                    let picker = self.outline_picker.as_mut().unwrap();
                    let previous = picker.query[..picker.col]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(index, _)| index);
                    let next = picker.query[picker.col..]
                        .graphemes(true)
                        .next()
                        .map_or(picker.col, |text| picker.col + text.len());
                    match key {
                        "left" => picker.col = previous,
                        "right" => picker.col = next,
                        "home" => picker.col = 0,
                        "end" => picker.col = picker.query.len(),
                        "backspace" => {
                            picker.query.replace_range(previous..picker.col, "");
                            picker.col = previous;
                        }
                        "delete" => {
                            picker.query.replace_range(picker.col..next, "");
                        }
                        _ => unreachable!(),
                    }
                    if matches!(key, "backspace" | "delete") {
                        self.filter_outline();
                    }
                }
                _ => return Ok(stroke.modifiers.control || stroke.modifiers.platform),
            }
            return Ok(true);
        }
        if let Some(picker) = &self.theme_picker {
            let selected = picker.selected;
            let count = picker.presets.len() + 1;
            match key {
                "escape" => self.finish_theme_picker(false, cx),
                "enter" => self.finish_theme_picker(true, cx),
                "up" | "k" => self.preview_theme((selected + count - 1) % count, cx),
                "down" | "j" | "tab" => self.preview_theme((selected + 1) % count, cx),
                "home" => self.preview_theme(0, cx),
                "end" => self.preview_theme(count - 1, cx),
                _ => {}
            }
            if let Some(picker) = &self.theme_picker {
                picker.scroll.scroll_to_item(picker.selected);
            }
            return Ok(true);
        }
        if self.help {
            match key {
                "escape" | "q" => self.help = false,
                "left" => self.help_section = (self.help_section + HELP.len() - 1) % HELP.len(),
                "right" | "tab" => self.help_section = (self.help_section + 1) % HELP.len(),
                _ => {}
            }
            return Ok(true);
        }
        if self.ui_mode == UiMode::Writer && key == "escape" && self.command.is_none()
            && self.pane == Pane::Explorer && self.buffer().mode == Mode::Normal {
                self.set_pane(Pane::Editor, cx);
                return Ok(true);
            }
        if clipboard_shortcut && stroke.key == "c" && self.pane != Pane::Terminal {
            let buffer = self.buffer();
            let mut selected = buffer
                .lines
                .iter()
                .enumerate()
                .filter_map(|(row, line)| {
                    buffer.selected_range(row).map(|r| line.text[r].to_string())
                })
                .collect::<Vec<_>>()
                .join("\n");
            let linewise = buffer.mode == Mode::VisualLine;
            if linewise {
                selected.push('\n');
            }
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                    selected,
                    if linewise {
                        "rockdown:lines"
                    } else {
                        "rockdown:characters"
                    }
                    .to_string(),
                ));
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
            self.preferred_visual_x = None;
            let continue_markdown = self.documents.current().is_markdown()
                && self.projection.get(self.buffer().row).is_some_and(|line| {
                    matches!(
                        line.kind,
                        markdown::BlockKind::List | markdown::BlockKind::Quote
                    )
                })
                && !stroke.modifiers.shift;
            let buffer = self.buffer_mut();
            match buffer.mode {
                Mode::Normal => {
                    buffer.key("o");
                }
                Mode::Insert => {
                    if continue_markdown {
                        buffer.markdown_enter();
                    } else {
                        buffer.key("enter");
                    }
                }
                Mode::Visual | Mode::VisualLine => {
                    buffer.key("c");
                    buffer.key("enter");
                }
            }
            self.marked = None;
            self.refresh_projection();
            return Ok(true);
        }
        let insert = self.buffer().mode == Mode::Insert;
        if insert
            && self.pane == Pane::Editor
            && self.documents.current().is_markdown()
            && !stroke.modifiers.control
            && !stroke.modifiers.alt
            && !stroke.modifiers.platform
            && matches!(key, "up" | "down")
            && let Some(row) = self.editable_navigation_row(self.buffer().row, window)
        {
            let position = row.position_for_index(self.buffer().col);
            let x = self.preferred_visual_x.unwrap_or(position.x);
            let y = position.y + row.line_height * if key == "up" { -0.5 } else { 1.5 };
            let destination = if y >= px(0.) && y < row.height {
                Some((row.source_row, row.index_for_position(point(x, y))))
            } else {
                let next = if key == "up" {
                    row.source_row.checked_sub(1)
                } else {
                    (row.source_row + 1 < self.buffer().lines.len()).then_some(row.source_row + 1)
                };
                next.and_then(|next| self.editable_navigation_row(next, window))
                    .map(|next| {
                        let y = if key == "up" {
                            next.height - next.line_height * 0.5
                        } else {
                            next.line_height * 0.5
                        };
                        (next.source_row, next.index_for_position(point(x, y)))
                    })
            };
            if let Some((row, col)) = destination {
                let buffer = self.buffer_mut();
                buffer.row = row;
                buffer.col = buffer.lines[row]
                    .text
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain(std::iter::once(buffer.lines[row].text.len()))
                    .take_while(|i| *i <= col)
                    .last()
                    .unwrap_or(0);
            }
            self.preferred_visual_x = Some(x);
            self.marked = None;
            return Ok(true);
        }
        self.preferred_visual_x = None;
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
        if self.buffer().mode != Mode::Insert && matches!(key.as_str(), "p" | "P") {
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
}
