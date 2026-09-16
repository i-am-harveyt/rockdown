use super::{
    UiMode, Workspace,
    ui_mode::{WRITER_POPUP_BOTTOM, WRITER_POPUP_TOP},
};
use gpui::{prelude::*, *};

impl Workspace {
    pub(super) fn help_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .when(self.ui_mode == UiMode::Writer, |overlay| {
                overlay
                    .bg(transparent_black())
                    .justify_end()
                    .items_end()
                    .pt(px(WRITER_POPUP_TOP))
                    .pb(px(WRITER_POPUP_BOTTOM))
            })
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
                    .when(self.ui_mode == UiMode::Writer, |sheet| {
                        sheet
                            .max_w(px(560.))
                            .bg(panel.opacity(0.96))
                            .border_color(foreground.opacity(0.2))
                    })
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px_6()
                            .py_4()
                            .when(self.ui_mode == UiMode::Writer, |header| {
                                header.px_4().py_2()
                            })
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
                                            .when(self.ui_mode == UiMode::Writer, |label| {
                                                label.hidden()
                                            })
                                            .child("ROCKDOWN / REFERENCE"),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_size(px(24.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(self.ui_mode == UiMode::Writer, |title| {
                                                title.mt_0().text_size(px(16.))
                                            })
                                            .child(if self.ui_mode == UiMode::Writer {
                                                "Help"
                                            } else {
                                                "Make yourself at home."
                                            }),
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
                                    .when(self.ui_mode == UiMode::Writer, |topic| {
                                        topic.px_2().py_1().text_size(px(11.))
                                    })
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
                            .when(self.ui_mode == UiMode::Writer, |content| {
                                content.px_4().py_3()
                            })
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
                            .when(
                                section.title == "Appearance" && self.ui_mode == UiMode::Dev,
                                |content| {
                                    content.child(
                                        div().flex().gap_2().mb_3().children(
                                            [
                                                ("tab-bar", "Tab bar", self.config.tab_bar_visible),
                                                (
                                                    "status-bar",
                                                    "Status bar",
                                                    self.config.status_bar_visible,
                                                ),
                                            ]
                                            .into_iter()
                                            .map(
                                                |(action, label, visible)| {
                                                    div()
                                                        .id(action)
                                                        .debug_selector(move || action.into())
                                                        .px_3()
                                                        .py_2()
                                                        .rounded_md()
                                                        .cursor_pointer()
                                                        .bg(panel)
                                                        .text_color(if visible {
                                                            accent
                                                        } else {
                                                            muted
                                                        })
                                                        .hover(|style| {
                                                            style.bg(muted.opacity(0.12))
                                                        })
                                                        .child(format!(
                                                            "{label}: {}",
                                                            if visible {
                                                                "Shown"
                                                            } else {
                                                                "Hidden"
                                                            }
                                                        ))
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.run_action(action, window, cx);
                                                                cx.stop_propagation();
                                                            },
                                                        ))
                                                },
                                            ),
                                        ),
                                    )
                                },
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
                            .when(self.ui_mode == UiMode::Writer, |footer| {
                                footer.px_4().py_2()
                            })
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
}

pub(super) struct HelpSection {
    title: &'static str,
    summary: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
    notes: &'static [&'static str],
}

pub(super) const HELP: &[HelpSection] = &[
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
            "Paste PNG/JPEG/GIF/WebP images with Cmd-V or Ctrl-Shift-V, or drop image files into the editor. Untitled documents ask for Save As first; assets are copied beside the document using image_assets_dir (default assets).",
            "Image imports are one undoable edit. Undo removes links, not shared asset files; Save writes the Markdown. Missing local images show a repair hint. No images are uploaded or fetched.",
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
            ("Ctrl/Cmd-Shift-M · :uimode", "Choose Dev or Writer mode"),
            (
                "Ctrl/Cmd-Alt-T · :tabbar",
                "Toggle Dev tabs / Writer Tabs popup",
            ),
            (
                "Ctrl/Cmd-Alt-S · :statusbar",
                "Toggle Dev status bar / Writer floating controls",
            ),
            ("Ctrl/Cmd-Shift-T", "Open the live theme picker"),
            (":theme", "Preview a built-in or file colorscheme"),
            ("↑ / ↓ · j / k", "Preview themes; hover works too"),
            ("Enter / Esc", "Keep a theme / cancel the preview"),
            (":config", "Reload your configuration"),
            ("--config PATH", "Launch with an explicit TOML config"),
        ],
        notes: &[
            "The top-right mode control has the same position and size in both modes. Writer places Tabs beside the bottom-left Vim letter; it opens a vertical document list. Outline / Files / Help float above the bottom-right controls with aligned lower edges. Click outside a floating window to dismiss it.",
            "Writer's page extends behind the floating controls. Hide (or Ctrl/Cmd-Alt-S / :statusbar) closes tools and hides the controls; Show or the same shortcut restores them without resizing the page. Tool shortcuts and command/error feedback remain available while hidden.",
            "Mode choices are session-only. Switching preserves edits and staged Files changes; returning to Dev restores its previous Files and Terminal visibility. Themes remain available through the shortcut or :theme, with no theme button in either mode.",
            "Bar toggles last for this session. Set tab_bar_visible and status_bar_visible in your config for startup; :config restores those settings. Notifications disappear after five seconds (× dismisses sooner); new notifications restart the timer. Command/search input never expires, even when the status bar is hidden.",
            "Rockdown and Paper are built in. Add colorscheme/*.toml beside your config file for other themes; reopen the picker to discover changes. Set [theme] preset to the filename without .toml for startup.",
            "Theme choices are session-only and replace Markdown color overrides, not typography. Current theme or cancel restores custom colors; :config reloads the configured theme and its file.",
            r"Config location: ~/.config/rockdown/config.toml on Unix; %APPDATA%\rockdown\config.toml on Windows. XDG_CONFIG_HOME overrides the base directory.",
            "markdown.colors controls normal, bold, italic, bold_italic, code, link, strikethrough, and quote colors. markdown.h1 through h6 accept font_size, color, and underline. markdown.divider sets color and thickness, also used for heading underlines.",
        ],
    },
];
