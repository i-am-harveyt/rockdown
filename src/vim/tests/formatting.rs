use super::*;
use crate::vim::InlineFormat;

#[test]
fn formatting_retains_reverse_grapheme_selection_and_clipboard() {
    let mut buffer = Buffer::new("a👩‍💻e\u{301}z");
    let original = buffer.lines.clone();
    buffer.set_clipboard(Some("saved".into()), false);
    buffer.select_with_mouse((0, "a👩‍💻".len()), (0, 1));
    assert!(buffer.toggle_inline_format(InlineFormat::Bold));
    assert_eq!(buffer.text(), "a**👩‍💻e\u{301}**z");
    assert_eq!(buffer.selected_range(0), Some(3.."a**👩‍💻e\u{301}".len()));
    assert_eq!((buffer.row, buffer.col), (0, 3));
    assert_eq!(buffer.mode, Mode::Visual);
    assert!(buffer.take_yank().is_none());
    assert!(buffer.toggle_inline_format(InlineFormat::Bold));
    assert_eq!(buffer.lines, original);
    assert_eq!(buffer.selected_range(0), Some(1.."a👩‍💻e\u{301}".len()));
    assert_eq!((buffer.row, buffer.col), (0, 1));
    buffer.key("escape");
    buffer.key("P");
    assert_eq!(buffer.text(), "asaved👩‍💻e\u{301}z");
}

#[test]
fn multiline_formatting_trims_whitespace_and_is_one_undo_transaction() {
    let mut buffer = Buffer::new("before one  \n\t \n  two after");
    let original = buffer.lines.clone();
    buffer.select_with_mouse((2, "  tw".len()), (0, "before ".len()));
    assert!(buffer.toggle_inline_format(InlineFormat::Underline));
    assert_eq!(
        buffer.text(),
        "before <u>one</u>  \n\t \n  <u>two</u> after"
    );
    assert_eq!(buffer.col, "before <u>".len());
    assert_eq!(
        buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
        original.iter().map(|line| line.id).collect::<Vec<_>>()
    );
    buffer.undo();
    assert_eq!(buffer.lines, original);
    buffer.redo();
    assert_eq!(
        buffer.text(),
        "before <u>one</u>  \n\t \n  <u>two</u> after"
    );
    buffer.undo();
    buffer.undo();
    assert_eq!(buffer.lines, original);
}

#[test]
fn linewise_formatting_preserves_mode_and_toggles_each_nonblank_line() {
    let mut buffer = Buffer::new("  one  \n\n\t two\t\nlast");
    let original = buffer.lines.clone();
    keys(&mut buffer, &["3", "G", "V", "k", "k"]);
    assert!(buffer.toggle_inline_format(InlineFormat::Strikethrough));
    assert_eq!(buffer.text(), "  ~~one~~  \n\n\t ~~two~~\t\nlast");
    assert_eq!(buffer.mode, Mode::VisualLine);
    assert_eq!(buffer.row, 0);
    assert!(buffer.toggle_inline_format(InlineFormat::Strikethrough));
    assert_eq!(buffer.lines, original);
    assert_eq!(buffer.mode, Mode::VisualLine);
    assert_eq!(buffer.row, 0);
}

#[test]
fn italic_never_removes_half_of_bold_delimiters() {
    for (anchor, head) in [((0, 2), (0, 5)), ((0, 0), (0, 7)), ((0, 1), (0, 6))] {
        let mut buffer = Buffer::new("**word**");
        buffer.select_with_mouse(anchor, head);
        assert!(buffer.toggle_inline_format(InlineFormat::Italic));
        assert_eq!(buffer.text(), "***word***");
        assert!(buffer.toggle_inline_format(InlineFormat::Italic));
        assert_eq!(buffer.text(), "**word**");
    }
}

#[test]
fn nested_emphasis_removes_only_requested_style() {
    for (format, expected) in [
        (InlineFormat::Bold, "*word*"),
        (InlineFormat::Italic, "**word**"),
    ] {
        for (anchor, head) in [((0, 3), (0, 6)), ((0, 0), (0, 9))] {
            let mut buffer = Buffer::new("***word***");
            buffer.select_with_mouse(anchor, head);
            assert!(buffer.toggle_inline_format(format));
            assert_eq!(buffer.text(), expected);
            assert!(buffer.toggle_inline_format(format));
            assert_eq!(buffer.text(), "***word***");
        }
    }
}

#[test]
fn inline_code_uses_safe_tick_runs_and_reverses_padding() {
    for (content, expected) in [
        ("a`b``c", "```a`b``c```"),
        ("`edge", "`` `edge ``"),
        ("edge`", "`` edge` ``"),
        ("`", "`` ` ``"),
        ("界", "`界`"),
    ] {
        let original = format!("α{content}ω");
        let mut buffer = Buffer::new(&original);
        let end = super::super::previous_boundary(&original, "α".len() + content.len());
        buffer.select_with_mouse((0, "α".len()), (0, end));
        assert!(buffer.toggle_inline_format(InlineFormat::Code));
        assert_eq!(buffer.text(), format!("α{expected}ω"));
        let selection = buffer.selected_range(0).unwrap();
        assert_eq!(&buffer.lines[0].text[selection], content);
        assert!(buffer.toggle_inline_format(InlineFormat::Code));
        assert_eq!(buffer.text(), original);
    }
}

#[test]
fn selected_code_wrapper_removes_delimiters_and_commonmark_padding() {
    let mut buffer = Buffer::new("`` `edge` ``");
    keys(&mut buffer, &["V"]);
    assert!(buffer.toggle_inline_format(InlineFormat::Code));
    assert_eq!(buffer.text(), "`edge`");
    buffer.undo();
    assert_eq!(buffer.text(), "`` `edge` ``");
}

#[test]
fn formatting_noops_do_not_invalidate_content_or_clear_redo() {
    let mut buffer = Buffer::new("word\n \t");
    buffer.select_with_mouse((0, 0), (0, 3));
    assert!(buffer.toggle_inline_format(InlineFormat::Bold));
    buffer.undo();
    let revision = buffer.revision();
    assert!(!buffer.toggle_inline_format(InlineFormat::Bold));
    buffer.select_with_mouse((1, 0), (1, 1));
    assert!(!buffer.toggle_inline_format(InlineFormat::Underline));
    assert_eq!(buffer.revision(), revision);
    buffer.redo();
    assert_eq!(buffer.text(), "**word**\n \t");
    let mut empty = Buffer::new("");
    empty.key("v");
    assert!(!empty.toggle_inline_format(InlineFormat::Code));
    assert_eq!(empty.revision(), 0);
}
