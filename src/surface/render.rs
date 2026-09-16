use super::{Prepared, Surface};
use gpui::{prelude::*, *};

impl IntoElement for Surface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        self.prepare(bounds, window, cx)
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepared: &mut Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        let app = self.workspace.read(cx);
        if app.pane == self.pane {
            window.handle_input(
                &app.focus,
                ElementInputHandler::new(bounds, self.workspace.clone()),
                cx,
            );
        }
        let focused = app.focus.is_focused(window);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepared.quads.drain(..) {
                window.paint_quad(quad);
            }
            for text in &prepared.text {
                let painted = if let Some(line) = &text.wrapped {
                    line.paint_background(
                        text.origin,
                        text.height,
                        text.align,
                        text.clip,
                        window,
                        cx,
                    )
                    .and_then(|_| {
                        line.paint(text.origin, text.height, text.align, text.clip, window, cx)
                    })
                } else {
                    text.line
                        .paint_background(text.origin, text.height, window, cx)
                        .and_then(|_| text.line.paint(text.origin, text.height, window, cx))
                };
                if let Err(error) = painted {
                    eprintln!("Text rendering: {error}");
                }
            }
            for (bounds, image) in prepared.images.drain(..) {
                let _ = window.paint_image(bounds, Corners::default(), image, 0, false);
            }
            if focused && let Some(cursor) = prepared.cursor.take() {
                window.paint_quad(cursor);
            }
        });
        self.workspace.update(cx, |app, _| {
            for row in &prepared.layout.rows {
                app.row_heights[self.pane.index()][row.source_row] = f32::from(row.height);
            }
            app.layouts[self.pane.index()] = std::mem::take(&mut prepared.layout);
        });
    }
}
