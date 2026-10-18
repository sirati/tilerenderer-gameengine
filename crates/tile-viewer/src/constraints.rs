//! The system pointer held at the divider's walls and moved where the
//! layout puts it, where the window cannot simply move it (Wayland: pointer
//! constraints, `wl_pointer::Constraints`).
//!
//! With the mouse free or using a captured-edge fallback, the pointer is
//! confined to the window less the divider, zoom and active-view walls
//! ([`Layout::pointer_walls`]); the layout is told whether
//! the system holds it there ([`Walls`]). The pointer is moved by locking
//! it with a position hint and letting it go. winit's own grabs (capture,
//! button snapping) constrain the pointer too: a surface takes one
//! constraint at a time, so this lets go of its own before winit grabs, and
//! confines only while winit does not.
//!
//! On Windows, a captured-edge pointer is clipped to the active view or
//! its single unlocked axis with the safe `inproc` wrapper. Zoom holes
//! still use the usual window-pointer warps. X11 and compositors without
//! the protocol use the window's pointer-movement fallback.

use std::sync::Arc;

use winit::window::Window;

use tile_render::Present;

use crate::layout::Layout;
#[cfg(windows)]
#[path = "constraints_windows.rs"]
mod windows;
#[cfg(target_os = "linux")]
use crate::layout::Walls;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Placement {
    Hinted((i32, i32)),
    Cancelled,
}

#[derive(Default)]
pub struct Constraints {
    #[cfg(target_os = "linux")]
    wayland: Option<wl_pointer::Constraints>,
    #[cfg(windows)]
    windows: windows::Clip,
    /// Whether winit holds the pointer (`Window::set_cursor_grab`).
    grabbed: bool,
    /// Where the system pointer is in the window (physical pixels), and
    /// where it was last logged.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pointer: (f64, f64),
    #[cfg(target_os = "linux")]
    seen: Option<(i32, i32)>,
    /// The requested physical point, committed to input state only after
    /// the compositor activates the lock and its hint has been committed.
    #[cfg(target_os = "linux")]
    pending: Option<(i32, i32)>,
}

impl Constraints {
    /// For the viewer's window, once it is made (logs what it can do).
    pub fn attach(&mut self, window: &Arc<Window>) {
        #[cfg(target_os = "linux")]
        match wl_pointer::Constraints::new(window.clone()) {
            Ok(c) => {
                log::info!("viewer: Wayland pointer constraints: the divider holds the pointer, and the pointer is moved, through them");
                self.wayland = Some(c);
            }
            Err(wl_pointer::Unavailable::NotWayland) => {}
            Err(e) => log::info!("viewer: no Wayland pointer constraints ({e}): the pointer is moved as the window can"),
        }
        #[cfg(not(target_os = "linux"))]
        let _ = window;
    }

    /// winit is to hold the pointer (`grab`) or let it go: first this lets
    /// go of its own constraints.
    pub fn winit_grab(&mut self, grab: bool) {
        self.grabbed = grab;
        #[cfg(windows)]
        if grab { self.windows.release(); }
        #[cfg(target_os = "linux")]
        if let (true, Some(c)) = (grab, &mut self.wayland) {
            c.release();
        }
    }

    /// Moves the system pointer to window pixel `p`, if it is this that
    /// moves it (else the window does).
    pub fn move_pointer(&mut self, window: &Window, p: (i32, i32)) -> bool {
        #[cfg(target_os = "linux")]
        if let (false, Some(c)) = (self.grabbed, &mut self.wayland) {
            c.move_pointer(wl_pointer::point_in_surface(p, window.scale_factor()));
            self.pending = c.is_moving().then_some(p);
            return self.pending.is_some();
        }
        let _ = (window, p);
        false
    }

    /// A lock-and-hint move has not finished yet. Keep the pointer hidden
    /// until the compositor has released that lock or the move times out.
    pub fn moving(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(c) = &self.wayland {
            return c.is_moving();
        }
        false
    }

    /// The system pointer moved to window point `at` (physical pixels).
    pub fn pointer_moved(&mut self, at: (f64, f64)) {
        self.pointer = at;
    }

    /// On every turn of the event loop: takes what the compositor said,
    /// confines the pointer to the layout's walls (`present`: how the
    /// layout's canvas shows in the window; `inside`: whether the system
    /// pointer is in the window) and tells the layout.
    pub fn update(&mut self, window: &Window, layout: &mut Layout, present: Option<Present>, inside: bool) -> Option<Placement> {
        let inside = inside && window.has_focus();
        #[cfg(windows)]
        self.windows.update(window, layout.pointer_bounds().filter(|_| inside && !self.grabbed).map(|r| edge_bounds_in_window(present, r)));
        #[cfg(target_os = "linux")]
        if let Some(c) = &mut self.wayland {
            c.dispatch();
            let placement = c.take_move_result().and_then(|result| self.pending.take().map(|target| match result {
                wl_pointer::MoveResult::Hinted(_) => {
                    self.pointer = (target.0 as f64, target.1 as f64);
                    Placement::Hinted(target)
                }
                wl_pointer::MoveResult::Cancelled => Placement::Cancelled,
            }));
            let at = (self.pointer.0.floor() as i32, self.pointer.1.floor() as i32);
            let pointer = inside.then_some(at);
            if pointer != self.seen {
                self.seen = pointer;
                log::trace!("viewer: system pointer at {pointer:?} (window pixels), confined: {}", c.is_confined());
            }
            let Some(walls) = layout.pointer_walls().filter(|_| !self.grabbed) else {
                c.unconfine();
                return placement;
            };
            let scale = window.scale_factor();
            let size = window.inner_size();
            let size = ((size.width as f64 / scale).ceil() as i32, (size.height as f64 / scale).ceil() as i32);
            let holes = surface_walls(present, &walls, layout.pointer_bounds(), size, scale);
            // (Not while the pointer is in a wall, e.g. one that just moved
            // under it: the compositor would put it elsewhere. It passes
            // till it is out.)
            let in_wall = pointer.is_some_and(|p| holes.iter().any(|h| h.contains(wl_pointer::pixel_in_surface(p, scale))));
            if pointer.is_none() || in_wall {
                c.unconfine();
            } else {
                c.confine(size, &holes);
            }
            layout.set_walls(if c.is_confined() { Walls::Confined } else { Walls::Open });
            return placement;
        }
        let _ = (window, layout, present, inside);
        None
    }
}

/// The real Wayland region uses the same divider/zoom holes for ordinary
/// and captured-edge pointers, with any fixed axis narrowed to one
/// compositor pixel at the cursor's hinted position.
#[cfg(target_os = "linux")]
fn surface_walls(present: Option<Present>, walls: &[[i32; 4]], bounds: Option<[i32; 4]>, size: (i32, i32), scale: f64) -> Vec<wl_pointer::Rect> {
    let physical: Vec<_> = walls.iter().map(|&r| in_window(present, r)).collect();
    let mut holes = wl_pointer::holes_in_surface(&physical, scale);
    if let Some(bounds) = bounds {
        let physical = edge_bounds_in_window(present, bounds);
        let mut rect = wl_pointer::holes_in_surface(&[physical], scale)[0];
        let at = wl_pointer::pixel_in_surface((physical[0], physical[1]), scale);
        if bounds[2] == 1 { rect.x = at.0; rect.w = 1; }
        if bounds[3] == 1 { rect.y = at.1; rect.h = 1; }
        holes.extend(outside(size, [rect.x, rect.y, rect.w, rect.h]).into_iter().map(|[x, y, w, h]| wl_pointer::Rect { x, y, w, h }));
    }
    holes
}

/// Complement of a confinement rectangle, including a one-pixel line.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn outside((w, h): (i32, i32), [x, y, rw, rh]: [i32; 4]) -> Vec<[i32; 4]> {
    let (x0, y0) = (x.clamp(0, w), y.clamp(0, h));
    let (x1, y1) = ((x + rw).clamp(x0, w), (y + rh).clamp(y0, h));
    [[0, 0, w, y0], [0, y1, w, h - y1], [0, y0, x0, y1 - y0], [x1, y0, w - x1, y1 - y0]]
        .into_iter().filter(|r| r[2] > 0 && r[3] > 0).collect()
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
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_wayland_regions_confine_single_axes_to_a_line_and_corners_to_the_view() {
        for n in 1..=6 {
            for scale in [1.0, 1.5, 2.0, 2.5] {
                let p = Present { canvas: (800, 600), zoom: (n, 1), at: (0, 0), target: (800 * n, 600 * n) };
                let size = ((p.target.0 as f64 / scale).ceil() as i32, (p.target.1 as f64 / scale).ceil() as i32);
                for bounds in [[400, 100, 1, 400], [100, 300, 600, 1], [400, 100, 400, 500]] {
                    let walls = outside((800, 600), bounds);
                    let holes = surface_walls(Some(p), &walls, Some(bounds), size, scale);
                    let physical = edge_bounds_in_window(Some(p), bounds);
                    let at = wl_pointer::pixel_in_surface((physical[0], physical[1]), scale);
                    let middle = wl_pointer::pixel_in_surface((physical[0] + physical[2] / 2, physical[1] + physical[3] / 2), scale);
                    let blocked = |point| holes.iter().any(|h| h.contains(point));
                    assert!(!blocked(middle), "n{n}, scale{scale}, {bounds:?}, {holes:?}");
                    if bounds[2] == 1 {
                        assert_eq!(middle.0, at.0);
                        for dx in [-3, -2, -1, 0, 1, 2, 3] {
                            assert_eq!(blocked((at.0 + dx, middle.1)), dx != 0, "n{n}, scale{scale}, vertical line");
                        }
                    }
                    if bounds[3] == 1 {
                        assert_eq!(middle.1, at.1);
                        for dy in [-3, -2, -1, 0, 1, 2, 3] {
                            assert_eq!(blocked((middle.0, at.1 + dy)), dy != 0, "n{n}, scale{scale}, horizontal line");
                        }
                    }
                    assert!(blocked((0, 0)));
                }
            }
        }
    }

}
