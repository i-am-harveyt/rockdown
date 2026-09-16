use super::*;

#[test]
fn crlf_buffers_and_replacements_are_clean_and_keep_lone_cr() {
    let mut buffer = Buffer::new("one\r\ntwo\rthree\r\nlast\r");
    assert_eq!(buffer.text(), "one\ntwo\rthree\nlast\r");
    assert!(!buffer.dirty());
    let original = buffer.lines.clone();
    let revision = buffer.revision();
    buffer.set_text("one\r\ntwo\rthree\r\nlast\r");
    assert_eq!(buffer.lines, original);
    assert_eq!(buffer.revision(), revision);
    buffer.set_text("one\r\nnew\r\ntwo\rthree\r\nlast\r");
    assert_eq!(buffer.text(), "one\nnew\ntwo\rthree\nlast\r");
    assert_eq!(buffer.lines[2].id, original[1].id);
    assert_eq!(buffer.lines[3].id, original[2].id);
    assert!(!buffer.dirty());
}

#[test]
fn line_identity_survives_edits_splits_and_undo_but_not_copies() {
    let mut buffer = Buffer::new("one\ntwo\nthree");
    let ids: Vec<_> = buffer.lines.iter().map(|line| line.id).collect();
    keys(&mut buffer, &["i", "X", "enter", "escape"]);
    assert_eq!(buffer.lines[0].id, ids[0]);
    assert_eq!(buffer.lines[2].id, ids[1]);
    let split_id = buffer.lines[1].id;
    buffer.undo();
    assert_eq!(
        buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
        ids
    );
    keys(&mut buffer, &["y", "y", "p"]);
    assert_eq!(buffer.text(), "one\none\ntwo\nthree");
    assert!(!ids.contains(&buffer.lines[1].id));
    assert_ne!(buffer.lines[1].id, split_id);
    keys(&mut buffer, &["d", "d"]);
    assert_eq!(
        buffer.lines.iter().map(|line| line.id).collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn insert_session_is_one_undo_and_saved_state_is_textual() {
    let mut buffer = Buffer::new("abc");
    keys(&mut buffer, &["A", "x", "y", "left", "z", "escape"]);
    assert_eq!(buffer.text(), "abcxzy");
    buffer.mark_saved();
    keys(&mut buffer, &["0", "l"]);
    assert!(!buffer.dirty());
    buffer.undo();
    assert_eq!(buffer.text(), "abc");
    assert!(buffer.dirty());
    keys(&mut buffer, &["ctrl-r"]);
    assert!(!buffer.dirty());
    buffer.undo();
    keys(&mut buffer, &["x", "ctrl-r"]);
    assert_eq!(buffer.text(), "bc");
}

#[test]
fn set_text_preserves_shifted_suffix_identities() {
    let mut buffer = Buffer::new("a\nb\nc");
    let b = buffer.lines[1].id;
    let c = buffer.lines[2].id;
    buffer.set_text("a\nnew\nb\nc");
    assert_eq!(buffer.lines[2].id, b);
    assert_eq!(buffer.lines[3].id, c);
    assert!(!buffer.dirty());
}

#[test]
fn revision_invalidates_content_not_cursor_or_saved_state() {
    let mut buffer = Buffer::new("abc");
    let initial = buffer.revision();
    keys(&mut buffer, &["l", "v", "escape", "i", "escape"]);
    buffer.mark_saved();
    assert_eq!(buffer.revision(), initial);
    keys(&mut buffer, &["i", "X"]);
    let inserted = buffer.revision();
    assert!(inserted > initial);
    keys(&mut buffer, &["escape"]);
    assert_eq!(buffer.revision(), inserted);
    buffer.undo();
    let undone = buffer.revision();
    assert!(undone > inserted);
    buffer.redo();
    assert!(buffer.revision() > undone);
    let redone = buffer.revision();
    buffer.set_text("Xabc");
    assert_eq!(buffer.revision(), redone);
    buffer.set_text("replacement");
    assert!(buffer.revision() > redone);
}

#[test]
fn undo_stack_is_bounded() {
    let mut buffer = Buffer::new("start");
    for _ in 0..150 {
        keys(&mut buffer, &["o", "x", "escape"]);
    }
    assert_eq!(buffer.undo_stack.len(), MAX_UNDO_DEPTH);
    for _ in 0..MAX_UNDO_DEPTH {
        buffer.undo();
    }
    assert_eq!(buffer.undo_stack.len(), 0);
    assert_eq!(buffer.redo_stack.len(), MAX_UNDO_DEPTH);
}
