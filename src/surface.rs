use crate::app::{Pane, Workspace};
use gpui::*;
use std::sync::Arc;

mod images;
mod layout;
mod render;
mod table;
mod terminal;
mod text;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct SurfaceLayout {
    pub rows: Vec<HitRow>,
    pub text_width: Pixels,
    pub links: Vec<LinkHit>,
}
pub struct LinkHit {
    pub bounds: Bounds<Pixels>,
    pub url: String,
}
pub struct HitRow {
    pub source_row: usize,
    pub origin: Point<Pixels>,
    pub line: ShapedLine,
    pub raw: bool,
    pub wrapped: Option<WrappedLine>,
    pub line_height: Pixels,
    pub height: Pixels,
}
struct DrawText {
    line: ShapedLine,
    wrapped: Option<WrappedLine>,
    origin: Point<Pixels>,
    height: Pixels,
    clip: Option<Bounds<Pixels>>,
    align: TextAlign,
}
#[derive(Default)]
pub struct Prepared {
    layout: SurfaceLayout,
    text: Vec<DrawText>,
    quads: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    images: Vec<(Bounds<Pixels>, Arc<RenderImage>)>,
}
pub struct Surface {
    pub workspace: Entity<Workspace>,
    pub pane: Pane,
}
