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

#[cfg(test)]
mod tests {
    use super::marker_math::{arrow_pixel, ARROW_TIP};

    /// Where the shaft's metal meets its right-hand ink edge on stage row
    /// `y` (from the tip) of the arrow painted `scale` times as large: the
    /// first pixel right of the shaft's middle darker than `threshold`.
    fn ink_from(y: f32, scale: f32) -> i32 {
        let threshold = 140.0;
        (0..40 * scale as i32).find(|&x| {
            let p = arrow_pixel(x as f32, y, scale, [232.0, 183.0, 87.0, 255.0]);
            p[3] > 0.0 && p[0] + p[1] + p[2] < threshold
        }).expect("the shaft's edge")
    }

    #[test]
    fn painted_larger_the_shafts_straight_edge_stays_straight() {
        for scale in [2.0f32, 3.0, 4.0] {
            // The right edge runs from art (23, 5.5) to (22, 23): leaning
            // left by one art pixel. Its ink must follow it, never wave.
            let rows: Vec<i32> = ((8.0 * scale) as i32..(20.0 * scale) as i32)
                .map(|row| ink_from(row as f32 - ARROW_TIP.1 * scale, scale))
                .collect();
            assert!(rows.windows(2).all(|w| w[1] <= w[0]), "at {scale}x the ink edge waves: {rows:?}");
            let lean = rows[0] - rows[rows.len() - 1];
            assert!(lean <= scale.ceil() as i32 + 1, "at {scale}x it leans {lean} pixels: {rows:?}");
        }
    }
}
