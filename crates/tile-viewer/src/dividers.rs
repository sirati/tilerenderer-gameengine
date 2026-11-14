//! The dividers' geometry, in one place: the split's divider between A
//! and B, and the zoom strip's divider below the strip. A child of
//! `layout`.
//!
//! [`Layout::dividers`] is the only code that decides where a divider is,
//! how thick it is, where it stops and where the pointer passes it. Every
//! consumer reads that one value:
//!
//! - drawing: the frosted strips over the split's gap and the strip's
//!   plain separator, or the glass bars ([`Segment::glass`], with
//!   glass dividers) and the view whose picture goes on under each bar
//!   ([`Dividers::under`]);
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

use super::{subtract_spans, Axis, Hit, Layout, Side, ZoomPlace};

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

/// How a segment is drawn as a glass bar (a the application frame effect): the
/// lens's rectangle (the segment's, run on past the window's edge and
/// into a divider it meets, so that only its free ends show their round
/// caps), its corner radius and rim (half the thickness: a rounded rod),
/// and whether it goes over the others (a divider others end against).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GlassBar {
    pub rect: [i32; 4],
    /// Where it is drawn (x0, y0, x1, y1; `Camera::target_clip`): the
    /// segment and the glass's faint halo around it, but not on into a
    /// divider it meets (that one's side closes it off cleanly).
    pub clip: [i32; 4],
    pub radius: f32,
    pub rim: f32,
    pub on_top: bool,
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

    /// The glass bar drawing it, exactly as thick as the segment (its
    /// wall); `junction`: the thickness of a divider it meets.
    pub fn glass(&self, junction: i32) -> GlassBar {
        let t = self.band.thickness();
        let run = |end: End| match end {
            // (Its round cap beyond the window.)
            End::Edge => t,
            // (Its round cap under the middle of the divider it meets,
            // which is drawn over it.)
            End::Junction => (junction + 1) / 2,
            End::Free => 0,
        };
        let along = (self.along.0 - run(self.ends[0]), self.along.1 + run(self.ends[1]));
        // (The halo: the application frame effect, 0.28 of the rim.)
        let halo = (t as f32 / 2.0 * 0.28).ceil() as i32 + 1;
        let shown = |at: i32, end: End, out: i32| if end == End::Junction { at } else { at + out };
        let (a0, a1) = (shown(self.along.0, self.ends[0], -halo - run(self.ends[0])), shown(self.along.1, self.ends[1], halo + run(self.ends[1])));
        let [x, y, w, h] = self.band.rect_of((a0, a1));
        let clip = if self.band.vertical { [x - halo, y, x + w + halo, y + h] } else { [x, y - halo, x + w, y + h + halo] };
        GlassBar { rect: self.band.rect_of(along), clip, radius: t as f32 / 2.0, rim: t as f32 / 2.0, on_top: !self.ends.contains(&End::Junction) }
    }

}

/// What a glass divider bends: a view's picture going on under it, past
/// the view's own rectangle (only drawing goes on there: the walls, the
/// hits and the views' rectangles stay where [`Dividers::segments`] and
/// the panes have them). One view lies under each stretch, so the glass
/// bends one continuous picture: no background, no seam between two views.
///
/// - The split's divider: the first view's picture (left, or top) under all
///   of its band, the open stretches and the free ends' caps too; the
///   second view's where the first shows a screen; the background where
///   neither shows the world.
/// - The strip's divider: the picture of the view below it; where it
///   crosses a side by side split, the left one's up to the split's far
///   side (the same view as under the split's divider, so the two bars
///   meet over one picture), the right one's from there.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UnderGlass {
    pub kind: Kind,
    /// Whose picture (none: the background colour).
    pub side: Option<Side>,
    /// The window rectangle it covers.
    pub rect: [i32; 4],
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
    /// With glass dividers: what goes on under the glass ([`UnderGlass`]).
    /// Drawing only.
    pub under: Vec<UnderGlass>,
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

    /// The segments with their glass bars in drawing order: those ending
    /// against another divider first, the ones they end against over them.
    pub fn glass_order(&self) -> Vec<(Segment, GlassBar)> {
        let meeting = |kind: Kind| match kind {
            Kind::Split => self.strip.map_or(0, |b| b.thickness()),
            Kind::Strip => self.split.map_or(0, |b| b.thickness()),
        };
        let mut out: Vec<_> = self.segments.iter().map(|s| (*s, s.glass(meeting(s.band.kind)))).collect();
        out.sort_by_key(|(_, g)| g.on_top);
        out
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
    /// window pixels (at least [`GLASS_WIDTH`] with glass dividers), at
    /// least one pixel.
    pub(crate) fn divider_thickness(&self) -> i32 {
        let width = self.separator_width.clamp(1, 16);
        let width = if self.effects_enabled && self.glass_dividers { width.max(GLASS_WIDTH) } else { width };
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
            // The stretch along it (exactly) of an interface rectangle drawn
            // over its band, where the viewer drew it: none where it is
            // drawn clear of it, however close.
            let over = |r: [i32; 4]| {
                let (c0, c1) = if b.vertical { (r[0], r[0] + r[2]) } else { (r[1], r[1] + r[3]) };
                let (a0, a1) = if b.vertical { (r[1], r[1] + r[3]) } else { (r[0], r[0] + r[2]) };
                (c0 < b.across.1 && c1 > b.across.0).then_some((a0.max(s0), a1.min(s1)))
            };
            // Where a press grabs it: its middle third while the mouse is
            // free, and the rows of the always reachable controls where
            // they are drawn over it (their bounding rectangle: split at
            // the gap, the gap between their two halves too). Moved into
            // the active view they are clear of it: the pointer reaches
            // them in the view it is in, the divider runs on beside them.
            let mut grab = Vec::new();
            if !self.locked {
                let third = (s1 - s0) / 3;
                grab.push((s0 + third, s1 - third));
            }
            grab.extend(self.orders_shown().and_then(over));
            grab.retain(|&(a, z)| z > a);
            if self.pointer_free_everywhere() {
                // (Drag map movement: the pointer goes anywhere, the line
                // only separates the pictures.)
                out.segments.extend(carve(b, &[], false, ends));
            } else {
                // Open where a press grabs it, and where the driver's
                // interface is drawn over it (`Layout::interface_drawn`).
                let mut open = grab.clone();
                open.extend(self.interface_drawn().into_iter().filter_map(over));
                open.retain(|&(a, z)| z > a);
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
        if self.effects_enabled && self.glass_dividers {
            out.under = self.under_glass(&out, strip.and_then(|(_, side)| side));
        }
        out
    }

    /// What goes on under the glass of `dividers` ([`UnderGlass`]);
    /// `strip_side`: the pane the zoom strip is carved from (none: across
    /// the window).
    fn under_glass(&self, dividers: &Dividers, strip_side: Option<Side>) -> Vec<UnderGlass> {
        // A view whose pane shows a screen fitted into it has no picture to
        // go on.
        let fitted = self.fit.map(|f| f.hit);
        let own = |side: Side| (fitted != Some(if side == Side::A { Hit::Main } else { Hit::Pane })).then_some(side);
        let split_owner = own(Side::A).or(own(Side::B));
        let mut out = Vec::new();
        if let Some(b) = dividers.split {
            out.push(UnderGlass { kind: Kind::Split, side: split_owner, rect: b.rect() });
        }
        if let Some(b) = dividers.strip {
            let pieces = match (strip_side, dividers.split.filter(|s| s.vertical)) {
                (Some(side), _) => vec![(b.along, own(side))],
                // (Across a side by side split: the split's own column
                // belongs to the view under its divider.)
                (None, Some(s)) => vec![((b.along.0, s.across.0), own(Side::A)), (s.across, split_owner), ((s.across.1, b.along.1), own(Side::B))],
                (None, None) => vec![(b.along, own(Side::A))],
            };
            let mut spans: Vec<((i32, i32), Option<Side>)> = Vec::new();
            for ((a0, a1), side) in pieces {
                let (a0, a1) = (a0.max(b.along.0), a1.min(b.along.1));
                if a1 <= a0 {
                    continue;
                }
                match spans.last_mut() {
                    Some((span, s)) if *s == side && span.1 == a0 => span.1 = a1,
                    _ => spans.push(((a0, a1), side)),
                }
            }
            out.extend(spans.into_iter().map(|(span, side)| UnderGlass { kind: Kind::Strip, side, rect: b.rect_of(span) }));
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

    /// The driver's interface where it is drawn over the views (none over
    /// a screen fitted into a pane): where the viewer showed it, after
    /// placing it (`interface_shown`), else where the layout put it.
    fn interface_drawn(&self) -> Vec<[i32; 4]> {
        if self.fit.is_some() { vec![] } else { self.interface_holes() }
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


/// The glass dividers' least thickness, window pixels: enough for the
/// glass to read as a rod, little enough not to take much room.
pub const GLASS_WIDTH: u32 = 16;
