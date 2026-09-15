use crate::surface::{Surface, SurfaceLayout};
use crate::{
    config::{Config, Theme, ThemePreset, parse_color},
    document::Document,
    explorer::Explorer,
    markdown::{self, RenderedLine},
    outline::{self, Heading},
    recovery::RecoveryStore,
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
        NewDocument,
        OpenDocument,
        SaveAs,
        Paste,
        ExplorerToggle,
        TerminalToggle,
        EditorPane,
        HelpToggle,
        ThemesToggle,
        OutlineToggle,
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
            "new" => Box::new(NewDocument),
            "open" => Box::new(OpenDocument),
            "save-as" => Box::new(SaveAs),
            "paste" => Box::new(Paste),
            "explorer" => Box::new(ExplorerToggle),
            "terminal" => Box::new(TerminalToggle),
            "editor" => Box::new(EditorPane),
            "help" => Box::new(HelpToggle),
            "themes" => Box::new(ThemesToggle),
            "outline" => Box::new(OutlineToggle),
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

type CloseContinuation = (Option<u64>, Vec<(u64, String)>);

struct ThemePicker {
    original: Theme,
    presets: Vec<Theme>,
    selected: usize,
}

struct OutlinePicker {
    query: String,
    col: usize,
    matches: Vec<usize>,
    selected: usize,
    scroll: ScrollHandle,
    input_bounds: Bounds<Pixels>,
    input_line: Option<ShapedLine>,
}

pub struct Workspace {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub documents: crate::documents::Documents,
    pub explorer: Explorer,
    pub explorer_visible: bool,
    explorer_resize: Option<(Pixels, f32)>,
    mouse_anchor: Option<(Pane, usize, usize)>,
    preferred_visual_x: Option<Pixels>,
    pub terminal: Option<Terminal>,
    pub terminal_visible: bool,
    terminal_task: Option<Task<()>>,
    recovery: Option<RecoveryStore>,
    recovery_task: Option<Task<()>>,
    recovery_error: Option<String>,
    pub(crate) viewport_needs_measurement: bool,
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
    help_section: usize,
    theme_picker: Option<ThemePicker>,
    headings: Vec<Heading>,
    outline_picker: Option<OutlinePicker>,
    pub marked: Option<Range<usize>>,
    dialog_pending: bool,
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
            markdown::project(&document.buffer.text(), config.theme.is_dark())
        } else {
            Vec::new()
        };
        let explorer_visible = document.path.is_none();
        let headings = if document.is_markdown() {
            outline::headings(&document.buffer.text())
        } else {
            Vec::new()
        };
        Self {
            config,
            config_path,
            documents: crate::documents::Documents::new(document),
            explorer,
            explorer_visible,
            explorer_resize: None,
            mouse_anchor: None,
            preferred_visual_x: None,
            terminal: None,
            terminal_visible: false,
            terminal_task: None,
            recovery: None,
            recovery_task: None,
            recovery_error: None,
            viewport_needs_measurement: false,
            pane: Pane::Editor,
            focus,
            layouts: Default::default(),
            tops: [0; 3],
            scroll_offsets: [0.; 3],
            row_heights: Default::default(),
            projection,
            command: None,
            message: if cfg!(target_os = "macos") {
                "i to write  ·  Cmd-S save  ·  F1 help"
            } else {
                "i to write  ·  Ctrl-S save  ·  F1 help"
            }
            .into(),
            help: false,
            help_section: 0,
            theme_picker: None,
            headings,
            outline_picker: None,
            marked: None,
            dialog_pending: false,
            search: String::new(),
            window_prefix: false,
            follow_cursor: true,
            pane_heights: [0.; 3],
            viewport_alignment: None,
        }
    }

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

    fn capture_viewport(&mut self) {
        let editor = Pane::Editor.index();
        *self.documents.viewport_mut() = crate::documents::Viewport {
            top: self.tops[editor],
            offset: self.scroll_offsets[editor],
        };
    }

    fn checkpoint_recovery(&mut self, cx: &mut Context<Self>) {
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

    fn finish_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        if self.recovery.is_none() {
            return true;
        }
        self.capture_viewport();
        if let Err(error) = self.recovery.as_ref().unwrap().finish(&self.documents) {
            let message = format!(
                "Close cancelled: recovery session could not be finalized: {error:#}. Check recovery-folder permissions and free space, then close again."
            );
            eprintln!("rockdown: {message}");
            self.message = message.clone();
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
        if document.is_markdown() {
            let source = document.buffer.text();
            self.projection = markdown::project(&source, self.config.theme.is_dark());
            self.headings = outline::headings(&source);
        } else {
            self.projection.clear();
            self.headings.clear();
        }
        self.filter_outline();
        self.row_heights[Pane::Editor.index()].clear();
    }

    fn dismiss_outline(&mut self) {
        if self.outline_picker.take().is_some() {
            self.marked = None;
        }
    }

    fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        if self.outline_picker.is_some() {
            self.dismiss_outline();
            return;
        }
        self.help = false;
        self.set_pane(Pane::Editor, cx);
        let selected = self
            .headings
            .partition_point(|heading| heading.row <= self.documents.current().buffer.row)
            .saturating_sub(1);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        self.outline_picker = Some(OutlinePicker {
            query: String::new(),
            col: 0,
            matches: (0..self.headings.len()).collect(),
            selected,
            scroll,
            input_bounds: Bounds::default(),
            input_line: None,
        });
    }

    fn filter_outline(&mut self) {
        let Some(picker) = &mut self.outline_picker else {
            return;
        };
        let query = picker.query.to_lowercase();
        picker.matches = self
            .headings
            .iter()
            .enumerate()
            .filter(|(_, heading)| heading.title.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        picker.selected = 0;
        picker.scroll.scroll_to_item(0);
    }

    fn jump_to_heading(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_outline();
        self.set_pane(Pane::Editor, cx);
        let buffer = &mut self.documents.current_mut().buffer;
        buffer.row = row.min(buffer.lines.len() - 1);
        buffer.col = 0;
        self.viewport_alignment = Some((Pane::Editor, false));
        self.follow_cursor = false;
        self.focus.focus(window);
        cx.notify();
    }

    fn outline_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picker = self.outline_picker.as_ref().expect("open outline");
        let panel = self.color(&self.config.theme.panel);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let current = self
            .headings
            .partition_point(|heading| heading.row <= self.documents.current().buffer.row)
            .checked_sub(1);
        let empty = if !self.documents.current().is_markdown() {
            "Outline is available for Markdown documents (.md)."
        } else if self.headings.is_empty() {
            "No headings yet. Add a Markdown heading to navigate."
        } else {
            "No headings match your search."
        };
        let entity = cx.entity();
        let input_entity = entity.clone();
        div()
            .id("outline-overlay")
            .absolute()
            .inset_0()
            .p_4()
            .flex()
            .items_center()
            .justify_end()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| {
                this.dismiss_outline();
                this.focus.focus(window);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("outline-panel")
                    .debug_selector(|| "outline-panel".into())
                    .w(px(440.))
                    .max_w_full()
                    .max_h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_xl()
                    .bg(panel)
                    .border_1()
                    .border_color(muted.opacity(0.25))
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .p_4()
                            .flex_shrink_0()
                            .text_color(foreground)
                            .child("Document Outline")
                            .child(
                                div()
                                    .mt_2()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child("Type to filter headings"),
                            ),
                    )
                    .child(
                        div()
                            .mx_4()
                            .mb_2()
                            .px_2()
                            .rounded_md()
                            .bg(self.color(&self.config.theme.background))
                            .child(
                                canvas(
                                    move |bounds, window, cx| {
                                        input_entity.update(cx, |this, _| {
                                            let picker =
                                                this.outline_picker.as_mut().expect("open outline");
                                            let line = window.text_system().shape_line(
                                                picker.query.clone().into(),
                                                px(14.),
                                                &[TextRun {
                                                    len: picker.query.len(),
                                                    font: font(this.config.font_family.clone()),
                                                    color: foreground,
                                                    background_color: None,
                                                    underline: None,
                                                    strikethrough: None,
                                                }],
                                                None,
                                            );
                                            picker.input_bounds = bounds;
                                            picker.input_line = Some(line.clone());
                                            (line, picker.col)
                                        })
                                    },
                                    move |bounds, (line, col), window, cx| {
                                        let focus = entity.read(cx).focus.clone();
                                        window.handle_input(
                                            &focus,
                                            ElementInputHandler::new(bounds, entity.clone()),
                                            cx,
                                        );
                                        let x = line.x_for_index(col);
                                        let shift = (x - bounds.size.width + px(3.)).max(px(0.));
                                        let origin = bounds.origin - point(shift, px(0.));
                                        window.with_content_mask(
                                            Some(ContentMask { bounds }),
                                            |window| {
                                                if let Err(error) =
                                                    line.paint(origin, px(30.), window, cx)
                                                {
                                                    eprintln!("Outline search rendering: {error}");
                                                }
                                                window.paint_quad(fill(
                                                    Bounds::new(
                                                        origin + point(x, px(5.)),
                                                        size(px(1.), px(20.)),
                                                    ),
                                                    accent,
                                                ));
                                            },
                                        );
                                    },
                                )
                                .w_full()
                                .h(px(30.)),
                            ),
                    )
                    .child(
                        div()
                            .id("outline-results")
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&picker.scroll)
                            .px_2()
                            .pb_2()
                            .when(picker.matches.is_empty(), |list| {
                                list.child(
                                    div()
                                        .id("outline-empty")
                                        .debug_selector(|| "outline-empty".into())
                                        .p_4()
                                        .text_size(px(13.))
                                        .text_color(muted)
                                        .child(empty),
                                )
                            })
                            .children(picker.matches.iter().enumerate().map(
                                |(position, &index)| {
                                    let heading = &self.headings[index];
                                    let row = heading.row;
                                    div()
                                        .id(("outline-heading", index))
                                        .debug_selector(move || format!("outline-heading-{index}"))
                                        .h(px(34.))
                                        .flex_shrink_0()
                                        .pl(px(10. + f32::from(heading.level - 1) * 14.))
                                        .pr_2()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(if position == picker.selected {
                                            accent.opacity(0.12)
                                        } else {
                                            transparent_black()
                                        })
                                        .hover(|style| style.bg(accent.opacity(0.18)))
                                        .text_size(px(13.))
                                        .text_color(foreground)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.jump_to_heading(row, window, cx);
                                            cx.stop_propagation();
                                        }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(heading.title.clone()),
                                        )
                                        .when(current == Some(index), |item| {
                                            item.child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(accent)
                                                    .child("current"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(muted)
                                                .child(format!("{}", row + 1)),
                                        )
                                },
                            )),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(muted.opacity(0.15))
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("↑ ↓ select · Enter jump · Esc close"),
                    ),
            )
    }

    fn open_theme_picker(&mut self, cx: &mut Context<Self>) {
        self.help = false;
        self.dismiss_outline();
        self.theme_picker = Some(ThemePicker {
            original: self.config.theme.clone(),
            presets: ThemePreset::ALL
                .iter()
                .map(|preset| preset.theme())
                .collect(),
            selected: 0,
        });
        cx.notify();
    }

    fn preview_theme(&mut self, selected: usize, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.theme_picker else {
            return;
        };
        if picker.selected == selected {
            return;
        }
        picker.selected = selected;
        let theme = if selected == 0 {
            &picker.original
        } else {
            &picker.presets[selected - 1]
        };
        let reproject = self.config.theme.is_dark() != theme.is_dark();
        self.config.theme = theme.clone();
        if reproject {
            self.refresh_projection();
        }
        cx.notify();
    }

    fn finish_theme_picker(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(picker) = self.theme_picker.take() else {
            return;
        };
        if keep {
            if picker.selected != 0 {
                self.message = format!(
                    "{} theme · this session only",
                    self.config.theme.preset.name()
                );
            }
        } else {
            let reproject = self.config.theme.is_dark() != picker.original.is_dark();
            self.config.theme = picker.original;
            if reproject {
                self.refresh_projection();
            }
        }
        cx.notify();
    }

    fn help_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self.color(&self.config.theme.panel);
        let background = self.color(&self.config.theme.background);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        let section = &HELP[self.help_section];
        div()
            .id("help-overlay")
            .absolute()
            .inset_0()
            .p_4()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(black().opacity(0.28))
            .on_click(cx.listener(|this, _, _, cx| {
                this.help = false;
                cx.notify();
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("help-sheet")
                    .debug_selector(|| "help-sheet".into())
                    .w_full()
                    .max_w(px(880.))
                    .h_full()
                    .max_h(px(640.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_xl()
                    .shadow_lg()
                    .bg(panel)
                    .border_1()
                    .border_color(muted.opacity(0.2))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px_6()
                            .py_4()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(accent)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("ROCKDOWN / REFERENCE"),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_size(px(24.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Make yourself at home."),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-help")
                                    .debug_selector(|| "close-help".into())
                                    .size(px(30.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(20.))
                                    .text_color(muted)
                                    .hover(|style| {
                                        style.bg(muted.opacity(0.12)).text_color(foreground)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.help = false;
                                        cx.notify();
                                        cx.stop_propagation();
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .pb_3()
                            .flex_shrink_0()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .children(HELP.iter().enumerate().map(|(index, topic)| {
                                let selected = index == self.help_section;
                                div()
                                    .id(("help-topic", index))
                                    .debug_selector(move || format!("help-topic-{index}"))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if selected { accent } else { muted })
                                    .bg(if selected {
                                        accent.opacity(0.12)
                                    } else {
                                        transparent_black()
                                    })
                                    .hover(|style| style.bg(muted.opacity(0.1)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.help_section = index;
                                        cx.notify();
                                        cx.stop_propagation();
                                    }))
                                    .child(topic.title)
                            })),
                    )
                    .child(
                        div()
                            .id(("help-content", self.help_section))
                            .debug_selector(|| "help-content".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_6()
                            .py_5()
                            .bg(background)
                            .border_t_1()
                            .border_color(muted.opacity(0.12))
                            .child(
                                div()
                                    .mb_1()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(section.title),
                            )
                            .child(
                                div()
                                    .mb_5()
                                    .text_size(px(13.))
                                    .text_color(muted)
                                    .child(section.summary),
                            )
                            .children(section.shortcuts.iter().enumerate().map(
                                |(index, (keys, description))| {
                                    div()
                                        .py_3()
                                        .flex()
                                        .items_start()
                                        .gap_4()
                                        .when(index > 0, |row| {
                                            row.border_t_1().border_color(muted.opacity(0.1))
                                        })
                                        .child(
                                            div().w(px(188.)).flex_shrink_0().child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .bg(panel)
                                                    .border_1()
                                                    .border_color(muted.opacity(0.16))
                                                    .text_color(foreground)
                                                    .font_family(self.config.font_family.clone())
                                                    .text_size(px(11.))
                                                    .child(*keys),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .pt_1()
                                                .text_size(px(13.))
                                                .text_color(foreground)
                                                .child(*description),
                                        )
                                },
                            ))
                            .child(
                                div()
                                    .mt_5()
                                    .p_4()
                                    .rounded_lg()
                                    .bg(panel)
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(accent)
                                            .child("GOOD TO KNOW"),
                                    )
                                    .children(section.notes.iter().map(|note| {
                                        div().text_size(px(12.)).text_color(muted).child(*note)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px_6()
                            .py_3()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_t_1()
                            .border_color(muted.opacity(0.12))
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("← → / Tab  Browse topics")
                            .child("Esc  Close guide"),
                    ),
            )
    }

    fn theme_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picker = self.theme_picker.as_ref().expect("open theme picker");
        let panel = self.color(&self.config.theme.panel);
        let foreground = self.color(&self.config.theme.foreground);
        let muted = self.color(&self.config.theme.muted);
        let accent = self.color(&self.config.theme.accent);
        div()
            .id("theme-overlay")
            .absolute()
            .inset_0()
            .p_4()
            .flex()
            .items_center()
            .justify_end()
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.finish_theme_picker(false, cx);
                cx.stop_propagation();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("theme-selector")
                    .w(px(320.))
                    .max_h_full()
                    .debug_selector(|| "theme-selector".into())
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_xl()
                    .bg(panel)
                    .border_1()
                    .border_color(muted.opacity(0.25))
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .p_4()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Appearance"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child("Preview a colorscheme"),
                            ),
                    )
                    .child(
                        div()
                            .id("theme-options")
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_2()
                            .pb_2()
                            .children(
                                std::iter::once(&picker.original)
                                    .chain(picker.presets.iter())
                                    .enumerate()
                                    .map(|(index, theme)| {
                                        let selected = picker.selected == index;
                                        let name = if index == 0 {
                                            "Current theme"
                                        } else {
                                            theme.preset.name()
                                        };
                                        let description =
                                            if theme.is_dark() { "Dark" } else { "Light" };
                                        div()
                                            .id(("theme-option", index))
                                            .h(px(42.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .debug_selector(move || format!("theme-option-{index}"))
                                            .rounded_md()
                                            .cursor_pointer()
                                            .bg(if selected {
                                                accent.opacity(0.12)
                                            } else {
                                                transparent_black()
                                            })
                                            .border_1()
                                            .border_color(if selected {
                                                accent.opacity(0.35)
                                            } else {
                                                transparent_black()
                                            })
                                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                                if *hovered {
                                                    this.preview_theme(index, cx);
                                                }
                                            }))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.preview_theme(index, cx);
                                                this.finish_theme_picker(true, cx);
                                                cx.stop_propagation();
                                            }))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_size(px(13.))
                                                    .text_color(foreground)
                                                    .child(name),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(muted)
                                                    .child(description),
                                            )
                                            .child(
                                                div().flex().gap_1().children(
                                                    [
                                                        &theme.background,
                                                        &theme.foreground,
                                                        &theme.accent,
                                                    ]
                                                    .into_iter()
                                                    .map(|hex| {
                                                        div()
                                                            .size(px(10.))
                                                            .rounded_full()
                                                            .border_1()
                                                            .border_color(muted.opacity(0.25))
                                                            .bg(self.color(hex))
                                                    }),
                                                ),
                                            )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(muted.opacity(0.15))
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("↑ ↓ / j k preview · Enter keep · Esc cancel")
                            .child(
                                div()
                                    .mt_1()
                                    .child("Session only · config file stays untouched"),
                            ),
                    ),
            )
    }

    fn change_document(
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
        self.message = format!(
            "Buffer {} · {} open",
            self.documents.active_id(),
            self.documents.entries().len()
        );
        self.checkpoint_recovery(cx);
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
    fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                            this.message = format!("{error:#}");
                        }
                    }
                    Ok(Err(error)) => this.message = format!("{error:#}"),
                    _ => this.message = "Open cancelled".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_dialog(
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
                    Ok(Err(error)) => this.message = format!("{error:#}"),
                    _ => this.message = "Save cancelled".into(),
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
                self.message = format!(
                    "Saved {}",
                    self.documents
                        .get(id)
                        .and_then(|doc| doc.path.as_ref())
                        .unwrap()
                        .display()
                );
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
            Err(error) => self.message = format!("{error:#}"),
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
                            Err(error) => this.message = format!("{error:#}"),
                        }
                    }
                    Ok(0) => this.save_dialog(id, false, Some((target, discarded)), window, cx),
                    Ok(1) => {
                        let mut discarded = discarded;
                        discarded.push((id, snapshot));
                        this.request_close(target, discarded, window, cx);
                    }
                    _ => this.message = "Close cancelled".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.viewport_alignment = None;
        self.preferred_visual_x = None;
        self.mouse_anchor = None;
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
        if self.help && !matches!(action, "help" | "themes" | "outline") {
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
            "editor" => self.set_pane(Pane::Editor, cx),
            "help" => {
                self.dismiss_outline();
                self.help = !self.help;
            }
            "themes" => self.open_theme_picker(cx),
            "outline" => self.toggle_outline(cx),
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
            "help" => self.help = true,
            "theme" | "themes" => self.open_theme_picker(cx),
            "outline" => self.toggle_outline(cx),
            "config" => {
                let (config, path) = Config::load(self.config_path.as_deref())?;
                bind_config_keys(&config, cx);
                self.config = config;
                self.config_path = path;
                self.refresh_projection();
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
        if self.theme_picker.is_none() && self.outline_picker.is_none() {
            self.follow_cursor = true;
            self.viewport_alignment = None;
        }
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
            self.request_window_close(window, cx);
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
            let count = ThemePreset::ALL.len() + 1;
            match key {
                "escape" => self.finish_theme_picker(false, cx),
                "enter" => self.finish_theme_picker(true, cx),
                "up" | "k" => self.preview_theme((selected + count - 1) % count, cx),
                "down" | "j" | "tab" => self.preview_theme((selected + 1) % count, cx),
                "home" => self.preview_theme(0, cx),
                "end" => self.preview_theme(count - 1, cx),
                _ => {}
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
    fn type_text(&mut self, text: &str) {
        if self.help || self.theme_picker.is_some() {
            return;
        }
        if self.outline_picker.is_some() {
            self.replace_input(None, text);
            return;
        }
        self.preferred_visual_x = None;
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
    /// Shape the destination as editable source, even while it is still shown as preview.
    /// This keeps arrow movement stable across Markdown syntax and heading font changes.
    fn editable_navigation_row(
        &self,
        source_row: usize,
        window: &mut Window,
    ) -> Option<crate::surface::HitRow> {
        let width = self.layouts[Pane::Editor.index()].text_width;
        if width <= px(0.) {
            return None;
        }
        let text: SharedString = self.buffer().lines.get(source_row)?.text.clone().into();
        let runs = [TextRun {
            len: text.len(),
            font: font(self.config.font_family.clone()),
            color: self.color(&self.config.theme.foreground),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let line =
            window
                .text_system()
                .shape_line(text.clone(), px(self.config.font_size), &runs, None);
        let wrap = self
            .projection
            .get(source_row)
            .is_some_and(|row| row.table.is_none() && row.kind != markdown::BlockKind::Code);
        let wrapped = if wrap && line.width > width {
            window
                .text_system()
                .shape_text(text, px(self.config.font_size), &runs, Some(width), None)
                .ok()?
                .pop()
        } else {
            None
        };
        let line_height = px(self.config.line_height);
        let height = line_height
            * wrapped
                .as_ref()
                .map_or(1, |line| line.wrap_boundaries.len() + 1) as f32;
        Some(crate::surface::HitRow {
            source_row,
            origin: point(px(0.), px(0.)),
            line,
            raw: true,
            wrapped,
            line_height,
            height,
        })
    }

    fn mouse_location(&self, pane: Pane, position: Point<Pixels>) -> Option<(usize, usize)> {
        let row = self.layouts[pane.index()]
            .rows
            .iter()
            .find(|row| position.y >= row.origin.y && position.y < row.origin.y + row.height)?;
        let index = row.index_for_position(position);
        Some((
            row.source_row,
            if row.raw {
                index
            } else {
                self.projection
                    .get(row.source_row)
                    .map_or(0, |projection| projection.source_column(index))
            },
        ))
    }

    fn mouse_move(&mut self, pane: Pane, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.mouse_anchor = None;
            return;
        }
        let Some((anchor_pane, row, col)) = self.mouse_anchor else {
            return;
        };
        if anchor_pane != pane {
            return;
        }
        if let Some(head) = self.mouse_location(pane, event.position)
            && (head != (row, col) || matches!(self.buffer().mode, Mode::Visual | Mode::VisualLine))
        {
            self.buffer_mut().select_with_mouse((row, col), head);
            cx.notify();
        }
    }

    pub fn mouse_down(
        &mut self,
        pane: Pane,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mouse_anchor = None;
        self.preferred_visual_x = None;
        let hit = self.layouts[pane.index()].rows.iter().find(|row| {
            event.position.y >= row.origin.y && event.position.y < row.origin.y + row.height
        });
        if pane == Pane::Editor
            && self.documents.current().is_markdown()
            && (event.modifiers.platform || event.modifiers.control)
        {
            let link = self.layouts[pane.index()]
                .links
                .iter()
                .find(|link| link.bounds.contains(&event.position))
                .map(|link| link.url.as_str())
                .or_else(|| {
                    let row = hit.filter(|row| !row.raw)?;
                    let projection = self.projection.get(row.source_row)?;
                    let mut offset = 0;
                    for span in &projection.spans {
                        let start = offset;
                        offset += span.text.len();
                        let Some(url) = span.link.as_deref() else {
                            continue;
                        };
                        for (index, character) in span.text.char_indices() {
                            let index = start + index;
                            let origin = row.position_for_index(index);
                            let width = row.line.x_for_index(index + character.len_utf8())
                                - row.line.x_for_index(index);
                            if Bounds::new(origin, size(width, row.line_height))
                                .contains(&event.position)
                            {
                                return Some(url);
                            }
                        }
                    }
                    None
                });
            if let Some(link) = link {
                if link.starts_with('#') {
                    if let Some(row) = outline::resolve_anchor(&self.headings, link) {
                        self.jump_to_heading(row, window, cx);
                    } else {
                        self.message = format!("No heading matches {link}");
                    }
                } else {
                    self.message =
                        "Only #fragment links within this document are navigated.".into();
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        let task = hit.and_then(|row| {
            if pane != Pane::Editor
                || !self.documents.current().is_markdown()
                || event.click_count != 1
                || event.modifiers.shift
                || event.modifiers.control
                || event.modifiers.alt
                || event.modifiers.platform
            {
                return None;
            }
            let marker = self.projection.get(row.source_row)?.task_marker?;
            let display = if row.raw {
                marker
            } else {
                row.line
                    .text
                    .find("[ ]")
                    .or_else(|| row.line.text.find("[x]"))?
            };
            // Check actual glyph rectangles: a narrow column can wrap even the
            // marker, and later visual rows must remain ordinary text targets.
            (display..display + 3)
                .any(|index| {
                    let origin = row.position_for_index(index);
                    let width = row.line.x_for_index(index + 1) - row.line.x_for_index(index);
                    event.position.x >= origin.x
                        && event.position.x < origin.x + width
                        && event.position.y >= origin.y
                        && event.position.y < origin.y + row.line_height
                })
                .then_some((row.source_row, marker))
        });
        if let Some((row, marker)) = task {
            self.set_pane(pane, cx);
            self.focus.focus(window);
            if self.buffer_mut().toggle_markdown_task(row, marker) {
                self.marked = None;
                self.refresh_projection();
            }
            cx.notify();
            return;
        }
        let location = self.mouse_location(pane, event.position);
        self.set_pane(pane, cx);
        self.focus.focus(window);
        if pane != Pane::Terminal
            && let Some((row, col)) = location
        {
            if self.buffer().mode != Mode::Insert {
                self.buffer_mut().key("escape");
            }
            let buffer = self.buffer_mut();
            buffer.row = row.min(buffer.lines.len() - 1);
            let text = &buffer.lines[buffer.row].text;
            buffer.col = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain((buffer.mode == Mode::Insert).then_some(text.len()))
                .take_while(|i| *i <= col)
                .last()
                .unwrap_or(0);
            if event.click_count == 2 {
                buffer.select_word_with_mouse();
            } else {
                self.mouse_anchor = Some((pane, buffer.row, buffer.col));
            }
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
            let top = &mut self.tops[pane.index()];
            if row < *top {
                *top = row;
                self.scroll_offsets[pane.index()] = 0.;
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
            .h(px(48.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_3()
            .gap_2()
            .overflow_x_scroll()
            .bg(panel)
            .border_b_1()
            .border_color(muted.opacity(0.12))
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
                    .h(px(32.))
                    .rounded_md()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .bg(if id == active { background } else { panel })
                    .hover(|style| {
                        style
                            .bg(muted.opacity(0.08))
                            .text_color(self.color(&self.config.theme.foreground))
                    })
                    .active(|style| style.bg(accent.opacity(0.12)))
                    .text_color(if id == active {
                        self.color(&self.config.theme.foreground)
                    } else {
                        muted
                    })
                    .border_1()
                    .border_color(if id == active {
                        muted.opacity(0.18)
                    } else {
                        transparent_black()
                    })
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
            .debug_selector(move || action.into())
            .w(px(if action == "outline" { 82. } else { 30. }))
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
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
                            "outline" => {
                                for (indent, y) in [(2., 5.), (6., 10.), (6., 15.)] {
                                    path.move_to(p(indent, y));
                                    path.line_to(p(18., y));
                                }
                            }
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
            .when(action == "outline", |button| button.child("Outline"))
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
            .on_mouse_move(cx.listener(move |this, event, _, cx| this.mouse_move(pane, event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.mouse_anchor = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.mouse_anchor = None),
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
                Mode::VisualLine => "VISUAL LINE",
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(
                cx.listener(|this, _: &Save, window, cx| this.run_action("save", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewDocument, window, cx| this.run_action("new", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenDocument, window, cx| {
                    this.run_action("open", window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SaveAs, window, cx| this.run_action("save-as", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Paste, window, cx| this.run_action("paste", window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExplorerToggle, window, cx| {
                this.run_action("explorer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TerminalToggle, window, cx| {
                this.run_action("terminal", window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &EditorPane, window, cx| {
                    this.run_action("editor", window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &HelpToggle, window, cx| this.run_action("help", window, cx)),
            )
            .on_action(cx.listener(|this, _: &ThemesToggle, window, cx| {
                this.run_action("themes", window, cx)
            }))
            .on_action(cx.listener(|this, _: &OutlineToggle, window, cx| {
                this.run_action("outline", window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousBuffer, window, cx| {
                this.run_action("previous-buffer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextBuffer, window, cx| {
                this.run_action("next-buffer", window, cx)
            }))
            .on_action(cx.listener(|this, _: &BufferDelete, window, cx| {
                this.run_action("buffer-delete", window, cx)
            }))
            .on_mouse_move(cx.listener(Self::resize_explorer))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.stop_explorer_resize(cx)),
            )
            .when(self.explorer_resize.is_some(), |root| {
                root.cursor(CursorStyle::ResizeLeftRight)
            })
            .when(cfg!(target_os = "macos"), |root| {
                root.child(
                    div()
                        .id("window-title")
                        .h(px(40.))
                        .flex_shrink_0()
                        .px(px(80.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(panel)
                        .text_size(px(12.))
                        .text_color(muted)
                        .window_control_area(WindowControlArea::Drag)
                        .on_mouse_down(MouseButton::Left, |event, window, _| {
                            if event.click_count == 2 {
                                window.zoom_window();
                            } else {
                                window.start_window_move();
                            }
                        })
                        .child(div().overflow_hidden().text_ellipsis().child(window_title)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(self.buffer_bar(cx))
                            .child(
                                div().flex_1().min_h_0().flex().justify_center().child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .h_full()
                                        .when(self.documents.current().is_markdown(), |column| {
                                            column.max_w(px(self.config.writing_width)).py_6()
                                        })
                                        .child(self.surface(Pane::Editor, cx)),
                                ),
                            ),
                    )
                    .when(self.explorer_visible, |body| {
                        body.child(
                            div()
                                .w(px(explorer_width))
                                .flex_shrink_0()
                                .flex()
                                .bg(panel)
                                .child(
                                    div()
                                        .id("explorer-splitter")
                                        .w(px(6.))
                                        .flex_shrink_0()
                                        .h_full()
                                        .cursor(CursorStyle::ResizeLeftRight)
                                        .child(div().w(px(1.)).h_full().bg(
                                            if self.pane == Pane::Explorer {
                                                accent.opacity(0.5)
                                            } else {
                                                muted.opacity(0.16)
                                            },
                                        ))
                                        .hover(|style| style.bg(accent.opacity(0.15)))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(Self::start_explorer_resize),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .h(px(48.))
                                                .flex_shrink_0()
                                                .px_4()
                                                .flex()
                                                .items_center()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(foreground)
                                                .child(if self.explorer.dirty() {
                                                    "Files  •"
                                                } else {
                                                    "Files"
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px_4()
                                                .pb_3()
                                                .text_size(px(11.))
                                                .text_color(muted)
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(
                                                    self.explorer.directory.display().to_string(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h_0()
                                                .child(self.surface(Pane::Explorer, cx)),
                                        ),
                                ),
                        )
                    }),
            )
            .when(self.terminal_visible, |root| {
                root.child(
                    div()
                        .h(px(self.config.terminal_height))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .border_t_1()
                        .border_color(muted.opacity(0.18))
                        .bg(background)
                        .child(
                            div()
                                .h(px(36.))
                                .flex_shrink_0()
                                .px_4()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(foreground)
                                        .child("Terminal"),
                                )
                                .child("Ctrl-` to hide"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(self.surface(Pane::Terminal, cx)),
                        ),
                )
            })
            .when_some(self.recovery_error.clone(), |root, error| {
                root.child(
                    div()
                        .id("recovery-error")
                        .debug_selector(|| "recovery-error".into())
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .bg(panel)
                        .border_t_1()
                        .border_color(accent)
                        .text_color(foreground)
                        .text_size(px(12.))
                        .child(error),
                )
            })
            .child(
                div()
                    .h(px(38.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(panel)
                    .border_t_1()
                    .border_color(muted.opacity(0.12))
                    .text_size(px(11.))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(accent.opacity(0.1))
                            .text_color(accent)
                            .font_weight(FontWeight::MEDIUM)
                            .child(mode),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(if self.command.is_some() {
                                foreground
                            } else {
                                muted
                            })
                            .child(status),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(muted)
                            .font_family(self.config.font_family.clone())
                            .child(location),
                    )
                    .child(
                        div()
                            .id("themes")
                            .px_2()
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .cursor_pointer()
                            .debug_selector(|| "themes".into())
                            .text_color(muted)
                            .hover(|style| style.bg(accent.opacity(0.12)).text_color(foreground))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run_action("themes", window, cx)
                            }))
                            .child(div().size(px(8.)).rounded_full().bg(accent))
                            .child("Theme"),
                    )
                    .child(self.dock_button(
                        "Outline",
                        "outline",
                        self.outline_picker.is_some(),
                        cx,
                    ))
                    .when(self.terminal_visible, |footer| {
                        footer.child(self.dock_button("Hide Terminal", "terminal", true, cx))
                    })
                    .child(self.dock_button(explorer_label, "explorer", self.explorer_visible, cx))
                    .child(self.dock_button(
                        if self.help { "Hide Help" } else { "Show Help" },
                        "help",
                        self.help,
                        cx,
                    )),
            )
            .when(self.help, |root| root.child(self.help_panel(cx)))
            .when(self.theme_picker.is_some(), |root| {
                root.child(self.theme_selector(cx))
            })
            .when(self.outline_picker.is_some(), |root| {
                root.child(self.outline_panel(cx))
            })
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

struct HelpSection {
    title: &'static str,
    summary: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
    notes: &'static [&'static str],
}

const HELP: &[HelpSection] = &[
    HelpSection {
        title: "Editing",
        summary: "Write, select, and reshape text without leaving the keyboard.",
        shortcuts: &[
            (
                "i / a / I / A",
                "Insert before / after the cursor, or at the start / end of a line",
            ),
            ("o / O", "Open a line below / above and enter Insert mode"),
            ("Esc", "Return to Normal mode"),
            (
                "v / V",
                "Select characters / whole lines; switch selection type",
            ),
            (
                "x · dd · dw · d$",
                "Delete a character, line, word, or to the end of a line",
            ),
            ("cc / cw", "Change a line / word and enter Insert mode"),
            ("yy · p / P", "Yank a line; paste after / before the cursor"),
            ("y / d / c", "Yank / delete / change a visual selection"),
            (
                ">> / <<",
                "Indent / outdent; use > / < on a visual selection",
            ),
            ("u / Ctrl-R", "Undo / redo"),
            (
                "Return",
                "Split a line in Insert mode; open a line below in Normal mode",
            ),
            (
                "Shift-Return",
                "Insert a literal newline without continuing Markdown",
            ),
        ],
        notes: &[
            "Counts combine with motions and operators: 3j moves three lines; 2dd deletes two; 3>> indents three.",
            "Editor and Files share the system clipboard. Ctrl-C/V (Cmd-C/V on macOS) copies/pastes; Ctrl-Shift-V pastes in every pane. Yanks, deletes, and changes copy text; linewise yanks paste as whole lines. Visual p/P replaces the selection.",
        ],
    },
    HelpSection {
        title: "Navigation",
        summary: "Move through a document, control the viewport, and find text.",
        shortcuts: &[
            (
                "h / j / k / l",
                "Move left / down / up / right; arrow keys work too",
            ),
            ("w / b / e", "Move by words"),
            ("0 / $", "Go to the beginning / end of a line"),
            ("gg / G", "Go to the beginning / end of the document"),
            ("Ctrl-D / Ctrl-U", "Move down / up by half a page"),
            ("zz / zt", "Center the current line / align it to the top"),
            ("/text · n", "Find text; repeat the search"),
            ("Ctrl-W h / l / j", "Focus the editor / Files / terminal"),
            (
                "Ctrl/Cmd-Shift-O · :outline",
                "Search document headings; ↑/↓ selects, Enter jumps, Esc closes",
            ),
            (
                "Ctrl/Cmd-click a preview link",
                "Jump to a #fragment in this document; external links stay closed",
            ),
        ],
        notes: &[
            "Click to place the caret, double-click to select a word, or drag to select text. The mouse wheel scrolls. Wrapped prose stays editable; Insert-mode Up/Down follows visual rows.",
            "A count before Ctrl-D/U sets the number of lines for later half-page motions. A count before zz/zt first selects that line, for example 40zz.",
        ],
    },
    HelpSection {
        title: "Documents",
        summary: "Open files, manage buffers, and save with conflict protection.",
        shortcuts: &[
            ("Ctrl/Cmd-N / O", "Create a document / open a file"),
            ("Ctrl/Cmd-S", "Save the current document"),
            ("Ctrl/Cmd-Shift-S", "Save As"),
            (
                ":w [file] / :w!",
                "Save; explicitly overwrite a disk conflict",
            ),
            (
                ":e [file] / :e!",
                "Open or reload; force reload discards unsaved edits",
            ),
            ("Ctrl-PageUp / Down", "Switch to the previous / next buffer"),
            (":bp / :bn", "Previous / next buffer"),
            ("Cmd-W / Ctrl-Shift-W", "Close the current buffer"),
            (":bd / :bd!", "Close a buffer; ! discards unsaved changes"),
            (
                ":q / :q! / :wq",
                "Close / discard all and close / save and close",
            ),
            (":%s/old/new/[giI]", "Substitute across the buffer"),
        ],
        notes: &[
            "Click a tab to select it or its × to close. Unsaved changes prompt Save / Discard / Cancel. Closing a buffer never deletes its file; opening another file preserves other edits.",
            "Buffer aliases: :bprevious / :previous-buffer, :bnext / :next-buffer, and :bdelete / :buffer-delete.",
            r"Substitution uses Rust regex: & inserts the match, \1 inserts a capture. g replaces every match; i/I controls case sensitivity. One u undoes the entire substitution.",
        ],
    },
    HelpSection {
        title: "Markdown",
        summary: "Plain text while you write. Rich preview when you move on.",
        shortcuts: &[
            (
                "Return",
                "Continue the current list, task, or quote in Insert mode",
            ),
            (
                "Shift-Return",
                "Skip list continuation and insert a literal newline",
            ),
            ("Click [ ] / [x]", "Toggle a task in source or preview"),
            ("![alt](pic.png)", "Display a local image inline"),
            (
                r#""40%" / "320px""#,
                "Use an image title to override its display width",
            ),
        ],
        notes: &[
            "Live preview applies to .md files (case-insensitive) and untitled buffers. Inactive lines render; the cursor line exposes editable syntax. Other files remain literal text. Save As updates the preview type.",
            "Prose wraps automatically. Local images default to 60% of the pane width; remote images remain linked alt text without network requests.",
            "Markdown styling supports headings, emphasis, code, lists, tasks, tables, and quotes. Your document stays plain UTF-8 text on disk.",
        ],
    },
    HelpSection {
        title: "Files & terminal",
        summary: "Keep the filesystem and a real shell close at hand.",
        shortcuts: &[
            ("Ctrl/Cmd-E", "Show / hide Files"),
            (":ex · Ctrl-W l", "Reveal and focus Files"),
            (
                "Return / -",
                "Open a file / navigate to the parent directory in Files",
            ),
            (
                "o · trailing /",
                "Create a listing entry; a trailing slash creates a directory",
            ),
            (
                "dd · :w · :e!",
                "Stage deletion / commit file changes / discard staging",
            ),
            ("Ctrl-`", "Show / hide the terminal"),
            ("Return / Ctrl-C", "Execute a shell command / interrupt it"),
            ("exit / Ctrl-D", "Close the shell / send EOF in Unix shells"),
        ],
        notes: &[
            "Edit filenames with Vim. Drag the drawer’s left edge to resize it; hiding Files preserves staged changes and width. Committed deletions go to .rockdown-trash.",
            "Ctrl-Shift-V (or Cmd-V on macOS) pastes into the terminal. Paste respects the shell’s bracketed-paste mode.",
        ],
    },
    HelpSection {
        title: "Appearance",
        summary: "Make the workspace yours without interrupting your writing.",
        shortcuts: &[
            (
                "Ctrl/Cmd-Shift-T",
                "Open the live theme picker; also available in the footer",
            ),
            (":theme", "Preview a built-in colorscheme"),
            ("↑ / ↓ · j / k", "Preview themes; hover works too"),
            ("Enter / Esc", "Keep a theme / cancel the preview"),
            (":config", "Reload your configuration"),
            ("--config PATH", "Launch with an explicit TOML config"),
        ],
        notes: &[
            "Theme choices are session-only. Set [theme] preset to rockdown, nord, dracula, gruvbox, paper, or solarized-light for startup. Explicit color overrides take precedence.",
            r"Config location: ~/.config/rockdown/config.toml on Unix; %APPDATA%\rockdown\config.toml on Windows. XDG_CONFIG_HOME overrides the base directory.",
            "markdown.colors controls normal, bold, italic, bold_italic, code, link, strikethrough, and quote colors. markdown.h1 through h6 accept font_size, color, and underline. markdown.divider sets color and thickness, also used for heading underlines.",
        ],
    },
];

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
        if let Some(picker) = &self.outline_picker {
            return &picker.query;
        }
        if self.pane == Pane::Terminal {
            ""
        } else if let Some(command) = &self.command {
            command
        } else {
            &self.buffer().lines[self.buffer().row].text
        }
    }
    fn input_col(&self) -> usize {
        if let Some(picker) = &self.outline_picker {
            return picker.col;
        }
        if self.pane == Pane::Terminal {
            0
        } else {
            self.command.as_ref().map_or(self.buffer().col, String::len)
        }
    }
    fn replace_input(&mut self, range: Option<Range<usize>>, text: &str) {
        if self.help || self.theme_picker.is_some() {
            return;
        }
        if let Some(picker) = &mut self.outline_picker {
            let range = range
                .or_else(|| self.marked.clone())
                .map(|range| {
                    utf8_offset(&picker.query, range.start)..utf8_offset(&picker.query, range.end)
                })
                .unwrap_or(picker.col..picker.col);
            let text = text.replace(['\r', '\n'], "");
            picker.col = range.start + text.len();
            picker.query.replace_range(range, &text);
            self.marked = None;
            self.filter_outline();
            return;
        }
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
            || (self.outline_picker.is_none()
                && self.command.is_none()
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
        if self.help || self.theme_picker.is_some() {
            return;
        }
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
        if let Some(picker) = &self.outline_picker {
            let line = picker.input_line.as_ref()?;
            let shift = (line.x_for_index(picker.col) - picker.input_bounds.size.width + px(3.))
                .max(px(0.));
            let start = utf8_offset(&picker.query, range.start);
            return Some(
                Bounds::new(
                    picker.input_bounds.origin
                        + point((line.x_for_index(start) - shift).max(px(0.)), px(0.)),
                    size(px(2.), px(30.)),
                )
                .intersect(&picker.input_bounds),
            );
        }
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        let start = utf8_offset(self.input_text(), range.start);
        Some(
            Bounds::new(
                row.position_for_index(start),
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
        if let Some(picker) = &self.outline_picker {
            let line = picker.input_line.as_ref()?;
            let shift = (line.x_for_index(picker.col) - picker.input_bounds.size.width + px(3.))
                .max(px(0.));
            let index = line.closest_index_for_x(position.x - picker.input_bounds.origin.x + shift);
            return Some(utf16_offset(&picker.query, index));
        }
        let row = self.layouts[self.pane.index()]
            .rows
            .iter()
            .find(|r| r.source_row == self.buffer().row)?;
        Some(utf16_offset(
            self.input_text(),
            row.index_for_position(position),
        ))
    }
}
