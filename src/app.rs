use crate::surface::SurfaceLayout;
use crate::{
    config::{Config, Theme, parse_color},
    document::Document,
    explorer::Explorer,
    markdown::{self, RenderedLine},
    outline::{self, Heading},
    recovery::RecoveryStore,
    terminal::Terminal,
    vim::Buffer,
};
use gpui::*;
use std::{ops::Range, path::PathBuf};

mod chrome;
mod commands;
mod documents;
mod help;
mod images;
mod keyboard;
mod navigation;
mod outline_picker;
mod panes;
mod recovery;
mod render;
mod text_input;
mod themes;

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

struct ThemePicker {
    original: Theme,
    original_markdown: crate::config::MarkdownStyle,
    presets: Vec<Theme>,
    selected: usize,
    scroll: ScrollHandle,
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
            markdown::project(&document.buffer.text(), &config.theme)
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
            self.projection = markdown::project(&source, &self.config.theme);
            self.headings = outline::headings(&source);
        } else {
            self.projection.clear();
            self.headings.clear();
        }
        self.filter_outline();
        self.row_heights[Pane::Editor.index()].clear();
    }
}
