use super::{ThemePicker, Workspace};
use crate::config::Config;
use gpui::{prelude::*, *};

impl Workspace {
    pub(super) fn open_theme_picker(&mut self, cx: &mut Context<Self>) {
        let presets = match Config::colorschemes(self.config_path.as_deref()) {
            Ok(presets) => presets,
            Err(error) => {
                self.message = format!("{error:#}");
                cx.notify();
                return;
            }
        };
        self.help = false;
        self.dismiss_outline();
        self.theme_picker = Some(ThemePicker {
            original: self.config.theme.clone(),
            original_markdown: self.config.markdown.clone(),
            presets,
            selected: 0,
            scroll: ScrollHandle::new(),
        });
        cx.notify();
    }

    pub(super) fn preview_theme(&mut self, selected: usize, cx: &mut Context<Self>) {
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
        self.config.theme = theme.clone();
        self.config.markdown = picker.original_markdown.clone();
        if selected != 0 {
            self.config
                .theme
                .apply_markdown_colors(&mut self.config.markdown);
        }
        self.refresh_projection();
        cx.notify();
    }

    pub(super) fn finish_theme_picker(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(picker) = self.theme_picker.take() else {
            return;
        };
        if keep {
            if picker.selected != 0 {
                self.message = format!("{} theme · this session only", self.config.theme.name);
            }
        } else {
            self.config.theme = picker.original;
            self.config.markdown = picker.original_markdown;
            self.refresh_projection();
        }
        cx.notify();
    }

    pub(super) fn theme_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .track_scroll(&picker.scroll)
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
                                            &theme.name
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
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_size(px(13.))
                                                    .text_color(foreground)
                                                    .child(name.to_owned()),
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
}
