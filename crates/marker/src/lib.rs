//! The selection marker: a gilded arrow over the active selection,
//! bouncing, and its ground shadow, painted per pixel at the scale of the
//! stage drawing them (crisp in a view drawn at its zoom) as a tile
//! renderer's painted shapes (`tile_render::Shape::Painted`). Their maths
//! (`marker_math`) is shared with the shaders painting them on the GPU
//! (`marker-shaders`, wired in as the shader module's painter); here it is
//! their CPU oracle ([`PAINTER`]) and paints their 1:1 art ([`sample`]).
#![forbid(unsafe_code)]

#[path = "../../marker-shaders/src/marker_math.rs"]
mod marker_math;

use marker_math::{SHAPE_ARROW, SHAPE_SHADOW};
pub use marker_math::{ARROW_ART, ARROW_TIP, sample};
use tile_render::{PaintedShape, Shape};

/// The arrow's bob (`bob` 64ths of a full-size world pixel) in pixels of
/// its 1:1 art at `size` sixteenths of it: what shapes its shadow.
fn art_bob(size: u32, bob: i32) -> f32 {
    let size = if size == 0 { 1 } else { size };
    bob as f32 / 64.0 * 16.0 / size as f32
}

/// The arrow over the active selection, in `rgba` (its metal's midtone and
/// opacity): `size` in sixteenths of its 1:1 art, its tip `tip` full-size
/// world pixels from the sprite's position, `bob` 64ths of a pixel below
/// that.
pub fn arrow(size: u32, tip: (i32, i32), bob: i32, rgba: [u8; 4]) -> Shape {
    Shape::Painted(PaintedShape {
        shape: SHAPE_ARROW,
        size,
        at: (tip.0 as f32, tip.1 as f32 + bob as f32 / 64.0),
        extent: [-ARROW_TIP.0, -ARROW_TIP.1, ARROW_ART.0 - ARROW_TIP.0, ARROW_ART.1 - ARROW_TIP.1],
        param: art_bob(size, bob),
        rgba,
    })
}

/// The arrow's ground shadow, centred `centre` full-size world pixels from
/// the sprite's position (it stays put), shaped by the arrow's `bob`:
/// wider and darker the lower the arrow is.
pub fn shadow(size: u32, centre: (i32, i32), bob: i32, rgba: [u8; 4]) -> Shape {
    let param = art_bob(size, bob);
    let (hx, hy) = marker_math::shadow_half(param);
    Shape::Painted(PaintedShape { shape: SHAPE_SHADOW, size, at: (centre.0 as f32, centre.1 as f32), extent: [-hx, -hy, hx, hy], param, rgba })
}

/// Whether `shape` is the arrow ([`arrow`]).
pub fn is_arrow(shape: Option<Shape>) -> bool {
    matches!(shape, Some(Shape::Painted(p)) if p.shape == SHAPE_ARROW)
}

/// Whether `shape` is the arrow's shadow ([`shadow`]).
pub fn is_shadow(shape: Option<Shape>) -> bool {
    matches!(shape, Some(Shape::Painted(p)) if p.shape == SHAPE_SHADOW)
}

/// The marker's CPU painter, the oracle of the GPU's.
#[derive(Copy, Clone, Debug, Default)]
pub struct MarkerPainter;

/// See [`MarkerPainter`].
pub const PAINTER: MarkerPainter = MarkerPainter;

impl tile_render::reference::Painter for MarkerPainter {
    fn paint(&self, shape: u32, x: f32, y: f32, scale: f32, param: f32, rgba: [f32; 4]) -> [f32; 4] {
        marker_math::paint(shape, x, y, scale, param, rgba)
    }
}
