use super::Workspace;
use crate::pdf::{self, ExportRequest};
use anyhow::{Result, bail};
use gpui::{prelude::*, *};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct PdfJob {
    destination: PathBuf,
    cancelled: Arc<AtomicBool>,
}

// A closing workspace must not leave an exporter running in the background.
impl Drop for PdfJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) enum PdfFeedback {
    Finished {
        destination: PathBuf,
        warnings: String,
    },
    Failed(String),
}

impl Workspace {
    pub(super) fn export_pdf_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.pdf_job.is_some() {
            bail!("A PDF export is already running; cancel it before starting another");
        }
        if self.dialog_pending {
            return Ok(());
        }
        let document = self.documents.current();
        if !document.is_markdown() {
            bail!("PDF export requires a Markdown (.md) document");
        }
        // Freeze text, resource base, and settings before showing the native dialog.
        // Switching tabs, saving elsewhere, or continuing to type cannot retarget it.
        let base_dir = document
            .path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(&self.explorer.directory)
            .to_path_buf();
        let name = document
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("Untitled.md"))
            .with_extension("pdf");
        let request = ExportRequest {
            source: document.buffer.text(),
            source_path: document.path.clone(),
            base_dir: base_dir.clone(),
            destination: PathBuf::new(),
            config: self.config.pdf.clone(),
        };
        let receiver = cx.prompt_for_new_path(&base_dir, name.to_str());
        self.dialog_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.dialog_pending = false;
                match result {
                    Ok(Ok(Some(mut path))) => {
                        if path.extension().is_none() {
                            path.set_extension("pdf");
                        }
                        this.start_pdf_export(
                            ExportRequest {
                                destination: path,
                                ..request
                            },
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        this.pdf_feedback = Some(PdfFeedback::Failed(format!("{error:#}")));
                    }
                    _ => this.set_message("PDF export cancelled".into()),
                }
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    fn start_pdf_export(&mut self, request: ExportRequest, cx: &mut Context<Self>) {
        let destination = request.destination.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pdf_feedback = None;
        self.pdf_job = Some(PdfJob {
            destination: destination.clone(),
            cancelled: cancelled.clone(),
        });
        let worker_cancelled = cancelled.clone();
        let task = cx
            .background_executor()
            .spawn(async move { pdf::export(request, &worker_cancelled) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let was_cancelled = cancelled.load(Ordering::Acquire);
                this.pdf_job = None;
                this.pdf_feedback = match result {
                    Ok(result) => {
                        let mut warnings = result.warnings;
                        if !this.explorer.dirty()
                            && let Err(error) = this.explorer.reload()
                        {
                            if !warnings.is_empty() {
                                warnings.push('\n');
                            }
                            warnings.push_str(&format!("Files refresh failed: {error:#}"));
                        }
                        Some(PdfFeedback::Finished {
                            destination,
                            warnings,
                        })
                    }
                    Err(_) if was_cancelled => {
                        this.set_message("PDF export cancelled; existing PDF unchanged".into());
                        None
                    }
                    Err(error) => Some(PdfFeedback::Failed(format!("{error:#}"))),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn cancel_pdf_export(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = &self.pdf_job {
            job.cancelled.store(true, Ordering::Release);
            cx.notify();
        }
    }

    pub(super) fn pdf_notice(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.color(&self.config.theme.accent);
        let muted = self.color(&self.config.theme.muted);
        let mut destination = None;
        let (title, detail) = if let Some(job) = &self.pdf_job {
            (
                if job.cancelled.load(Ordering::Acquire) {
                    "Cancelling PDF export…"
                } else {
                    "Exporting PDF…"
                },
                job.destination.display().to_string(),
            )
        } else {
            match &self.pdf_feedback {
                Some(PdfFeedback::Finished {
                    destination: path,
                    warnings,
                }) => {
                    destination = Some(path.clone());
                    (
                        if warnings.is_empty() {
                            "PDF exported"
                        } else {
                            "PDF exported with warnings"
                        },
                        if warnings.is_empty() {
                            path.display().to_string()
                        } else {
                            format!("{}\n{warnings}", path.display())
                        },
                    )
                }
                Some(PdfFeedback::Failed(error)) => ("PDF export failed", error.clone()),
                None => ("", String::new()),
            }
        };
        div()
            .id("pdf-notice")
            .debug_selector(|| "pdf-notice".into())
            .flex_shrink_0()
            .px_3()
            .py_2()
            .bg(self.color(&self.config.theme.panel))
            .border_t_1()
            .border_color(accent)
            .text_size(px(12.))
            .when(self.ui_mode == super::UiMode::Writer, |notice| {
                notice
                    .absolute()
                    .top(px(52.))
                    .left(px(80.))
                    .right(px(144.))
                    .rounded_lg()
                    .shadow_lg()
                    .occlude()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when_some(destination, |row, path| {
                        row.child(
                            div()
                                .id("open-exported-pdf")
                                .debug_selector(|| "open-exported-pdf".into())
                                .cursor_pointer()
                                .text_color(accent)
                                .child("Open PDF")
                                .on_click(move |_, _, cx| cx.open_with_system(&path)),
                        )
                    })
                    .child(
                        div()
                            .id("pdf-notice-action")
                            .debug_selector(|| "pdf-notice-action".into())
                            .cursor_pointer()
                            .text_color(accent)
                            .child(if self.pdf_job.is_some() {
                                "Cancel"
                            } else {
                                "Dismiss"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.pdf_job.is_some() {
                                    this.cancel_pdf_export(cx);
                                } else {
                                    this.pdf_feedback = None;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .id("pdf-notice-detail")
                    .max_h(px(144.))
                    .overflow_y_scroll()
                    .text_color(muted)
                    .child(detail),
            )
    }
}
