//! The divider between A and B: a band of frosted glass across the gap
//! (the divider's thickness, `Layout::dividers`), in every map movement
//! mode. A child of `layout`.
//!
//! Each pane's picture goes on under its half of the gap (one camera a
//! half, [`Layout::gap_halves`]; a half whose pane shows no world, a
//! screen fitted there, is a block of the background instead). Over the
//! stretches where the divider is shown, translucent strips one pixel
//! across ([`Layout::glass_strips`]): a light edge on A's side, a parchment
//! frost in the middle, a soft shadow on B's side. The world shows through
//! dimmed and tinted, the band reads as one piece over black (unexplored)
//! and bright terrain alike, and it costs two small cameras and a few
//! sprites (no extra pass, no shader of its own).
//!
//! The glass is shown where the divider is drawn, which is where it holds
//! the pointer back (`Layout::dividers`, the one source of both): its
//! closed stretches with edge scrolling (the open middle third, the
//! controls' rows and where the interface reaches across show the
//! pictures alone), all of it while the mouse is captured, and all of it
//! with Drag map movement, where the pointer passes it everywhere (nothing
//! there to show as open).
//!
//! With glass dividers there are no halves and no frost: one view's picture
//! goes on under the whole of each divider (`Dividers::under`), and the
//! glass bars bend that one picture.

use tile_render::Camera;
use super::{world_at_floor, Axis, DividerKind, Dividers, Hit, Layout, Side};

/// The shade of one column (side by side) or row (stacked) of the glass.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Shade {
    /// A's edge: a light highlight.
    Light,
    /// The frosted middle.
    Frost,
    /// B's edge: a soft shadow.
    Shadow,
}

impl Shade {
    /// The shade at `k` pixels across a band `width` wide.
    pub fn at(k: i32, width: i32) -> Shade {
        if k == 0 && width > 1 {
            Shade::Light
        } else if k == width - 1 && width > 2 {
            Shade::Shadow
        } else {
            Shade::Frost
        }
    }
}

impl Layout {
    /// What lies under the gap, A's half then B's: its rectangle and the
    /// camera drawing it, the pane's picture going on into it (a block of
    /// the background where the pane shows no world).
    pub(super) fn gap_halves(&self) -> Vec<([i32; 4], Camera)> {
        let (Some(((g0, g1), (s0, s1))), Some(split)) = (self.gap(), self.split) else { return vec![] };
        let half = (g1 - g0) / 2;
        let mut out = Vec::new();
        for (side, from, to) in [(Side::A, g0, g0 + half), (Side::B, g0 + half, g1)] {
            if to <= from || s1 <= s0 {
                continue;
            }
            let rect = match split.axis {
                Axis::Vertical => [from, s0, to - from, s1 - s0],
                Axis::Horizontal => [s0, from, s1 - s0, to - from],
            };
            out.push((rect, self.picture_over(Some(side), rect)));
        }
        out
    }

    /// What lies under the glass dividers (`Dividers::under`, which view's
    /// picture goes on where): a camera for each stretch.
    pub(super) fn under_glass_cameras(&self, dividers: &Dividers) -> Vec<Camera> {
        dividers.under.iter().filter(|u| u.rect[2] > 0 && u.rect[3] > 0).map(|u| self.picture_over(u.side, u.rect)).collect()
    }

    /// A camera drawing `side`'s picture going on over window rectangle
    /// `rect`, as its view draws it (a block of the background where the
    /// side shows no world, or none is given).
    fn picture_over(&self, side: Option<Side>, rect: [i32; 4]) -> Camera {
        let fitted = self.fit.map(|f| f.hit);
        let hit = side.map(|s| if s == Side::A { Hit::Main } else { Hit::Pane }).filter(|h| Some(*h) != fitted);
        let at = (rect[0], rect[1]);
        let size = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        match hit.and_then(|hit| self.shown.iter().find(|(h, c)| *h == hit && !c.stacked)) {
            Some((_, c)) => {
                let (x, y) = world_at_floor(c, at);
                Camera { x, y, origin: at, viewport: size, ..*c }
            }
            // (A zoomed view of nothing: the background colour.)
            None => Camera { zoom: (2, 1), origin: at, cells: false, ..Camera::new(size) },
        }
    }

    /// The glass's strips: each a window rectangle one pixel across the
    /// gap, whether the split is side by side, and its shade; over the
    /// split's drawn segments (`Layout::dividers`).
    pub(super) fn glass_strips(&self) -> Vec<([i32; 4], bool, Shade)> {
        let mut out = Vec::new();
        for s in self.dividers().segments.iter().filter(|s| s.band.kind == DividerKind::Split) {
            let (r, (g0, g1)) = (s.rect(), s.band.across);
            let width = g1 - g0;
            for k in 0..width {
                let strip = if s.band.vertical { [g0 + k, r[1], 1, r[3]] } else { [r[0], g0 + k, r[2], 1] };
                out.push((strip, s.band.vertical, Shade::at(k, width)));
            }
        }
        out
    }
}
