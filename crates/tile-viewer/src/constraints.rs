//! Layout adapter for the shared display pointer API. Layout rectangles
//! are converted from canvas pixels to physical window pixels here; capture,
//! pointer warps and display-specific confinement live in pointer-control.

use std::sync::Arc;
use winit::window::Window;
use tile_render::Present;
use crate::layout::{Layout, Walls};

pub use pointer_control::{Move, Placement};

#[derive(Default)]
pub struct Constraints {
    control: pointer_control::Controller,
    pointer: (f64, f64),
    confined: bool,
}

impl Constraints {
    pub fn attach(&mut self, window: &Arc<Window>) { self.control.attach(window.clone()); }

    /// Capture owns the pointer instead of layout regions. Returns whether
    /// the display can keep it locked, rather than merely inside the window.
    pub fn grab(&mut self, on: bool) -> bool {
        match self.control.grab(on) {
            Ok(result) => result == pointer_control::Grab::Locked,
            Err(error) => {
                log::debug!("viewer: pointer capture: {error}");
                false
            }
        }
    }

    pub fn move_pointer(&mut self, at: (i32, i32)) -> Result<Move, pointer_control::Error> { self.control.move_pointer(at) }
    pub fn moving(&self) -> bool { self.control.moving() }
    pub fn pointer_moved(&mut self, at: (f64, f64)) { self.pointer = at; }

    pub fn update(&mut self, window: &Window, layout: &mut Layout, present: Option<Present>, inside: bool) -> Option<Placement> {
        let region = layout.pointer_walls().map(|walls| {
            let size = window.inner_size();
            pointer_control::Region {
                bounds: layout.pointer_bounds().map(|r| edge_bounds_in_window(present, r)).unwrap_or([0, 0, size.width as i32, size.height as i32]),
                exclusions: walls.into_iter().map(|r| in_window(present, r)).collect(),
            }
        });
        let update = self.control.update(region.as_ref(), self.pointer, inside && window.has_focus());
        if update.confined {
            layout.set_walls(Walls::Confined);
        } else if self.confined {
            layout.set_walls(Walls::Warp);
        }
        self.confined = update.confined;
        if let Some(Placement::Hinted(at)) = update.placement { self.pointer = (at.0 as f64, at.1 as f64); }
        update.placement
    }
}

/// An edge region uses the centre of a canvas pixel for a locked axis,
/// just like the pointer position hint, and confines it to one physical
/// pixel instead of widening it with the rendered picture.
fn edge_bounds_in_window(present: Option<Present>, r: [i32; 4]) -> [i32; 4] {
    let mut out = in_window(present, r);
    if r[2] == 1 { out[0] += out[2] / 2; out[2] = 1; }
    if r[3] == 1 { out[1] += out[3] / 2; out[3] = 1; }
    out
}

/// Canvas rectangle `r` (the layout's pixels) in the window (physical
/// pixels): the window pixels wholly inside it, at least one per nonempty
/// span so a thin divider still holds the pointer.
fn in_window(present: Option<Present>, [x, y, w, h]: [i32; 4]) -> [i32; 4] {
    let Some(p) = present else { return [x, y, w, h] };
    let (n, d) = (p.zoom.0.max(1) as i64, p.zoom.1.max(1) as i64);
    let span = |v: i32, len: i32, at: i32| {
        let a = -(-(v as i64 * n)).div_euclid(d);
        let b = ((v as i64 + len as i64) * n).div_euclid(d);
        // A divider thinner than a window pixel must still block motion.
        (at + a as i32, if len > 0 { (b - a).max(1) as i32 } else { 0 })
    };
    let ((x, w), (y, h)) = (span(x, w, p.at.0), span(y, h, p.at.1));
    [x, y, w, h]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walls_go_to_the_window_pixels_the_canvas_shows_them_on() {
        let wall = [960, 0, 4, 360];
        assert_eq!(in_window(None, wall), wall);
        assert_eq!(in_window(Some(Present::unscaled((1920, 1080))), wall), wall);
        // Canvas 960×540 shown 2× in a 1921×1081 window (a pixel left over,
        // the picture at 0, 0).
        let p = Present { canvas: (960, 540), zoom: (2, 1), at: (0, 0), target: (1921, 1081) };
        assert_eq!(in_window(Some(p), [480, 0, 2, 180]), [960, 0, 4, 360]);
        let p = Present { at: (1, 2), ..p };
        assert_eq!(in_window(Some(p), [480, 10, 2, 180]), [961, 22, 4, 360]);
        // At 3/2 only the window pixels wholly inside: 481..483 covers
        // 721.5..724.5, window pixels 722 and 723.
        let p = Present { canvas: (1280, 720), zoom: (3, 2), at: (0, 0), target: (1920, 1080) };
        assert_eq!(in_window(Some(p), [481, 1, 2, 3]), [722, 2, 2, 4]);
        // A canvas scaled below one window pixel per divider width still
        // has a physical wall. Negative origins round towards the inside.
        let p = Present { zoom: (1, 8), at: (0, 0), ..p };
        assert_eq!(in_window(Some(p), [481, -9, 2, 17]), [61, -1, 1, 2]);
        assert_eq!(in_window(Some(p), [481, 0, 0, 3]), [61, 0, 0, 1]);
    }

    #[test]
    fn captured_edge_lines_stay_one_physical_pixel_at_every_render_scale() {
        for n in 1..=6 {
            let p = Present { canvas: (800, 600), zoom: (n, 1), at: (7, 11), target: (800 * n + 14, 600 * n + 22) };
            assert_eq!(edge_bounds_in_window(Some(p), [400, 100, 1, 400]), [7 + 400 * n as i32 + n as i32 / 2, 11 + 100 * n as i32, 1, 400 * n as i32]);
            assert_eq!(edge_bounds_in_window(Some(p), [100, 300, 600, 1]), [7 + 100 * n as i32, 11 + 300 * n as i32 + n as i32 / 2, 600 * n as i32, 1]);
            assert_eq!(edge_bounds_in_window(Some(p), [400, 0, 400, 600]), [7 + 400 * n as i32, 11, 400 * n as i32, 600 * n as i32]);
        }
    }

}
