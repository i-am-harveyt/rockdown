use super::{Pane, Workspace};
use crate::recovery::RecoveryStore;
use gpui::*;
use std::time::Duration;

impl Workspace {
    /// Opt in only at real application startup; ordinary workspaces stay storage-free.
    pub fn initialize_recovery(
        &mut self,
        store: Option<RecoveryStore>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let editor = Pane::Editor.index();
        self.tops[editor] = self.documents.viewport().top;
        self.scroll_offsets[editor] = self.documents.viewport().offset;
        self.follow_cursor = false;
        self.viewport_needs_measurement = true;
        self.recovery = store;
        self.recovery_error = error;
        if self.recovery.is_none() {
            return;
        }
        // A platform quit cannot be cancelled here. Preserve edits rather than
        // treating an unconfirmed quit as permission to discard them.
        cx.on_app_quit(|this, cx| {
            this.checkpoint_recovery(cx);
            async {}
        })
        .detach();
        self.recovery_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let running = this
                    .update(cx, |this, cx| {
                        this.checkpoint_recovery(cx);
                        this.recovery.is_some()
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        }));
    }

    pub(super) fn capture_viewport(&mut self) {
        let editor = Pane::Editor.index();
        *self.documents.viewport_mut() = crate::documents::Viewport {
            top: self.tops[editor],
            offset: self.scroll_offsets[editor],
        };
    }

    pub(super) fn checkpoint_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery.is_none() {
            return;
        }
        self.capture_viewport();
        match self.recovery.as_ref().unwrap().checkpoint(&self.documents) {
            Ok(()) => {
                if self.recovery_error.take().is_some() {
                    cx.notify();
                }
            }
            Err(error) => {
                let message = format!(
                    "Recovery checkpoint failed: {error:#}. Save with :w or Save As; check recovery-folder permissions and free space. Retrying every 2 seconds."
                );
                if self.recovery_error.as_ref() != Some(&message) {
                    eprintln!("rockdown: {message}");
                    self.recovery_error = Some(message);
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn finish_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        if self.recovery.is_none() {
            return true;
        }
        self.capture_viewport();
        if let Err(error) = self.recovery.as_ref().unwrap().finish(&self.documents) {
            let message = format!(
                "Close cancelled: recovery session could not be finalized: {error:#}. Check recovery-folder permissions and free space, then close again."
            );
            eprintln!("rockdown: {message}");
            self.set_message(message.clone());
            self.recovery_error = Some(message);
            cx.notify();
            return false;
        }
        // Stop periodic/shutdown checkpoints before they can revive discarded edits.
        self.recovery_task = None;
        self.recovery = None;
        self.recovery_error = None;
        true
    }
}
