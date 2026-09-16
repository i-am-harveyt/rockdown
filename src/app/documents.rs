use super::{Pane, Workspace};
use crate::surface::SurfaceLayout;
use anyhow::{Result, bail};
use gpui::*;
use std::path::Path;

type CloseContinuation = (Option<u64>, Vec<(u64, String)>);

impl Workspace {
    pub(super) fn change_document(
        &mut self,
        change: impl FnOnce(&mut crate::documents::Documents) -> Result<()>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let editor = Pane::Editor.index();
        let previous = self.documents.active_id();
        self.capture_viewport();
        if let Err(error) = change(&mut self.documents) {
            self.documents.select(previous)?;
            return Err(error);
        }
        self.refresh_projection();
        self.layouts[editor] = SurfaceLayout::default();
        self.tops[editor] = self.documents.viewport().top;
        self.scroll_offsets[editor] = self.documents.viewport().offset;
        self.viewport_needs_measurement = true;
        self.set_pane(Pane::Editor, cx);
        self.follow_cursor = false;
        self.set_message(format!(
            "Buffer {} · {} open",
            self.documents.active_id(),
            self.documents.entries().len()
        ));
        self.checkpoint_recovery(cx);
        Ok(())
    }
    pub fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.documents.dirty() || self.explorer.dirty() {
            self.set_message(
                "Unsaved changes. Save with :w, or discard all and close with :q!".into(),
            );
            cx.notify();
            false
        } else {
            true
        }
    }
    pub(super) fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_pending {
            return;
        }
        self.dialog_pending = true;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open document".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.dialog_pending = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first()
                            && let Err(error) =
                                this.change_document(|documents| documents.open(path), cx)
                        {
                            this.set_message(format!("{error:#}"));
                        }
                    }
                    Ok(Err(error)) => this.set_message(format!("{error:#}")),
                    _ => this.set_message("Open cancelled".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_dialog(
        &mut self,
        id: u64,
        save_as: bool,
        close: Option<CloseContinuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog_pending {
            return;
        }
        let Some(document) = self.documents.get(id) else {
            return;
        };
        if !save_as && document.path.is_some() {
            self.finish_save(id, None, close, window, cx);
            return;
        }
        let directory = document
            .path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(&self.explorer.directory);
        let name = document
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.md");
        let receiver = cx.prompt_for_new_path(directory, Some(name));
        self.dialog_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_pending = false;
                match result {
                    Ok(Ok(Some(path))) => this.finish_save(id, Some(&path), close, window, cx),
                    Ok(Err(error)) => this.set_message(format!("{error:#}")),
                    _ => this.set_message("Save cancelled".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_save(
        &mut self,
        id: u64,
        path: Option<&Path>,
        close: Option<CloseContinuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.documents.save_id(id, path) {
            Ok(()) => {
                self.refresh_projection();
                self.set_message(format!(
                    "Saved {}",
                    self.documents
                        .get(id)
                        .and_then(|doc| doc.path.as_ref())
                        .unwrap()
                        .display()
                ));
                if !self.explorer.dirty()
                    && let Err(error) = self.explorer.reload()
                {
                    self.message
                        .push_str(&format!(" · Files refresh failed: {error:#}"));
                }
                if let Some((target, discarded)) = close {
                    self.request_close(target, discarded, window, cx);
                }
            }
            Err(error) => self.set_message(format!("{error:#}")),
        }
        cx.notify();
    }

    /// The OS close callback always defers removal to the same guarded workflow as tabs.
    pub fn request_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.request_close(None, Vec::new(), window, cx);
        false
    }

    fn request_close(
        &mut self,
        target: Option<u64>,
        discarded: Vec<(u64, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog_pending {
            return;
        }
        let pending = self
            .documents
            .entries()
            .iter()
            .find(|entry| {
                target.is_none_or(|id| id == entry.id)
                    && entry.document.buffer.dirty()
                    && !discarded.contains(&(entry.id, entry.document.buffer.text()))
            })
            .map(|entry| {
                (
                    entry.id,
                    entry.document.buffer.text(),
                    entry
                        .document
                        .path
                        .as_ref()
                        .map_or("Untitled".into(), |path| path.display().to_string()),
                )
            });
        // Explorer decisions are explicit: saving staged operations can move or trash files.
        let pending = pending.or_else(|| {
            let snapshot = format!(
                "{}\n{}\n{}",
                self.explorer.directory.display(),
                self.explorer.buffer.revision(),
                self.explorer.buffer.text()
            );
            (target.is_none()
                && self.explorer.dirty()
                && !discarded.contains(&(0, snapshot.clone())))
            .then_some((
                0,
                snapshot,
                "staged Files changes (renames, creations and moves to trash)".into(),
            ))
        });
        let Some((id, snapshot, name)) = pending else {
            if let Some(id) = target {
                self.delete_tab(id, window, cx);
            } else if self.finish_recovery(cx) {
                window.remove_window();
            }
            return;
        };
        let receiver = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {name}?"),
            Some("Unsaved changes will be lost if you discard them."),
            &["Save", "Discard", "Cancel"],
            cx,
        );
        self.dialog_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            let answer = receiver.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_pending = false;
                match answer {
                    Ok(0) if id == 0 => {
                        // Recheck the staged contents before applying the operations that were shown.
                        let current = format!(
                            "{}\n{}\n{}",
                            this.explorer.directory.display(),
                            this.explorer.buffer.revision(),
                            this.explorer.buffer.text()
                        );
                        if current != snapshot {
                            this.request_close(target, discarded, window, cx);
                            return;
                        }
                        match this.explorer.commit() {
                            Ok(report) => {
                                this.documents.reconcile(&report);
                                this.refresh_projection();
                                this.request_close(target, discarded, window, cx);
                            }
                            Err(error) => this.set_message(format!("{error:#}")),
                        }
                    }
                    Ok(0) => this.save_dialog(id, false, Some((target, discarded)), window, cx),
                    Ok(1) => {
                        let mut discarded = discarded;
                        discarded.push((id, snapshot));
                        this.request_close(target, discarded, window, cx);
                    }
                    _ => this.set_message("Close cancelled".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save(&mut self, path: Option<&Path>, force: bool) -> Result<()> {
        if self.pane == Pane::Explorer {
            if path.is_some() || force {
                bail!("Explorer commits use :w; :e! discards staged changes");
            }
            let report = self.explorer.commit()?;
            self.documents.reconcile(&report);
            self.refresh_projection();
            self.set_message(format!(
                "Explorer saved: {} created, {} renamed, {} moved to .rockdown-trash",
                report.created.len(),
                report.renamed.len(),
                report.deleted.len()
            ));
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
            self.set_message(format!(
                "Saved {}",
                self.documents.current().path.as_ref().unwrap().display()
            ));
            if !self.explorer.dirty() {
                self.explorer.reload()?;
            }
        }
        Ok(())
    }

    pub(super) fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(Some(id), Vec::new(), window, cx);
    }

    fn delete_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.documents.active_id();
        let result = self.change_document(
            |documents| {
                documents.select(id)?;
                documents.delete(true)?;
                if id != active {
                    documents.select(active)?;
                }
                Ok(())
            },
            cx,
        );
        if let Err(error) = result {
            self.set_message(error.to_string());
        }
        self.focus.focus(window);
        cx.notify();
    }
}
