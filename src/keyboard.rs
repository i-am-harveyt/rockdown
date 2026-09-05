use gpui::Keystroke;

/// Native macOS events attach key_char="\n" to Return and "\t" to Tab.
/// Those are commands, not committed IME text; keep their semantic key names.
pub(crate) fn key(stroke: &Keystroke) -> &str {
    named_key(&stroke.key).unwrap_or_else(|| {
        stroke
            .key_char
            .as_deref()
            .filter(|text| !text.chars().any(char::is_control))
            .unwrap_or(&stroke.key)
    })
}

pub(crate) fn text(stroke: &Keystroke) -> Option<&str> {
    if stroke.modifiers.control
        || stroke.modifiers.platform
        || stroke.modifiers.alt
        || named_key(&stroke.key).is_some()
    {
        return None;
    }
    stroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

fn named_key(key: &str) -> Option<&str> {
    match key {
        "enter" | "return" => Some("enter"),
        "tab" | "escape" | "backspace" | "delete" | "left" | "right" | "up" | "down" | "home"
        | "end" | "pageup" | "pagedown" => Some(key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rockdown::{terminal::key_bytes, vim::Buffer};

    #[test]
    fn native_return_splits_editor_line_and_encodes_terminal_submit() {
        let mut buffer = Buffer::new("beforeafter");
        buffer.key("i");
        for _ in 0..6 {
            buffer.key("right");
        }
        let event = Keystroke {
            key: "enter".into(),
            key_char: Some("\n".into()),
            ..Default::default()
        };
        assert!(text(&event).is_none());
        buffer.key(key(&event));
        assert_eq!(buffer.text(), "before\nafter");
        assert_eq!((buffer.row, buffer.col), (1, 0));
        assert_eq!(
            key_bytes(key(&event), false, false, false),
            Some(b"\r".to_vec())
        );
    }

    #[test]
    fn named_controls_do_not_mask_composed_text_or_tab_commands() {
        let tab = Keystroke {
            key: "tab".into(),
            key_char: Some("\t".into()),
            ..Default::default()
        };
        assert!(text(&tab).is_none());
        assert_eq!(key(&tab), "tab");
        let composed = Keystroke {
            key: "a".into(),
            key_char: Some("界".into()),
            ..Default::default()
        };
        assert_eq!(text(&composed), Some("界"));
        let named_return = Keystroke {
            key: "return".into(),
            key_char: Some("\r".into()),
            ..Default::default()
        };
        assert_eq!(key(&named_return), "enter");
        assert!(text(&named_return).is_none());
    }
}
