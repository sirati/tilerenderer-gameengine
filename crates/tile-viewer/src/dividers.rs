//! The dividers' geometry, in one place: the split's divider between A
//! and B, and the zoom strip's divider below the strip. A child of
//! `layout`.
//!
//! [`Layout::dividers`] is the only code that decides where a divider is,
//! how thick it is, where it stops and where the pointer passes it. Every
//! consumer reads that one value:
//!
//! - drawing: the frosted strips over the split's gap and the strip's
//!   plain separator;
//! - the pointer: the walls the system holds it out of
//!   ([`Dividers::walls`], [`Layout::pointer_walls`]), the walls the
//!   layout holds it back at ([`Dividers::holds`], `Layout::constrain`),
//!   the zoom strip's wall;
//! - hit testing: which side of the split a pixel is on
//!   ([`Dividers::side_of`]) and where a press grabs the split's divider
//!   ([`Dividers::handle_at`]);
//! - edge scrolling: the first row below the strip scrolls up
//!   ([`Dividers::strip`]).
//!
//! A divider is a [`Band`] (its full extent, a thickness across and a
//! length along), cut into [`Segment`]s where the pointer passes it. The
//! drawn pixels of a divider are exactly its segments, and a segment with
//! `wall` holds the pointer exactly over its rectangle, so drawing and
//! pointer cannot disagree. (With Drag map movement the split's divider
//! is drawn whole and holds nothing: `wall` is false there, the pointer
//! goes anywhere, the line only separates the pictures.)

use super::{subtract_spans, Axis, Layout, Side, ZoomPlace, GAP};

/// Which divider.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Between A and B.
    Split,
    /// Below the zoom strip.
    Strip,
}

/// How a segment ends (at the low and the high end along it).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// At the window's edge.
    Edge,
    /// Against another divider (a T junction).
    Junction,
    /// In the open: the divider stops there (the pointer passes beyond).
    Free,
}

/// A divider's full extent, canvas pixels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Band {
    pub kind: Kind,
    /// A vertical line (the split side by side): across is x, along is y.
    pub vertical: bool,
    /// From, to (exclusive) across it: its thickness.
    pub across: (i32, i32),
    /// From, to (exclusive) along it.
    pub along: (i32, i32),
}

impl Band {
    pub fn thickness(&self) -> i32 {
        self.across.1 - self.across.0
    }

    /// The rectangle (x, y, w, h) of the stretch `along` of it.
    pub fn rect_of(&self, (a0, a1): (i32, i32)) -> [i32; 4] {
        let (c0, c1) = self.across;
        if self.vertical { [c0, a0, c1 - c0, a1 - a0] } else { [a0, c0, a1 - a0, c1 - c0] }
    }

    pub fn rect(&self) -> [i32; 4] {
        self.rect_of(self.along)
    }

    /// Point `p` as (across, along).
    pub fn local(&self, p: (i32, i32)) -> (i32, i32) {
        if self.vertical { p } else { (p.1, p.0) }
    }

    pub fn contains(&self, p: (i32, i32)) -> bool {
        let (c, a) = self.local(p);
        c >= self.across.0 && c < self.across.1 && a >= self.along.0 && a < self.along.1
    }
}

/// A drawn stretch of a divider.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub band: Band,
    /// From, to (exclusive) along the band.
    pub along: (i32, i32),
    pub ends: [End; 2],
    /// It holds the free pointer back over its rectangle (while walls hold
    /// at all: not while captured, dragging, snapping, ...).
    pub wall: bool,
}

impl Segment {
    pub fn rect(&self) -> [i32; 4] {
        self.band.rect_of(self.along)
    }

    pub fn contains(&self, p: (i32, i32)) -> bool {
        let (c, a) = self.band.local(p);
        c >= self.band.across.0 && c < self.band.across.1 && a >= self.along.0 && a < self.along.1
    }

}

/// Every divider of the arrangement (see the module's documentation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dividers {
    /// The split's divider (the gap between A and B).
    pub split: Option<Band>,
    /// The zoom strip's divider.
    pub strip: Option<Band>,
    /// The drawn stretches, the split's first.
    pub segments: Vec<Segment>,
    /// Where a left press grabs the split's divider to move it (window
    /// rectangles): its open stretches a little into each pane.
    pub handles: Vec<[i32; 4]>,
}

/// How far into each pane a press still grabs the split's divider.
const GRAB: i32 = 2;

impl Dividers {
    /// The rectangles the divider walls hold the pointer out of (where
    /// walls hold at all).
    pub fn walls(&self) -> Vec<[i32; 4]> {
        self.segments.iter().filter(|s| s.wall).map(Segment::rect).collect()
    }

    /// Whether a `kind` divider holds the pointer back at `along`.
    pub fn holds(&self, kind: Kind, along: i32) -> bool {
        self.segments.iter().any(|s| s.wall && s.band.kind == kind && along >= s.along.0 && along < s.along.1)
    }

    /// Which side of the split window pixel `p` is on (none: in its gap,
    /// or no split).
    pub fn side_of(&self, p: (i32, i32)) -> Option<Side> {
        let b = self.split?;
        let (c, _) = b.local(p);
        if c < b.across.0 {
            Some(Side::A)
        } else if c >= b.across.1 {
            Some(Side::B)
        } else {
            None
        }
    }

    /// Whether a press at `p` grabs the split's divider.
    pub fn handle_at(&self, p: (i32, i32)) -> bool {
        self.handles.iter().any(|r| super::inside(*r, p))
    }

    /// The stretches along the split's divider where it is not drawn (the
    /// pointer passes there).
    pub fn split_openings(&self) -> Vec<(i32, i32)> {
        let Some(b) = self.split else { return vec![] };
        let drawn: Vec<(i32, i32)> = self.segments.iter().filter(|s| s.band.kind == Kind::Split).map(|s| s.along).collect();
        subtract_spans(b.along, &drawn)
    }

}

/// What is left of `band` with `openings` cut out, its own `ends` where
/// it reaches them (a cut end is free).
fn carve(band: Band, openings: &[(i32, i32)], wall: bool, ends: [End; 2]) -> Vec<Segment> {
    subtract_spans(band.along, openings)
        .into_iter()
        .map(|along| Segment {
            band,
            along,
            ends: [if along.0 == band.along.0 { ends[0] } else { End::Free }, if along.1 == band.along.1 { ends[1] } else { End::Free }],
            wall,
        })
        .collect()
}

impl Layout {
    /// How thick the dividers are (canvas pixels): the separator width in
    /// window pixels, at
    /// least one pixel.
    pub(crate) fn divider_thickness(&self) -> i32 {
        let width = self.separator_width.clamp(1, 16);
        (f64::from(width) / self.window_pixel_scale.max(1.0)).round().max(1.0) as i32
    }

    /// The dividers as they are now (see the module's documentation).
    pub(crate) fn dividers(&self) -> Dividers {
        let (w, h) = (self.window.0 as i32, self.window.1 as i32);
        let mut out = Dividers::default();
        // The strip's divider: right below it, as wide as it.
        let strip = self.strip().filter(|_| self.magnifier_shown && self.world_shown && self.magnifier.is_some_and(|m| matches!(m.place, ZoomPlace::Strip(_))));
        out.strip = strip.map(|(s, _)| Band { kind: Kind::Strip, vertical: false, across: (s[1] + s[3], s[1] + s[3] + self.divider_thickness()), along: (s[0], s[0] + s[2]) });
        out.split = self.gap().zip(self.split).map(|(((g0, g1), along), split)| Band { kind: Kind::Split, vertical: split.axis == Axis::Vertical, across: (g0, g1), along });
        let meets = |band: Band, at: i32, other: Option<Band>| other.is_some_and(|o| o.vertical != band.vertical && (at == o.across.0 || at == o.across.1)
            && { let (c0, c1) = band.across; c0 >= o.along.0 && c1 <= o.along.1 });
        let end = |band: Band, at: i32, other: Option<Band>, extent: i32| if at <= 0 || at >= extent { End::Edge } else if meets(band, at, other) { End::Junction } else { End::Free };
        if let Some(b) = out.split {
            let extent = if b.vertical { h } else { w };
            let ends = [end(b, b.along.0, out.strip, extent), end(b, b.along.1, out.strip, extent)];
            let (s0, s1) = b.along;
            // Where a press grabs it: its middle third while the mouse is
            // free, and the rows of the always reachable controls.
            let mut grab = Vec::new();
            if !self.locked {
                let third = (s1 - s0) / 3;
                grab.push((s0 + third, s1 - third));
            }
            if let Some(r) = self.orders_shown() {
                // Where they are drawn, along all of their rows (columns),
                // whether they reach over the gap or not, a little around
                // them: the pointer crosses from one view to the other
                // anywhere there.
                let r = [r[0] - GAP, r[1] - GAP, r[2] + 2 * GAP, r[3] + 2 * GAP];
                let (a0, a1) = if b.vertical { (r[1], r[1] + r[3]) } else { (r[0], r[0] + r[2]) };
                grab.push((a0.max(s0), a1.min(s1)));
            }
            grab.retain(|&(a, z)| z > a);
            if self.pointer_free_everywhere() {
                // (Drag map movement: the pointer goes anywhere, the line
                // only separates the pictures.)
                out.segments.extend(carve(b, &[], false, ends));
            } else {
                // Open where a press grabs it, and where the driver's
                // interface reaches across the gap (over it there).
                let mut open = grab.clone();
                if self.fit.is_none() {
                    let moved = self.moved.iter().map(|(r, _)| r);
                    for &r in self.interface.iter().chain(&self.driver_rects).chain(moved) {
                        let (c0, c1) = if b.vertical { (r[0], r[0] + r[2]) } else { (r[1], r[1] + r[3]) };
                        if c0 < b.across.0 && c1 > b.across.1 {
                            open.push(if b.vertical { (r[1], r[1] + r[3]) } else { (r[0], r[0] + r[2]) });
                        }
                    }
                }
                out.segments.extend(carve(b, &open, true, ends));
            }
            out.handles = grab.iter().map(|&a| {
                let r = b.rect_of(a);
                if b.vertical { [r[0] - GRAB, r[1], r[2] + 2 * GRAB, r[3]] } else { [r[0], r[1] - GRAB, r[2], r[3] + 2 * GRAB] }
            }).collect();
        }
        if let (Some(b), Some((s, _))) = (out.strip, strip) {
            // Open below the interface over the strip: the controls stay
            // reachable from the map below (`Layout::zoom_pointer_walls`).
            let open: Vec<(i32, i32)> = self.strip_holes(s).iter().map(|c| (c[0], c[0] + c[2])).collect();
            let ends = [end(b, b.along.0, out.split, w), end(b, b.along.1, out.split, w)];
            out.segments.extend(carve(b, &open, true, ends));
        }
        out
    }

    /// The driver's interface over the zoom strip `s` (or its divider), as
    /// the pointer reaches it: each run on down through the divider.
    pub(super) fn strip_holes(&self, s: [i32; 4]) -> Vec<[i32; 4]> {
        let reach = [s[0], s[1], s[2], s[3] + self.divider_thickness()];
        self.interface_holes().into_iter().filter(|c| super::overlap(reach, *c)).map(|mut c| {
            c[3] = c[3].max(reach[1] + reach[3] - c[1]);
            c
        }).collect()
    }

    /// The driver's interface the pointer reaches over a zoom.
    pub(super) fn interface_holes(&self) -> Vec<[i32; 4]> {
        if let Some(shown) = &self.interface_shown {
            shown.iter().map(|s| s.rect).collect()
        } else if self.fit.is_none() {
            self.interface.iter().chain(&self.driver_rects).copied().chain(self.moved.iter().map(|(r, _)| *r)).collect()
        } else {
            vec![]
        }
    }
}

