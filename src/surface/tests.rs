use super::{Pane, Surface, Workspace};
use crate::{config::Config, document::Document, explorer::Explorer};
use gpui::{Background, Bounds, Element, TestAppContext, point, px, size};

#[gpui::test]
fn image_previews_center_in_writing_column(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    image::RgbaImage::new(80, 40)
        .save(directory.path().join("photo.png"))
        .unwrap();
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled("outside\n![Image](photo.png)\n![Sized](photo.png \"120px\")"),
            Explorer::open(directory.path()).unwrap(),
            window,
            cx,
        )
    });
    window.run_until_parked();
    window.update(|window, cx| {
        let mut surface = Surface {
            workspace: view.clone(),
            pane: Pane::Editor,
        };
        for width in [300., 1000.] {
            let prepared = surface.prepaint(
                None,
                None,
                Bounds::new(point(px(40.), px(0.)), size(px(width), px(2000.))),
                &mut (),
                window,
                cx,
            );
            assert_eq!(prepared.images.len(), 2);
            for (bounds, _) in &prepared.images {
                let row = &prepared.layout.rows[1];
                let left = bounds.left() - row.origin.x;
                let right = row.origin.x + prepared.layout.text_width - bounds.right();
                assert!((f32::from(left - right)).abs() < 0.01);
                assert!(left >= px(0.));
            }
        }
    });
}

#[gpui::test]
fn code_block_background_covers_wrapped_rows(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let text = format!(
        "outside\n```rust\n{}\nshort\n```\nafter",
        "let message = \"hello world\"; ".repeat(12)
    );
    let (view, window) = cx.add_window_view(|window, cx| {
        Workspace::new(
            Config::default(),
            None,
            Document::untitled(&text),
            Explorer::open(directory.path()).unwrap(),
            window,
            cx,
        )
    });
    window.run_until_parked();
    window.update(|window, cx| {
        let mut surface = Surface {
            workspace: view.clone(),
            pane: Pane::Editor,
        };
        for width in [300., 600.] {
            let prepared = surface.prepaint(
                None,
                None,
                Bounds::new(point(px(0.), px(0.)), size(px(width), px(2000.))),
                &mut (),
                window,
                cx,
            );
            let panel: Background = view.read(cx).color(&view.read(cx).config.theme.panel).into();
            for source_row in [2, 3] {
                let row = prepared
                    .layout
                    .rows
                    .iter()
                    .find(|row| row.source_row == source_row)
                    .unwrap();
                assert!(!row.raw);
                if source_row == 2 {
                    assert!(row.height > row.line_height);
                }
                for visual_row in 0..((row.height / row.line_height) as usize) {
                    let sample = row.origin
                        + point(px(1.), row.line_height * (visual_row as f32 + 0.5));
                    assert!(
                        prepared.quads.iter().any(|quad| {
                            quad.background == panel && quad.bounds.contains(&sample)
                        }),
                        "code row {source_row}, visual row {visual_row} has no panel background at width {width}"
                    );
                }
            }
        }
    });
}
