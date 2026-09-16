use super::*;

#[test]
fn counted_shifts_preserve_ids_clipboard_and_one_undo() {
    let mut buffer = Buffer::new("α\nβ\nγ\nδ\nε\nζ\nlast");
    let original = buffer.lines.clone();
    buffer.set_clipboard(Some("saved".into()), false);
    keys(&mut buffer, &["2", ">", "3", ">"]);
    assert_eq!(buffer.text(), "\tα\n\tβ\n\tγ\n\tδ\n\tε\n\tζ\nlast");
    assert_eq!(buffer.col, 1);
    assert_eq!(
        buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
        original.iter().map(|line| line.id).collect::<Vec<_>>()
    );
    assert!(buffer.take_yank().is_none());
    buffer.undo();
    assert_eq!(buffer.lines, original);
    buffer.redo();
    keys(&mut buffer, &["6", "<", "<"]);
    assert_eq!(buffer.lines, original);
    keys(&mut buffer, &["P"]);
    assert_eq!(buffer.lines[0].text, "savedα");
}

#[test]
fn shifts_cover_motion_rows_and_outdent_only_ascii_indentation() {
    let mut buffer = Buffer::new("    世界\n \tα\n  β\n\u{2003}γ\n\nlast");
    let original = buffer.lines.clone();
    keys(&mut buffer, &["<", "G"]);
    assert_eq!(buffer.text(), "世界\nα\nβ\n\u{2003}γ\n\nlast");
    assert_eq!(buffer.col, 0);
    buffer.undo();
    assert_eq!(buffer.lines, original);
    keys(&mut buffer, &["G", ">", "g", "g"]);
    assert_eq!(
        buffer.text(),
        "\t    世界\n\t \tα\n\t  β\n\t\u{2003}γ\n\n\tlast"
    );
    buffer.undo();
    assert_eq!(buffer.lines, original);
    keys(&mut buffer, &["g", "g", ">", "$"]);
    assert_eq!(buffer.lines[0].text, "\t    世界");
    assert_eq!(buffer.lines[1], original[1]);
}

#[test]
fn both_visual_kinds_shift_reverse_rows_and_visual_count_sets_levels() {
    for visual in ["v", "V"] {
        let mut buffer = Buffer::new("α\n\nβ\noutside");
        let original = buffer.lines.clone();
        keys(&mut buffer, &["3", "G", visual, "2", "k", "2", ">"]);
        assert_eq!(buffer.text(), "\t\tα\n\n\t\tβ\noutside");
        assert_eq!(buffer.mode, Mode::Normal);
        assert_eq!((buffer.row, buffer.col), (0, 2));
        assert!(buffer.take_yank().is_none());
        buffer.undo();
        assert_eq!(buffer.lines, original);
    }
}

#[test]
fn markdown_enter_preserves_tail_and_undo() {
    let mut buffer = Buffer::new("- hello 世界");
    buffer.key("i");
    buffer.col = 8;
    buffer.markdown_enter();
    assert_eq!(buffer.text(), "- hello \n- 世界");
    assert_eq!(buffer.col, 2);
    buffer.key("escape");
    buffer.undo();
    assert_eq!(buffer.text(), "- hello 世界");
    buffer.redo();
    assert_eq!(buffer.text(), "- hello \n- 世界");
}

#[test]
fn empty_task_exit_and_checkbox_toggles_are_undoable() {
    let mut buffer = Buffer::new("> - [ ] ");
    buffer.key("A");
    buffer.markdown_enter();
    assert_eq!(buffer.text(), "> ");
    buffer.key("escape");
    buffer.undo();
    assert_eq!(buffer.text(), "> - [ ] ");
    buffer.key("i");
    assert!(buffer.toggle_markdown_task(0, 4));
    assert_eq!(buffer.text(), "> - [x] ");
    assert_eq!(buffer.mode, Mode::Insert);
    assert!(buffer.take_yank().is_none());
    buffer.insert_text("typed");
    buffer.key("escape");
    buffer.undo();
    assert_eq!(buffer.text(), "> - [x] ");
    buffer.undo();
    assert_eq!(buffer.text(), "> - [ ] ");
    buffer.redo();
    assert_eq!(buffer.text(), "> - [x] ");
    assert!(!buffer.toggle_markdown_task(0, usize::MAX));
    assert!(!buffer.toggle_markdown_task(1, 0));
}

#[test]
fn literal_crlf_paste_is_one_undo_in_normal_insert_and_visual_modes() {
    for mode in [Mode::Normal, Mode::Insert, Mode::Visual, Mode::VisualLine] {
        let mut buffer = Buffer::new("ab");
        let original = buffer.lines.clone();
        match mode {
            Mode::Insert => keys(&mut buffer, &["i"]),
            Mode::Visual => keys(&mut buffer, &["v"]),
            Mode::VisualLine => keys(&mut buffer, &["V"]),
            Mode::Normal => {}
        }
        buffer.insert_text("x\r\ny\rz\r\n");
        keys(&mut buffer, &["escape"]);
        let expected = match mode {
            Mode::Visual => "x\ny\rz\nb",
            Mode::VisualLine => "x\ny\rz",
            Mode::Normal | Mode::Insert => "x\ny\rz\nab",
        };
        assert_eq!(buffer.text(), expected);
        assert!(buffer.dirty());
        buffer.undo();
        assert_eq!(buffer.lines, original);
        assert!(!buffer.dirty());
        buffer.redo();
        assert_eq!(buffer.text(), expected);
    }
}

#[test]
fn clipboard_register_pastes_normalize_crlf_in_both_register_types() {
    for linewise in [false, true] {
        for key in ["p", "P"] {
            let mut buffer = Buffer::new("ab");
            let original = buffer.lines.clone();
            buffer.set_clipboard(Some("x\r\ny\rz\r\n".into()), linewise);
            assert!(buffer.take_yank().is_none());
            keys(&mut buffer, &[key]);
            let expected = match (linewise, key) {
                (true, "p") => "ab\nx\ny\rz",
                (true, _) => "x\ny\rz\nab",
                (false, "p") => "ax\ny\rz\nb",
                (false, _) => "x\ny\rz\nab",
            };
            assert_eq!(buffer.text(), expected);
            assert!(buffer.take_yank().is_none());
            buffer.undo();
            assert_eq!(buffer.lines, original);
            buffer.redo();
            assert_eq!(buffer.text(), expected);
        }
    }
}

#[test]
fn graphemes_are_atomic_for_motion_deletion_and_insert_undo() {
    let mut buffer = Buffer::new("e\u{301}👩‍💻界");
    keys(&mut buffer, &["l", "x"]);
    assert_eq!(buffer.text(), "e\u{301}界");
    assert_eq!(buffer.col, "e\u{301}".len());
    keys(
        &mut buffer,
        &["u", "a", "backspace", "enter", "🦀", "escape"],
    );
    assert_eq!(buffer.text(), "e\u{301}\n🦀界");
    buffer.undo();
    assert_eq!(buffer.text(), "e\u{301}👩‍💻界");
    buffer.redo();
    assert_eq!(buffer.text(), "e\u{301}\n🦀界");
}

#[test]
fn changing_single_character_words_includes_the_character() {
    let mut buffer = Buffer::new("a b c");
    keys(&mut buffer, &["c", "w", "Z", "escape"]);
    assert_eq!(buffer.text(), "Z b c");
    buffer.undo();
    keys(&mut buffer, &["2", "c", "w", "Q", "escape"]);
    assert_eq!(buffer.text(), "Q c");
}
