use super::{Pane, Workspace};
use crate::{
    image_assets::{self, ImageInput},
    vim::Mode,
};
use anyhow::{Result, bail};
use gpui::*;
use std::path::PathBuf;

impl Workspace {
    pub(super) fn accepts_images(&self) -> bool {
        self.pane == Pane::Editor
            && !self.help
            && self.theme_picker.is_none()
            && self.outline_picker.is_none()
            && self.command.is_none()
            && self.documents.current().is_markdown()
    }

    pub(super) fn paste_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let has_images = item
            .entries()
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::Image(_)));
        if has_images && self.accepts_images() {
            let inputs = item
                .into_entries()
                .filter_map(|entry| match entry {
                    ClipboardEntry::Image(image) => Some(ImageInput::Bytes(image.bytes)),
                    ClipboardEntry::String(_) => None,
                })
                .collect();
            self.begin_image_import(inputs, window, cx);
        } else if let Some(text) = item.text() {
            self.type_text(&text);
        } else if has_images {
            self.set_message("Images can only be inserted into a Markdown editor (.md)".into());
            cx.notify();
        }
    }

    /// Common entrypoint for native editor file drops and callers with local paths.
    pub fn import_image_files(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_image_import(
            paths.iter().cloned().map(ImageInput::File).collect(),
            window,
            cx,
        );
    }

    fn begin_image_import(
        &mut self,
        inputs: Vec<ImageInput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if inputs.is_empty() {
            return;
        }
        if !self.accepts_images() {
            self.set_message("Images can only be inserted into a Markdown editor (.md)".into());
            cx.notify();
            return;
        }
        if self.dialog_pending {
            self.set_message("Finish the open dialog before inserting images".into());
            cx.notify();
            return;
        }
        let id = self.documents.active_id();
        let document = self.documents.current();
        if document.path.is_some() {
            if let Err(error) = self.start_image_import(id, inputs, None, cx) {
                self.set_message(format!("Image import failed: {error:#}"));
            }
            cx.notify();
            return;
        }
        let token = document.recovery_token();
        let caret = (document.buffer.row, document.buffer.col);
        let receiver = cx.prompt_for_new_path(&self.explorer.directory, Some("Untitled.md"));
        self.dialog_pending = true;
        self.set_message("Save this document as Markdown to insert images".into());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.dialog_pending = false;
                let result = match result {
                    Ok(Ok(Some(path))) => (|| -> Result<()> {
                        let document = this
                            .documents
                            .get(id)
                            .ok_or_else(|| anyhow::anyhow!("The original document was closed"))?;
                        if document.path.is_some() || document.recovery_token() != token {
                            bail!("The original document changed; paste or drop the images again");
                        }
                        if !path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                        {
                            bail!("Cannot insert images: choose a Markdown (.md) save path");
                        }
                        this.documents.save_id(id, Some(&path))?;
                        this.start_image_import(id, inputs, Some(caret), cx)
                    })(),
                    Ok(Err(error)) => Err(error),
                    _ => {
                        this.set_message("Image insertion cancelled".into());
                        cx.notify();
                        return;
                    }
                };
                if let Err(error) = result {
                    this.set_message(format!("Image import failed: {error:#}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_image_import(
        &mut self,
        id: u64,
        inputs: Vec<ImageInput>,
        caret: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let document = self
            .documents
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("The original document was closed"))?;
        if !document.is_markdown() {
            bail!("Images require a Markdown (.md) document");
        }
        let path = document
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Save the document before inserting images"))?
            .to_path_buf();
        let token = document.recovery_token();
        let caret = caret.unwrap_or((document.buffer.row, document.buffer.col));
        let count = inputs.len();
        let assets_dir = self.config.image_assets_dir.clone();
        self.set_message(format!("Importing {count} image(s)…"));
        cx.notify();
        let worker_path = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { image_assets::import_images(&worker_path, &assets_dir, &inputs) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let imported = result.is_ok();
                let result = result.and_then(|text| {
                    let document = this
                        .documents
                        .get(id)
                        .ok_or_else(|| anyhow::anyhow!("The original document was closed"))?;
                    // Never insert a stale caret into edited/reloaded contents or
                    // relative asset links into a document saved under another path.
                    if document.path.as_ref() != Some(&path) || document.recovery_token() != token {
                        bail!("The original document changed; paste or drop the images again");
                    }
                    this.finish_image_import(id, &text, caret, cx)
                });
                this.set_message(match result {
                    Ok(()) => format!("Inserted {count} local image(s)"),
                    Err(error) if imported => {
                        format!("Imported {count} local image(s); insertion skipped: {error:#}")
                    }
                    Err(error) => format!("Image import failed: {error:#}"),
                });
                // Copying can succeed even when a concurrent document change
                // prevents inserting the links. Files still needs the new assets.
                if imported {
                    if this.explorer.dirty() {
                        this.message.push_str(
                            " · Files refresh deferred: commit or undo pending Files edits, then :e",
                        );
                    } else if let Err(error) = this.explorer.reload() {
                        this.message
                            .push_str(&format!(" · Files refresh failed: {error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    fn finish_image_import(
        &mut self,
        id: u64,
        text: &str,
        caret: (usize, usize),
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let active = self.documents.active_id();
        self.documents.select(id)?;
        let buffer = &mut self.documents.current_mut().buffer;
        buffer.row = caret.0;
        buffer.col = caret.1;
        let insert_mode = buffer.mode == Mode::Insert;
        if insert_mode {
            // Finish preceding typing without moving the source insertion caret.
            let col = buffer.col;
            buffer.key("escape");
            buffer.mode = Mode::Insert;
            buffer.col = col;
        }
        buffer.insert_text(text);
        if insert_mode {
            // Isolate the import from both preceding and subsequent Insert typing.
            let col = buffer.col;
            buffer.key("escape");
            buffer.mode = Mode::Insert;
            buffer.col = col;
        }
        self.documents.select(active)?;
        if active == id {
            self.marked = None;
            self.preferred_visual_x = None;
            self.follow_cursor = true;
            self.refresh_projection();
        }
        self.checkpoint_recovery(cx);
        Ok(())
    }
}
