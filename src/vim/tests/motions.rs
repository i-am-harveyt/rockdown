use super::*;

#[test]
fn half_page_counts_boundaries_and_cancelled_z_prefix() {
    let mut buffer = Buffer::new(
        &(0..40)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    buffer.page_rows = 20;
    keys(&mut buffer, &["ctrl-d"]);
    assert_eq!(buffer.row, 10);
    keys(&mut buffer, &["3", "ctrl-u"]);
    assert_eq!(buffer.row, 7);
    keys(&mut buffer, &["ctrl-u"]);
    assert_eq!(buffer.row, 4);
    keys(&mut buffer, &["ctrl-u", "ctrl-u"]);
    assert_eq!(buffer.row, 0);
    buffer.take_viewport_motion();
    keys(&mut buffer, &["z", "escape"]);
    buffer.key("t");
    assert!(buffer.take_viewport_motion().is_none());
    keys(&mut buffer, &["3", "0", "z", "t"]);
    assert_eq!(buffer.row, 29);
    assert!(matches!(
        buffer.take_viewport_motion(),
        Some(ViewportMotion::Top)
    ));
}

#[test]
fn counts_words_and_empty_boundaries() {
    let mut buffer = Buffer::new("one two, three\n\nlast");
    keys(&mut buffer, &["2", "w"]);
    assert_eq!(buffer.col, 7);
    keys(&mut buffer, &["b"]);
    assert_eq!(buffer.col, 4);
    keys(&mut buffer, &["c", "w", "X", "escape"]);
    assert_eq!(buffer.text(), "one X, three\n\nlast");
    buffer.undo();
    keys(&mut buffer, &["0", "2", "d", "w"]);
    assert_eq!(buffer.text(), ", three\n\nlast");
    keys(
        &mut buffer,
        &["9", "9", "9", "G", "9", "9", "9", "l", "e", "w", "j"],
    );
    assert_eq!((buffer.row, buffer.col), (2, 3));
    keys(&mut buffer, &["9", "9", "9", "d", "d"]);
    assert_eq!(buffer.text(), ", three\n");
    keys(
        &mut buffer,
        &["g", "g", "9", "9", "9", "d", "d", "b", "e", "h", "k", "x"],
    );
    assert_eq!(buffer.text(), "");
    assert_eq!((buffer.row, buffer.col), (0, 0));
}

#[test]
fn vertical_motion_preserves_grapheme_column_across_short_lines() {
    let mut buffer = Buffer::new("αβγ\nx\n👩‍💻界z");
    keys(&mut buffer, &["2", "l", "j"]);
    assert_eq!((buffer.row, buffer.col), (1, 0));
    keys(&mut buffer, &["j"]);
    assert_eq!((buffer.row, buffer.col), (2, "👩‍💻界".len()));
    keys(&mut buffer, &["i", "up", "up"]);
    assert_eq!((buffer.row, buffer.col), (0, "αβ".len()));
    buffer.col = 1;
    keys(&mut buffer, &["delete"]);
    assert_eq!(buffer.lines[0].text, "βγ");
}
