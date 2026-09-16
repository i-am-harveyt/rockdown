use super::*;

#[test]
fn visual_line_selection_is_full_rows_and_toggles_keep_anchor() {
    let mut buffer = Buffer::new("αβ\n\n世界\noutside");
    keys(&mut buffer, &["3", "G", "l", "V", "2", "k"]);
    assert_eq!(buffer.selected_range(0), Some(0..4));
    assert_eq!(buffer.selected_range(1), Some(0..0));
    assert_eq!(buffer.selected_range(2), Some(0..6));
    assert_eq!(buffer.selected_range(3), None);
    keys(&mut buffer, &["v"]);
    assert_eq!(buffer.mode, Mode::Visual);
    assert_eq!(buffer.selected_range(0), Some(2..4));
    keys(&mut buffer, &["V"]);
    assert_eq!(buffer.mode, Mode::VisualLine);
    assert_eq!(buffer.selected_range(0), Some(0..4));
    keys(&mut buffer, &["V"]);
    assert_eq!(buffer.mode, Mode::Normal);
    assert_eq!(buffer.selected_range(0), None);
    keys(&mut buffer, &["3", "V"]);
    assert_eq!(buffer.selected_range(2), Some(0..6));
    keys(&mut buffer, &["y"]);
    assert_eq!(buffer.take_yank(), Some(("αβ\n\n世界\n".into(), true)));
    assert_eq!((buffer.row, buffer.col), (0, 0));
}

#[test]
fn visual_line_delete_empty_and_final_rows_does_not_join_neighbors() {
    let mut buffer = Buffer::new("before\n\nlast");
    let original = buffer.lines.clone();
    keys(&mut buffer, &["G", "V", "k", "d"]);
    assert_eq!(buffer.text(), "before");
    assert_eq!(buffer.take_yank(), Some(("\nlast\n".into(), true)));
    buffer.undo();
    assert_eq!(buffer.lines, original);
    keys(&mut buffer, &["g", "g", "V", "G", "d"]);
    assert_eq!(buffer.text(), "");
    assert_eq!((buffer.row, buffer.col), (0, 0));
    assert_eq!(buffer.take_yank(), Some(("before\n\nlast\n".into(), true)));
    keys(&mut buffer, &["V", "y"]);
    assert_eq!(buffer.take_yank(), Some(("\n".into(), true)));
    buffer.undo();
    assert_eq!(buffer.lines, original);
}

#[test]
fn visual_line_change_including_empty_eof_is_one_insert_transaction() {
    for text in ["first\n\nlast", "first\n"] {
        let mut buffer = Buffer::new(text);
        let original = buffer.lines.clone();
        keys(&mut buffer, &["V", "j", "c"]);
        assert_eq!(buffer.mode, Mode::Insert);
        assert_eq!(buffer.take_yank(), Some(("first\n\n".into(), true)));
        buffer.insert_text("replacement");
        keys(&mut buffer, &["escape"]);
        assert_eq!(
            buffer.text(),
            if text.ends_with("last") {
                "replacement\nlast"
            } else {
                "replacement"
            }
        );
        assert_eq!(buffer.lines[0].id, original[0].id);
        buffer.undo();
        assert_eq!(buffer.lines, original);
        buffer.redo();
        assert_eq!(buffer.lines[0].text, "replacement");
    }
}

#[test]
fn visual_line_literal_paste_keeps_neighbors_and_register_at_eof() {
    for (motion, expected) in [("j", "before\nx\n\ny\nlast"), ("G", "before\n\nx\n\ny")] {
        let mut buffer = Buffer::new("before\n\nlast");
        let original = buffer.lines.clone();
        buffer.set_clipboard(Some("saved".into()), false);
        keys(&mut buffer, &[motion, "V"]);
        buffer.insert_text("x\r\n\r\ny\r\n");
        assert_eq!(buffer.text(), expected);
        assert_eq!(buffer.mode, Mode::Normal);
        assert!(buffer.take_yank().is_none());
        buffer.undo();
        assert_eq!(buffer.lines, original);
        keys(&mut buffer, &["g", "g", "P"]);
        assert_eq!(buffer.lines[0].text, "savedbefore");
    }
}

#[test]
fn visual_line_register_replacement_uses_boundaries_and_exports_deleted_rows() {
    for linewise in [false, true] {
        for paste in ["p", "P"] {
            let mut buffer = Buffer::new("before\n\nlast");
            let original = buffer.lines.clone();
            buffer.set_clipboard(Some("x\r\ny\r\n".into()), linewise);
            keys(&mut buffer, &["j", "V", paste]);
            assert_eq!(buffer.text(), "before\nx\ny\nlast");
            assert_eq!(buffer.take_yank(), Some(("\n".into(), true)));
            buffer.undo();
            assert_eq!(buffer.lines, original);
        }
    }
    let mut buffer = Buffer::new("only");
    buffer.set_clipboard(Some("x\n".into()), true);
    keys(&mut buffer, &["V", "2", "p"]);
    assert_eq!(buffer.text(), "x\nx");
    assert_eq!(buffer.take_yank(), Some(("only\n".into(), true)));
    buffer.undo();
    assert_eq!(buffer.text(), "only");
}

#[test]
fn character_visual_paste_keeps_character_and_line_register_semantics() {
    let mut buffer = Buffer::new("a世界z");
    keys(&mut buffer, &["l", "v", "l", "y"]);
    assert_eq!(buffer.take_yank(), Some(("世界".into(), false)));
    buffer.set_clipboard(Some("x\ny\n".into()), true);
    keys(&mut buffer, &["v", "l", "p"]);
    assert_eq!(buffer.text(), "a\nx\ny\nz");
    assert_eq!(buffer.take_yank(), Some(("世界".into(), false)));
    buffer.undo();
    assert_eq!(buffer.text(), "a世界z");
    buffer.set_clipboard(Some("界".into()), false);
    keys(&mut buffer, &["v", "P"]);
    assert_eq!(buffer.text(), "a世界z");
    assert_eq!(buffer.take_yank(), Some(("界".into(), false)));
}

#[test]
fn visual_is_inclusive_and_reversed_multiline_selection_is_safe() {
    let mut buffer = Buffer::new("αβ\n👩‍💻z");
    keys(&mut buffer, &["G", "l", "v", "g", "g"]);
    assert_eq!(buffer.selected_range(0), Some(0.."αβ".len()));
    assert_eq!(buffer.selected_range(1), Some(0.."👩‍💻z".len()));
    keys(&mut buffer, &["d"]);
    assert_eq!(buffer.text(), "");
    buffer.undo();
    assert_eq!(buffer.text(), "αβ\n👩‍💻z");
    keys(&mut buffer, &["g", "g", "v", "c", "Q", "escape"]);
    assert_eq!(buffer.text(), "Qβ\n👩‍💻z");
}

#[test]
fn mouse_word_selection_preserves_unicode_graphemes_and_undo() {
    let mut buffer = Buffer::new("hello café 👩‍💻 world");
    buffer.key("i");
    buffer.col = "hello ca".len();
    buffer.select_word_with_mouse();
    assert_eq!(buffer.selected_range(0), Some(6..11));
    buffer.key("d");
    assert_eq!(buffer.text(), "hello  👩‍💻 world");
    buffer.key("u");
    assert_eq!(buffer.text(), "hello café 👩‍💻 world");
    buffer.select_with_mouse((0, 12), (0, 13));
    assert_eq!(buffer.selected_range(0), Some(12..23));
}
