//! Button snapping: a mode in which the pointer goes from one thing of the
//! driver's interface that can be clicked (`scene::Interactable`: buttons,
//! list entries, lists) to the next rather than move freely.
//!
//! **The mode.** Alt pressed on its own turns it on, on any screen, the
//! mouse captured or not; it stays on once Alt is let go. Alt again, Esc or
//! Ctrl (which also lets go of a captured mouse) turn it off. Alt held
//! with another key is a shortcut for the driver: the driver gets Alt and
//! the key, and the mode does not come on. While the mode is on its pointer
//! (and a translucent ghost) are the only cursor ([`SnapInput::active`],
//! `crate::cursor::SnapMode`): the system pointer and the world cursor are
//! not shown. The mouse's raw motion steers the pointer; the layout's
//! divider, its walls and the window's edges (edge scrolling) do not apply
//! to it: it reaches the interface anywhere in the window, in any view,
//! the corner's buttons and popups drawn elsewhere included
//! ([`Layout::moved_parts`]). Turned on again, it goes on from the element
//! it last rested on (if that is still there), else from the one nearest
//! the pointer.
//!
//! **Where it goes.** On Alt it snaps at once to the element nearest the
//! pointer. Then, at most [`Rates::updates_per_second`] times a second, the
//! mouse's motion since the last update picks the next element: one lying
//! within [`CONE`] (45°) of the motion's direction, seen from the element
//! snapped to, the one most nearly along that direction (the angle weighs
//! far more than the distance) at about the distance the mouse moved.
//! Motion shorter than [`DEAD_ZONE`] picks nothing (jitter does not make it
//! flip); without an element within the cone it stays. The first update
//! after the mouse rested comes at once. A control a person sees as more
//! than where it takes clicks (a check box with its label,
//! `scene::Interactable::region`; a slider or scroll bar's whole bar)
//! draws the pointer from all of it, and the pointer still lands where a
//! click works: on the box, on the slider's knob.
//!
//! **How it moves.** Every move is a flight ([`Flight`]) from where the
//! pointer is to the element's middle, lasting exactly [`FLIGHT`] update
//! intervals (1.33: 444 ms at 3 a second): a damped spring (accelerating
//! smoothly, overshooting a little and settling back, as Apple's) eased
//! onto the target at its end, so that it lands on it exactly then. A new
//! target in flight keeps the pointer's velocity and steers it there, the
//! flight timed anew. It lands also once the mode is turned off; it rests
//! only on an element. Only at rest does the driver get the pointer there
//! (the element shows its hover).
//!
//! **Clicks** go to the element snapped to, never to the world. Each view
//! of a split (a single view if it follows the world cursor) has an
//! attractor, an element too ([`TargetKind::Capture`], from the layout,
//! `Layout::attractors`, not the driver), snapped to at its middle, which
//! draws the pointer from afar ([`ATTRACTION`]) and is drawn there while
//! the mouse is free ([`ATTRACTOR_SPRITES`]): clicking it ends the mode
//! and goes back into the game in that view, capturing the mouse in the
//! one following the world cursor where it may be (else the free pointer
//! put there), the press and its release going nowhere else. With
//! `viewer_settings::Input::alt_in_menus` the mode comes on by itself
//! while a menu is open and goes with the last ([`SnapInput::set_alt_in_menus`]).
//! A command given in the mode (a
//! click on its button, or its key) that waits for a place of the world
//! (`scene::Interface::targeting`, e.g. a go-to's destination) ends the
//! mode the same way: the mouse is captured again, the view and the world
//! cursor staying put, the tile under it the command's target.
//!
//! **Lists.** On an entry of a list that scrolls, motion up or down that
//! stays inside the list scrolls it instead, smoothly ([`Scroller`]: wheel
//! notches spread out in time), the pointer staying on its entry.
//!
//! **Sliders by keyboard** (in the mode or not). A slider or scroll bar is
//! selected by a left press the driver gets on it (a click or the start of
//! a drag), by its Alt edit, or by stepping it; it stays selected as the
//! one last interacted with. In the mode the slider snapped to is the one
//! (none while snapped to anything else). Arrow keys step it
//! (`SliderAction::Step`: Left/Up down, Right/Down up, Shift ten steps;
//! the driver's own step) and never reach the driver or the world. Esc, a
//! press anywhere else (the world too), or the slider no longer shown (a
//! tab or screen changed) deselect it. The selected slider is drawn
//! hovered (`looks`). Without one, arrow keys are what they were.
//!
//! **Leaving.** The pointer lands, and the usual rules take over where it
//! is: on an element of the driver's the system pointer shows there (a
//! captured mouse is let go there); on the capture view the world cursor
//! goes on. Should the element snapped to go (a screen opened over it), the
//! pointer goes on to the one nearest it.
//!
//! [`Layout::moved_parts`]: crate::layout::Layout::moved_parts

use std::f64::consts::PI;

use scene::{Button, Input, Interactable, InteractableKind, SliderAction};

use crate::layout::Layout;

/// A window position in pixels (fractional while moving).
pub type Point = (f64, f64);

/// How long a flight lasts, in update intervals: 4/3 (1.33; 444 ms at 3
/// updates a second). The next update comes before it lands, so a pointer
/// kept moving is steered on rather than stopped.
pub const FLIGHT: f64 = 4.0 / 3.0;

/// The flight's spring: its damping fraction (ζ, SwiftUI's
/// `dampingFraction`), and when (a fraction of the flight) it first reaches
/// its target from rest. From rest it overshoots by about
/// `e^(−πζ/√(1−ζ²))`, 4.6 % at 0.7, a little less as it is eased in.
pub const DAMPING: f64 = 0.7;
pub const CROSSING: f64 = 0.5;

/// From this fraction of the flight on the spring is eased onto the target
/// (smoothstep), landing on it at the flight's end, still.
pub const SETTLE_FROM: f64 = 0.6;

/// The next element lies within this angle of the mouse's motion (radians).
pub const CONE: f64 = PI / 4.0;

/// Motion (pixels) an update needs to pick an element: less is jitter.
pub const DEAD_ZONE: f64 = 12.0;

/// Motion short of twice the dead zone is forgotten after the mouse rested
/// this long (seconds): slow drift adds up, a nudge long ago does not (nor
/// the rest of a nudge that picked at once).
pub const IDLE_RESET: f64 = 0.25;

/// How much an element off the motion's direction is held back: its
/// distance term times `1 + ANGLE_WEIGHT · (angle / CONE)²` (9 at the
/// cone's edge, 2.6 at 20°, 1.4 at 10°): one straight ahead wins over one
/// 20° off unless that is more than twice as near.
pub const ANGLE_WEIGHT: f64 = 8.0;

/// The distance term: how far an element is from the distance the mouse
/// moved, plus this much of its distance (nearer ones first when equally
/// far off).
pub const DISTANCE_WEIGHT: f64 = 0.25;

/// The capture view's middle (its target) attracts the pointer: in
/// [`pick`] it is as far as the edge of a disc round it, its radius this
/// fraction of the view's smaller side (a quarter: the middle of a 600
/// pixel tall view counts 150 pixels nearer than it is), so that motion
/// towards the middle of the view reaches it past the elements on the way.
pub const ATTRACTION: f64 = 0.25;

/// Window pixels per raw mouse count: slower than an unaccelerated system
/// pointer, so a flick a little too fast or off-angle does not carry across
/// the screen to a far element.
pub const MOTION_GAIN: f64 = 0.7;

/// Track pixels per raw mouse count while an Alt-mode slider is edited (as
/// an unaccelerated system pointer).
pub const SLIDER_GAIN: f64 = 1.0;

/// Wheel notches for a list's scroll are spread out with this time
/// constant (seconds): what an update asks for is 95 % sent by the next
/// update at 3 a second (3 time constants).
pub const SCROLL_SMOOTHING: f64 = 0.1;

/// At most this many wheel notches a second (a game takes each as a step).
pub const MAX_NOTCHES_PER_SECOND: f64 = 30.0;

/// Pixels a list scrolls for one notch when neither the list nor its
/// entries tell.
pub const DEFAULT_SCROLL_STEP: f64 = 16.0;

/// One notch, as `Input::Wheel` counts them.
const NOTCH: i32 = 120;

/// The sprite sets of the snapping pointer and its half-transparent ghost
/// (the pointer's tip at the anchor): the driver's arrow and its ghost if
/// the description has them, else the viewer's own.
pub const ARROW_SPRITE: &str = "Viewer@cursors/0";
pub const ARROW_GHOST_SPRITE: &str = "Viewer@cursor_ghost/0";
pub const POINTER_SPRITE: &str = "Viewer@pointer/0";
pub const GHOST_SPRITE: &str = "Viewer@pointer_ghost/0";
/// The attractors (a click there goes back into the game): the game's
/// crosshair, translucent (else the opaque crosshair, else the ghost).
pub const ATTRACTOR_SPRITES: &str = "Viewer@attractor/";

/// The id of the capture view's target (no driver's element has it).
pub const CAPTURE_ID: u64 = u64::MAX;
/// The Alt mode's target on the first queued report's button (and below it,
/// none of the driver's: its ids are its controls' addresses).
pub const QUEUED_ID: u64 = u64::MAX - 0x100;

const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_SHIFT: u32 = 0x10;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const VK_C: u32 = 0x43;

/// Something to snap to, in window pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Target {
    /// x, y, width, height: where it takes clicks.
    pub rect: [f64; 4],
    /// The whole control as a person sees it (`rect` and more: a check
    /// box's label; `Interactable::region`), placed the same way: the
    /// snapping pointer is drawn to it from anywhere in it, and lands on
    /// [`Target::center`] all the same.
    pub area: [f64; 4],
    /// It has an area of its own (`Interactable::region`: a check box
    /// and its label), even one no larger than `rect` (a menu's check box
    /// takes clicks on its label too, its pointer spot on the box).
    pub spread: bool,
    /// Preferred pointer tip, transformed with the same affine as `rect`.
    pub anchor: Option<Point>,
    pub kind: TargetKind,
    pub id: u64,
    /// A left click on it goes to the driver as a right click
    /// ([`Interactable::alt_right_click`]).
    pub alt_right_click: bool,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TargetKind {
    Button,
    /// A slider's shown knob-center travel and position in millionths.
    Slider { track: [f64; 4], value: i32 },
    /// An entry, of the list at this index of the targets if it lies in one.
    ListItem { list: Option<usize> },
    /// A list whose entries scroll in `scroll`, `step` pixels a notch;
    /// snapped to itself only if no entry of it is known.
    List { scroll: [f64; 4], step: f64, has_items: bool },
    /// The view a click captures the mouse in (`rect`), snapped to at `at`
    /// (where the world cursor is in it): a click there captures the mouse.
    Capture { at: Point },
}

fn inside([x, y, w, h]: [f64; 4], (px, py): Point) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

impl Target {
    /// The capture view `rect` (window pixels), the world cursor at `at`.
    pub fn capture(rect: [i32; 4], at: (i32, i32)) -> Self {
        let [x, y, w, h] = rect;
        let rect = [x as f64, y as f64, w as f64, h as f64];
        Self { rect, area: rect, spread: false, anchor: None, kind: TargetKind::Capture { at: (at.0 as f64, at.1 as f64) }, id: CAPTURE_ID, alt_right_click: false }
    }

    /// Where the pointer goes on it: a slider's (or scroll bar's) knob,
    /// as where a person grabs it.
    pub fn center(&self) -> Point {
        let knob = match self.kind {
            TargetKind::Capture { at } => return at,
            TargetKind::Slider { track: [tx, ty, tw, th], value } => {
                let t = value.clamp(0, 1_000_000) as f64 / 1_000_000.0;
                // (A tall track is vertical, as `InteractableKind::Slider`.)
                Some(if th > tw { (tx + tw / 2.0, ty + t * th) } else { (tx + t * tw, ty + th / 2.0) })
            }
            _ => None,
        };
        let [x, y, w, h] = self.rect;
        // Whole pixels strictly inside the visible hit rectangle. A text
        // or icon anchor never changes the entry's actual hit area.
        let (px, py) = self.anchor.or(knob).unwrap_or((x + w / 2.0, y + h / 2.0));
        (px.floor().clamp(x.ceil(), ((x + w).ceil() - 1.0).max(x.ceil())),
         py.floor().clamp(y.ceil(), ((y + h).ceil() - 1.0).max(y.ceil())))
    }

    /// How far `p` is from the whole control (0 inside).
    fn distance(&self, p: Point) -> f64 {
        let q = self.closest(p);
        (q.0 - p.0).hypot(q.1 - p.1)
    }

    /// Seen by [`pick`] as all of its [`Target::area`], not a point: a
    /// control larger than where it takes the pointer (a check box with
    /// its label), and a slider or scroll bar (the pointer goes on its
    /// knob, the whole bar is the control).
    fn spread(&self) -> bool {
        !self.is_capture() && (self.spread || self.area != self.rect || matches!(self.kind, TargetKind::Slider { .. }))
    }

    /// The point of the whole control ([`Target::area`]) nearest `p`.
    fn closest(&self, (px, py): Point) -> Point {
        let [x, y, w, h] = self.area;
        (px.clamp(x, x + w), py.clamp(y, y + h))
    }

    fn snappable(&self) -> bool {
        !matches!(self.kind, TargetKind::List { has_items: true, .. })
    }

    fn is_capture(&self) -> bool {
        matches!(self.kind, TargetKind::Capture { .. })
    }

    /// The list it scrolls with (an entry's list, or a list itself).
    fn list(&self, index: usize) -> Option<usize> {
        match self.kind {
            TargetKind::ListItem { list } => list,
            TargetKind::List { .. } => Some(index),
            TargetKind::Button | TargetKind::Slider { .. } | TargetKind::Capture { .. } => None,
        }
    }
}

/// The driver's interactables in window pixels: `mapping` is where its
/// screen lies (`Layout::screen_mapping`), those wholly outside the window
/// are left out. Entries are matched with the list they lie in.
pub fn targets(items: &[Interactable], mapping: ((i32, i32), (u32, u32)), window: (u32, u32)) -> Vec<Target> {
    let ((ox, oy), (n, d)) = mapping;
    let scale = n.max(1) as f64 / d.max(1) as f64;
    targets_mapped(items, |[x, y, w, h]| [ox as f64 + x as f64 * scale, oy as f64 + y as f64 * scale, w as f64 * scale, h as f64 * scale], window)
}

/// [`targets`] with each rectangle placed by `map` (driver's pixels to the
/// window's).
pub fn targets_mapped(items: &[Interactable], map: impl Fn([i32; 4]) -> [f64; 4], window: (u32, u32)) -> Vec<Target> {
    // (A unit's size as shown: for the guessed scroll step.)
    let scale = map([0, 0, 1000, 1000])[3] / 1000.0;
    let on_screen = |[x, y, w, h]: [f64; 4]| w > 0.0 && h > 0.0 && x + w > 0.0 && y + h > 0.0 && x < window.0 as f64 && y < window.1 as f64;
    let mut out: Vec<Target> = items
        .iter()
        .map(|i| Target {
            alt_right_click: i.alt_right_click,
            rect: map(i.rect),
            spread: i.region.is_some(),
            // (The same affine as the control's, as its anchor.)
            area: i.region.map_or_else(|| map(i.rect), |[x, y, w, h]| {
                let shown = map(i.rect);
                let (sx, sy) = (shown[2] / i.rect[2].max(1) as f64, shown[3] / i.rect[3].max(1) as f64);
                let [l, t] = [shown[0] + (x - i.rect[0]) as f64 * sx, shown[1] + (y - i.rect[1]) as f64 * sy];
                // (Never less than the control itself.)
                let (r, b) = ((l + w as f64 * sx).max(shown[0] + shown[2]), (t + h as f64 * sy).max(shown[1] + shown[3]));
                let (l, t) = (l.min(shown[0]), t.min(shown[1]));
                [l, t, r - l, b - t]
            }),
            anchor: i.anchor.map(|[x, y]| {
                // Choose the affine from the full control rectangle. An
                // anchor on the other side of an overlap must not select
                // a different interface piece or placement.
                let shown = map(i.rect);
                (shown[0] + (x - i.rect[0]) as f64 * shown[2] / i.rect[2].max(1) as f64,
                 shown[1] + (y - i.rect[1]) as f64 * shown[3] / i.rect[3].max(1) as f64)
            }),
            kind: match i.kind {
                InteractableKind::Button => TargetKind::Button,
                InteractableKind::Slider { track, value } => {
                    let shown = map(i.rect);
                    let (sx, sy) = (shown[2] / i.rect[2].max(1) as f64, shown[3] / i.rect[3].max(1) as f64);
                    let track = [shown[0] + (track[0] - i.rect[0]) as f64 * sx,
                                 shown[1] + (track[1] - i.rect[1]) as f64 * sy,
                                 track[2] as f64 * sx, track[3] as f64 * sy];
                    TargetKind::Slider { track, value }
                }
                InteractableKind::ListItem => TargetKind::ListItem { list: None },
                InteractableKind::List { scroll, step } => {
                    // (Its step as large as it shows.)
                    let shown = map(scroll);
                    let scale = if scroll[3] > 0 { shown[3] / scroll[3] as f64 } else { 1.0 };
                    TargetKind::List { scroll: shown, step: step.max(0) as f64 * scale, has_items: false }
                }
            },
            id: i.id,
        })
        .filter(|t| on_screen(t.rect))
        .collect();
    // Each entry belongs to the smallest list its middle lies in.
    let lists: Vec<(usize, [f64; 4])> = out
        .iter()
        .enumerate()
        .filter_map(|(k, t)| match t.kind {
            TargetKind::List { scroll, .. } => Some((k, scroll)),
            _ => None,
        })
        .collect();
    let mut heights: Vec<Vec<f64>> = vec![Vec::new(); out.len()];
    for t in &mut out {
        if let TargetKind::ListItem { list } = &mut t.kind {
            let c = (t.rect[0] + t.rect[2] / 2.0, t.rect[1] + t.rect[3] / 2.0);
            *list = lists.iter().filter(|(_, s)| inside(*s, c)).min_by(|a, b| (a.1[2] * a.1[3]).total_cmp(&(b.1[2] * b.1[3]))).map(|(k, _)| *k);
            if let Some(k) = *list {
                heights[k].push(t.rect[3]);
            }
        }
    }
    for (k, t) in out.iter_mut().enumerate() {
        if let TargetKind::List { step, has_items, .. } = &mut t.kind {
            *has_items = !heights[k].is_empty();
            if *step <= 0.0 {
                // The entries' usual height, else a guess.
                heights[k].sort_by(f64::total_cmp);
                *step = heights[k].get(heights[k].len() / 2).copied().unwrap_or(DEFAULT_SCROLL_STEP * scale);
            }
        }
    }
    out
}

/// Clip hit areas after their affine and semantic anchor have been placed.
/// Preserve each list relationship when invisible targets are removed.
fn clip_targets(targets: &mut Vec<Target>, clip: impl Fn(&Target) -> Option<[f64; 4]>) {
    let ids: Vec<u64> = targets.iter().map(|t| t.id).collect();
    let intersection = |[x, y, w, h]: [f64; 4], [cx, cy, cw, ch]: [f64; 4]| {
        let (l, t) = (x.max(cx), y.max(cy));
        let (r, b) = ((x + w).min(cx + cw), (y + h).min(cy + ch));
        // Driver pointer events use whole pixels. A subpixel sliver with
        // no whole pixel inside it cannot be clicked.
        (l.ceil() < r.ceil() && t.ceil() < b.ceil()).then_some([l, t, r - l, b - t])
    };
    targets.retain_mut(|target| {
        let Some(clip) = clip(target) else { return false };
        let Some(rect) = intersection(target.rect, clip) else { return false };
        target.rect = rect;
        target.area = intersection(target.area, clip).unwrap_or(rect);
        if let TargetKind::List { scroll, .. } = &mut target.kind {
            *scroll = intersection(*scroll, clip).unwrap_or(rect);
        }
        true
    });
    let placed_ids: Vec<u64> = targets.iter().map(|t| t.id).collect();
    let mut children = vec![false; targets.len()];
    for target in targets.iter_mut() {
        if let TargetKind::ListItem { list } = &mut target.kind {
            *list = list.and_then(|old| ids.get(old)).and_then(|id| placed_ids.iter().position(|candidate| candidate == id));
            if let Some(k) = *list { children[k] = true; }
        }
    }
    for (k, target) in targets.iter_mut().enumerate() {
        if let TargetKind::List { has_items, .. } = &mut target.kind { *has_items = children[k]; }
    }
}

/// The driver's element to snap to from `p`: the nearest (by distance to
/// the whole control, [`Target::area`], then to where the pointer goes on
/// it). Not the capture view.
pub fn nearest(targets: &[Target], p: Point) -> Option<usize> {
    let key = |t: &Target| {
        let c = t.center();
        (t.distance(p), (c.0 - p.0).hypot(c.1 - p.1))
    };
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.snappable() && !t.is_capture())
        .min_by(|(_, a), (_, b)| {
            let (ka, kb) = (key(a), key(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })
        .map(|(k, _)| k)
}

/// The element the mouse's `moved` (pixels since the last update) points
/// at from `from` (the element snapped to, `current`): within [`CONE`] of
/// its direction, the angle weighing heavily, at about the distance moved.
/// None for motion within the [`DEAD_ZONE`], or nothing within the cone.
/// An element is seen where it takes the pointer ([`Target::center`]); one
/// a person sees as more than that ([`Target::spread`]: a check box with
/// its label, a scroll bar's whole length) where that comes nearest where
/// the motion leads, so all of it draws the pointer. Of those the motion
/// leads into alike, the one whose pointer spot is nearest.
pub fn pick(targets: &[Target], from: Point, current: Option<u64>, moved: Point) -> Option<usize> {
    let reach = moved.0.hypot(moved.1);
    if reach < DEAD_ZONE {
        return None;
    }
    let dir = (moved.0 / reach, moved.1 / reach);
    let aim = (from.0 + moved.0, from.1 + moved.1);
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.snappable() && Some(t.id) != current)
        .filter_map(|(k, t)| {
            let spot = t.center();
            let c = if t.spread() { t.closest(aim) } else { spot };
            let d = (c.0 - from.0, c.1 - from.1);
            let dist = d.0.hypot(d.1);
            if dist < 1.0 {
                return None;
            }
            let angle = ((d.0 * dir.0 + d.1 * dir.1) / dist).clamp(-1.0, 1.0).acos();
            if angle > CONE + 1e-9 {
                return None;
            }
            // (The capture view's middle draws the pointer from afar: it
            // counts as near as the edge of its disc of attraction.)
            let near = if t.is_capture() { (dist - ATTRACTION * t.rect[2].min(t.rect[3])).max(0.0) } else { dist };
            let off = (near - reach).abs() + DISTANCE_WEIGHT * near + 1.0;
            Some((k, off * (1.0 + ANGLE_WEIGHT * (angle / CONE).powi(2)), (spot.0 - aim.0).hypot(spot.1 - aim.1)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(k, _, _)| k)
}

/// The angle (degrees) between `moved` and the way from `from` to `to`.
fn degrees(from: Point, to: Point, moved: Point) -> f64 {
    let d = (to.0 - from.0, to.1 - from.1);
    let n = d.0.hypot(d.1) * moved.0.hypot(moved.1);
    if n == 0.0 { 0.0 } else { ((d.0 * moved.0 + d.1 * moved.1) / n).clamp(-1.0, 1.0).acos().to_degrees() }
}

/// The visible pointer's way to its target: from `from` at `vel` (pixels a
/// second) at time `start`, landing on `to` exactly `duration` seconds
/// later, still. A damped spring ([`DAMPING`], first reaching the target
/// at [`CROSSING`] of the way from rest), eased onto the target from
/// [`SETTLE_FROM`] on; the starting velocity carries on and dies out the
/// same way.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Flight {
    pub from: Point,
    pub vel: Point,
    pub to: Point,
    pub start: f64,
    pub duration: f64,
}

/// The flight's shape at `u` (0..1 of its time): how much of the way from
/// `from` is left (`F`), how much of the starting velocity's travel
/// (`G`, in flight times), and their rates (per flight time).
fn shape(u: f64) -> (f64, f64, f64, f64) {
    let root = (1.0 - DAMPING * DAMPING).sqrt();
    let omega = (PI - DAMPING.acos()) / (root * CROSSING);
    let (a, wd) = (DAMPING * omega, omega * root);
    let e = (-a * u).exp();
    let (sin, cos) = (wd * u).sin_cos();
    let s = e * (cos + a / wd * sin);
    let s_rate = -e * omega * omega / wd * sin;
    let q = e * sin / wd;
    let q_rate = e * (cos - a / wd * sin);
    let v = ((u - SETTLE_FROM) / (1.0 - SETTLE_FROM)).clamp(0.0, 1.0);
    let m = 1.0 - v * v * (3.0 - 2.0 * v);
    let m_rate = -6.0 * v * (1.0 - v) / (1.0 - SETTLE_FROM);
    (s * m, s_rate * m + s * m_rate, q * m, q_rate * m + q * m_rate)
}

impl Flight {
    /// At rest at `at`.
    pub fn rest(at: Point) -> Self {
        Self { from: at, vel: (0.0, 0.0), to: at, start: 0.0, duration: 0.0 }
    }

    /// When it lands.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    pub fn done(&self, now: f64) -> bool {
        now >= self.end()
    }

    fn progress(&self, now: f64) -> Option<f64> {
        (self.duration > 0.0 && now < self.end()).then(|| ((now - self.start) / self.duration).max(0.0))
    }

    pub fn position(&self, now: f64) -> Point {
        let Some(u) = self.progress(now) else { return self.to };
        let (f, _, g, _) = shape(u);
        let d = self.duration;
        (self.to.0 + (self.from.0 - self.to.0) * f + self.vel.0 * d * g, self.to.1 + (self.from.1 - self.to.1) * f + self.vel.1 * d * g)
    }

    /// Pixels a second.
    pub fn velocity(&self, now: f64) -> Point {
        let Some(u) = self.progress(now) else { return (0.0, 0.0) };
        let (_, f_rate, _, g_rate) = shape(u);
        let d = self.duration;
        ((self.from.0 - self.to.0) * f_rate / d + self.vel.0 * g_rate, (self.from.1 - self.to.1) * f_rate / d + self.vel.1 * g_rate)
    }

    /// From where it is at `now`, at the speed it has, to `to` in
    /// `duration` seconds.
    pub fn retarget(&mut self, to: Point, now: f64, duration: f64) {
        *self = Self { from: self.position(now), vel: self.velocity(now), to, start: now, duration };
    }
}

/// Wheel notches spread out in time.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Scroller {
    /// Notches still to send (positive: wheel up, towards the top).
    pending: f64,
    /// Sent in part, not yet a whole notch.
    carry: f64,
    since_notch: f64,
}

impl Scroller {
    pub fn add(&mut self, notches: f64) {
        self.pending += notches;
    }

    pub fn idle(&self) -> bool {
        self.pending == 0.0 && self.carry == 0.0
    }

    /// `dt` seconds on: the notch to send now (+1 up, −1 down, 0 none).
    pub fn step(&mut self, dt: f64) -> i32 {
        self.since_notch += dt;
        let part = self.pending * (1.0 - (-dt / SCROLL_SMOOTHING).exp());
        self.pending -= part;
        self.carry += part;
        if self.pending.abs() < 0.02 {
            self.carry += std::mem::take(&mut self.pending);
        }
        // The last part of a notch rounds.
        let whole = if self.pending == 0.0 { 0.5 } else { 1.0 };
        if self.carry.abs() >= whole && self.since_notch >= 1.0 / MAX_NOTCHES_PER_SECOND {
            let n = self.carry.signum();
            self.carry -= n;
            self.since_notch = 0.0;
            if self.pending == 0.0 && self.carry.abs() < 0.5 {
                self.carry = 0.0;
            }
            return n as i32;
        }
        if self.pending == 0.0 && self.carry.abs() < 0.5 {
            self.carry = 0.0;
        }
        0
    }
}

/// What the snapping pointer does, in window pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Action {
    Move(Point),
    Button { button: Button, down: bool, at: Point },
    Wheel { delta: i32, at: Point },
    /// The capture view was clicked: the mode ends, the mouse is captured
    /// at this point.
    Capture(Point),
}

/// The element snapped to.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Snapped {
    id: u64,
    /// The list it scrolls with (its id).
    list: Option<u64>,
    /// Whether it is a list's entry (the pointer stays while the list
    /// scrolls under it).
    entry: bool,
    capture: bool,
    /// Its left clicks go to the driver as right clicks.
    alt_right_click: bool,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Off,
    /// The mode is on.
    Snapping,
    /// The mode was turned off in flight: the pointer lands.
    Landing,
    /// Landed after the mode: the pointer stays here until the mouse moves.
    Parked(Point),
}

/// How often things happen (from the settings).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rates {
    pub updates_per_second: f64,
    pub ghost_fps: f64,
}

impl Rates {
    pub fn of(input: &viewer_settings::Input) -> Self {
        Self { updates_per_second: (input.snap_updates_per_second as f64).clamp(0.5, 60.0), ghost_fps: (input.snap_ghost_fps as f64).clamp(1.0, 240.0) }
    }

    /// How long a flight lasts (seconds).
    pub fn flight(&self) -> f64 {
        FLIGHT / self.updates_per_second
    }
}

impl Default for Rates {
    fn default() -> Self {
        Self::of(&viewer_settings::Input::default())
    }
}

/// The snapping pointer's state (no input handling, no layout: see
/// [`SnapInput`]). Times are seconds on any clock.
#[derive(Clone, Debug, Default)]
pub struct Snapper {
    phase: Phase,
    flight: Flight,
    /// The visible pointer as of the last step.
    pos: Point,
    snapped: Option<Snapped>,
    ghost: Option<Point>,
    /// The flight landed and the driver has the pointer there (or a click
    /// put it there).
    arrived: bool,
    /// Mouse motion since the last update (pixels), whether some came since
    /// the last step, and how long none has come.
    moved: Point,
    moving: bool,
    idle: f64,
    next_update: f64,
    next_ghost: f64,
    last: f64,
    rates: Rates,
    scroll: Scroller,
    /// Notches moved in the list not sent yet (less than half of one).
    scroll_rest: f64,
    /// Notches were sent: once they are all out, the driver gets the
    /// pointer again (what is under it now shows its hover).
    scrolled: bool,
    /// Where each button's press went, and as which button.
    pressed: [Option<(Point, Button)>; 3],
    /// The driver's element the pointer last rested on (kept across modes:
    /// the next one goes on from it).
    rested: Option<u64>,
    /// Where it parked, until [`Snapper::take_parked`].
    parked: Option<Point>,
    /// When the element now snapped to was picked (for the log).
    picked: f64,
}

fn button_index(b: Button) -> usize {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

/// What an update decides.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Choice {
    /// Snap to the target at this index.
    Snap(usize),
    /// Stay, and scroll the list by this many pixels (positive: the pointer
    /// went down the list).
    Scroll { by: f64, step: f64 },
    Nothing,
}

impl Snapper {
    /// The mode is on.
    pub fn snapping(&self) -> bool {
        self.phase == Phase::Snapping
    }

    /// The snapping pointer is the pointer: the mode is on, or it lands.
    pub fn holds_pointer(&self) -> bool {
        matches!(self.phase, Phase::Snapping | Phase::Landing)
    }

    /// The visible pointer, while snapping, landing or parked.
    pub fn pointer(&self) -> Option<Point> {
        match self.phase {
            Phase::Snapping | Phase::Landing => Some(self.pos),
            Phase::Parked(p) => Some(p),
            Phase::Off => None,
        }
    }

    /// The ghost pointer, while snapping.
    pub fn ghost(&self) -> Option<Point> {
        self.ghost.filter(|_| self.snapping())
    }

    /// The point snapped to (where the visible pointer is headed).
    pub fn target(&self) -> Option<Point> {
        self.snapped.filter(|_| self.holds_pointer()).map(|_| self.flight.to)
    }

    /// The id of the element snapped to.
    pub fn target_id(&self) -> Option<u64> {
        self.snapped.filter(|_| self.holds_pointer()).map(|s| s.id)
    }

    pub fn flight(&self) -> &Flight {
        &self.flight
    }

    /// The driver's element the pointer last rested on.
    pub fn rested(&self) -> Option<u64> {
        self.rested
    }

    /// The pointer rests on a driver's element and the driver has it there
    /// (not in flight, not on an attractor): what the driver shows as its
    /// pointer now is the pointer for that element.
    pub fn resting_on_driver(&self) -> bool {
        self.holds_pointer() && self.arrived && self.snapped.is_some_and(|s| !s.capture)
    }

    /// Where the pointer parked (landed after the mode), once.
    pub fn take_parked(&mut self) -> Option<Point> {
        self.parked.take()
    }

    /// Turns the mode on at time `now`, the pointer at `at`: it goes on
    /// from the element it last rested on if that is still there (from
    /// there), else it flies to the one nearest `at` at once (the capture
    /// view if there is no other).
    pub fn enter(&mut self, now: f64, at: Point, targets: &[Target], rates: Rates) {
        *self = Self { pressed: self.pressed, rested: self.rested, ..Self::default() };
        self.phase = Phase::Snapping;
        self.rates = rates;
        self.last = now;
        self.pos = at;
        self.flight = Flight::rest(at);
        let resume = self.rested.and_then(|id| targets.iter().position(|t| t.id == id && !t.is_capture() && t.snappable()));
        if let Some(k) = resume {
            self.pos = targets[k].center();
            self.flight = Flight::rest(self.pos);
        }
        if log::log_enabled!(log::Level::Debug) {
            let list: Vec<String> = targets
                .iter()
                .map(|t| {
                    let kind = match t.kind {
                        TargetKind::Button => "button",
                        TargetKind::Slider { .. } => "slider",
                        TargetKind::ListItem { .. } => "entry",
                        TargetKind::List { .. } => "list",
                        TargetKind::Capture { .. } => "capture",
                    };
                    let [x, y, w, h] = t.rect;
                    format!("{kind} {:x} {x:.0},{y:.0} {w:.0}x{h:.0}", t.id)
                })
                .collect();
            log::debug!("snap: {} targets: {}", targets.len(), list.join("; "));
        }
        let k = resume.or_else(|| nearest(targets, at)).or_else(|| targets.iter().position(Target::is_capture));
        match k {
            Some(k) => {
                log::debug!("snap: on at {now:.3} s from {at:?}, {} {}", if resume.is_some() { "back on" } else { "nearest" }, targets[k].id);
                self.snap_to(targets, k, now);
            }
            None => log::debug!("snap: on at {now:.3} s from {at:?}, nothing to snap to"),
        }
        self.ghost = Some(self.flight.to);
        // (The first motion past the dead zone picks at once.)
        self.next_update = now;
        self.next_ghost = now + 1.0 / rates.ghost_fps;
    }

    /// Turns the mode off: the pointer lands (on the capture view: the
    /// world cursor is the pointer again at once). What the driver gets.
    pub fn leave(&mut self) -> Vec<Action> {
        if !self.snapping() {
            return vec![];
        }
        self.ghost = None;
        match self.snapped {
            Some(s) if !s.capture => {
                if self.flight.done(self.last) {
                    let moved = !self.arrived;
                    self.park();
                    if moved {
                        return vec![Action::Move(self.flight.to)];
                    }
                } else {
                    self.phase = Phase::Landing;
                }
            }
            _ => self.phase = Phase::Off,
        }
        vec![]
    }

    fn park(&mut self) {
        let p = self.flight.to;
        self.phase = Phase::Parked(p);
        self.pos = p;
        self.arrived = true;
        self.parked = Some(p);
        if let Some(s) = self.snapped.filter(|s| !s.capture) {
            self.rested = Some(s.id);
        }
        log::debug!("snap: off, the pointer stays at {p:?}");
    }

    /// Nothing more to do (the mouse was let go, or moved after snapping).
    pub fn stop(&mut self) {
        self.phase = Phase::Off;
        self.ghost = None;
        self.parked = None;
    }

    /// Raw mouse motion: steers the pointer while snapping (true; dropped
    /// while it lands); a pointer left by snapping is taken up by the mouse
    /// again (false: the motion is the mouse's as usual).
    pub fn motion(&mut self, d: Point) -> bool {
        match self.phase {
            Phase::Snapping => {
                self.moved = (self.moved.0 + d.0 * MOTION_GAIN, self.moved.1 + d.1 * MOTION_GAIN);
                self.moving = true;
                true
            }
            Phase::Landing => true,
            Phase::Parked(_) => {
                self.phase = Phase::Off;
                false
            }
            Phase::Off => false,
        }
    }

    /// A button: while snapping it goes to the element snapped to (the
    /// pointer put there), where the pointer parked to that point; a
    /// release goes where its press went. Nothing snapped to: nothing (never
    /// the world). `None`: not the snapping pointer's.
    pub fn click(&mut self, button: Button, down: bool) -> Option<Vec<Action>> {
        let b = button_index(button);
        if !down {
            return self.pressed[b].take().map(|(at, sent)| vec![Action::Button { button: sent, down, at }]);
        }
        let mut sent = button;
        let at = match self.phase {
            Phase::Snapping | Phase::Landing => match self.snapped {
                Some(s) if s.capture => return Some(vec![Action::Capture(self.flight.to)]),
                Some(s) => {
                    if s.alt_right_click && button == Button::Left {
                        sent = Button::Right;
                    }
                    // On the element now.
                    let at = self.flight.to;
                    self.flight = Flight::rest(at);
                    self.pos = at;
                    self.arrived = true;
                    self.rested = Some(s.id);
                    if self.phase == Phase::Landing {
                        self.park();
                    } else {
                        self.ghost = Some(at);
                    }
                    at
                }
                None => return Some(vec![]),
            },
            Phase::Parked(p) => p,
            Phase::Off => return None,
        };
        self.pressed[b] = Some((at, sent));
        Some(vec![Action::Move(at), Action::Button { button: sent, down, at }])
    }

    /// A wheel turn goes to the element snapped to, or where the pointer
    /// parked. `None`: not the snapping pointer's.
    pub fn wheel(&mut self, delta: i32) -> Option<Vec<Action>> {
        match self.phase {
            Phase::Snapping | Phase::Landing => Some(match self.snapped {
                Some(s) if !s.capture => vec![Action::Wheel { delta, at: self.flight.to }],
                _ => vec![],
            }),
            Phase::Parked(p) => Some(vec![Action::Wheel { delta, at: p }]),
            Phase::Off => None,
        }
    }

    /// Time `now` (`targets` as they are now): the pointer moves on, an
    /// update and the ghost come when due, the list scrolls.
    pub fn tick(&mut self, now: f64, targets: &[Target], rates: Rates) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.holds_pointer() {
            return out;
        }
        self.rates = rates;
        let dt = (now - self.last).max(0.0);
        self.last = now;
        if std::mem::take(&mut self.moving) {
            self.idle = 0.0;
        } else {
            self.idle += dt;
            // (A move on, beyond a nudge, waits for the next update.)
            if self.idle > IDLE_RESET && self.moved.0.hypot(self.moved.1) < 2.0 * DEAD_ZONE {
                self.moved = (0.0, 0.0);
            }
        }
        self.track(targets, now);
        if self.snapping() {
            let reach = self.moved.0.hypot(self.moved.1);
            if now >= self.next_update && reach >= DEAD_ZONE {
                self.update(targets, now);
                self.moved = (0.0, 0.0);
                self.next_update = now + 1.0 / rates.updates_per_second;
            }
            if now >= self.next_ghost {
                self.ghost = Some(match self.choose(targets) {
                    Choice::Snap(k) => targets[k].center(),
                    Choice::Scroll { .. } | Choice::Nothing => self.flight.to,
                });
                self.next_ghost = (self.next_ghost + 1.0 / rates.ghost_fps).max(now);
            }
        }
        self.pos = self.flight.position(now);
        if self.flight.done(now) && !self.arrived {
            self.arrived = true;
            if let Some(s) = self.snapped {
                log::debug!(
                    "snap: landed on {} at {:?} at {:.3} s, {:.0} ms after it was picked (flight {:.0} ms; seen by the step at {now:.3} s, {:.0} ms later)",
                    s.id,
                    self.flight.to,
                    self.flight.end().max(self.picked),
                    (self.flight.end() - self.picked).max(0.0) * 1000.0,
                    self.flight.duration * 1000.0,
                    (now - self.flight.end().max(self.picked)) * 1000.0
                );
                if !s.capture {
                    // At rest on the element: it shows its hover.
                    out.push(Action::Move(self.flight.to));
                    self.rested = Some(s.id);
                }
            }
        }
        if self.phase == Phase::Landing && self.flight.done(now) {
            self.park();
        }
        match self.scroll.step(dt) {
            0 => {}
            n => {
                log::debug!("snap: the list scrolls a notch {}", if n > 0 { "up" } else { "down" });
                out.push(Action::Wheel { delta: n * NOTCH, at: self.flight.to });
                self.scrolled = true;
            }
        }
        if self.scrolled && self.scroll.idle() {
            self.scrolled = false;
            if self.flight.done(now) {
                out.push(Action::Move(self.flight.to));
            }
        }
        out
    }

    /// Snaps to target `k`: the pointer flies there from where it is.
    fn snap_to(&mut self, targets: &[Target], k: usize, now: f64) {
        let t = &targets[k];
        let list = t.list(k).map(|l| targets[l].id);
        self.snapped = Some(Snapped { id: t.id, list, entry: matches!(t.kind, TargetKind::ListItem { .. }), capture: t.is_capture(), alt_right_click: t.alt_right_click });
        let c = t.center();
        self.picked = now;
        self.scroll_rest = 0.0;
        if c != self.flight.to || !self.flight.done(now) || self.pos != c {
            let duration = self.rates.flight();
            self.flight.retarget(c, now, duration);
            self.arrived = false;
            log::debug!("snap: at {now:.3} s picked {} ({:?}) at {c:?} from {:?}, lands at {:.3} s", t.id, t.kind, self.pos, now + duration);
        } else if !self.arrived {
            // Already there and still.
            self.flight = Flight::rest(c);
        }
    }

    /// What an update would do now.
    fn choose(&self, targets: &[Target]) -> Choice {
        let from = self.flight.to;
        let m = self.moved;
        // In a list that scrolls, motion up or down staying inside it (or
        // within a step past its edge: from its first or last entry shown)
        // scrolls it.
        let list = self.snapped.and_then(|s| s.list).and_then(|id| targets.iter().find(|t| t.id == id));
        if let Some(Target { rect, kind: TargetKind::List { step, .. }, .. }) = list {
            let [x, y, w, h] = *rect;
            let reach = [x, y - step, w, h + 2.0 * step];
            if m.0.hypot(m.1) >= DEAD_ZONE && m.1.abs() >= m.0.abs() && inside(reach, (from.0 + m.0, from.1 + m.1)) {
                return Choice::Scroll { by: m.1, step: *step };
            }
        }
        pick(targets, from, self.snapped.map(|s| s.id), m).map_or(Choice::Nothing, Choice::Snap)
    }

    /// An update: the motion picks the next element.
    fn update(&mut self, targets: &[Target], now: f64) {
        match self.choose(targets) {
            Choice::Snap(k) => {
                log::debug!("snap: motion {:?} picks {} at {:.0}° off it", self.moved, targets[k].id, degrees(self.flight.to, targets[k].center(), self.moved));
                self.snap_to(targets, k, now);
            }
            Choice::Scroll { by, step } => {
                // Moving down the list shows what is further down: the wheel
                // turned towards the user (negative).
                // (In whole notches, the rest kept for the next: short moves
                // add up.)
                self.scroll_rest += -by / step.max(1.0);
                if self.scroll_rest.abs() >= 0.5 {
                    let whole = self.scroll_rest.round();
                    self.scroll_rest -= whole;
                    self.scroll.add(whole);
                }
            }
            Choice::Nothing => log::debug!("snap: motion {:?}: nothing within {:.0}°, stays", self.moved, CONE.to_degrees()),
        }
    }

    /// Keeps the target on its element as the scene changes: an element
    /// that moved is followed; a list's entry stays where the pointer is
    /// (the entry there now, once the list scrolled).
    fn track(&mut self, targets: &[Target], now: f64) {
        let Some(s) = self.snapped else {
            if let Some(k) = nearest(targets, self.pos).or_else(|| targets.iter().position(Target::is_capture)) {
                self.snap_to(targets, k, now);
            }
            return;
        };
        let t = self.flight.to;
        if s.entry {
            if targets.iter().any(|x| x.id == s.id && inside(x.rect, t)) {
                return;
            }
            let list = s.list.and_then(|id| targets.iter().position(|x| x.id == id));
            let under = targets.iter().find(|x| matches!(x.kind, TargetKind::ListItem { list: l } if l == list) && inside(x.rect, t));
            match under {
                Some(x) => self.snapped = Some(Snapped { id: x.id, alt_right_click: x.alt_right_click, ..s }),
                None => self.gone(targets, s.id, now),
            }
            return;
        }
        match targets.iter().find(|x| x.id == s.id) {
            Some(x) => {
                let c = x.center();
                if c != t {
                    self.flight.retarget(c, now, self.rates.flight());
                    self.arrived = false;
                }
            }
            None => self.gone(targets, s.id, now),
        }
    }

    /// The element snapped to is gone (a screen opened over it, or closed):
    /// on to the one nearest the pointer (the capture view if there is no
    /// other), so that it never rests where nothing is.
    fn gone(&mut self, targets: &[Target], id: u64, now: f64) {
        let Some(k) = nearest(targets, self.pos).or_else(|| targets.iter().position(Target::is_capture)) else {
            self.snapped = None;
            self.ghost = None;
            self.rested = None;
            self.moved = (0.0, 0.0);
            self.flight = Flight::rest(self.pos);
            self.arrived = true;
            return;
        };
        log::debug!("snap: {id} is gone: on to the nearest, {}", targets[k].id);
        self.moved = (0.0, 0.0);
        self.snap_to(targets, k, now);
    }
}

#[derive(Debug)]
struct SliderEdit {
    id: u64,
    track: [f64; 4],
    value: f64,
    dirty: bool,
    /// An arrow key stepped it in the driver: the scene's value then, the
    /// first different one it shows becomes the edit's value.
    resync: Option<i32>,
}

/// Button snapping in the viewer: Alt turns the mode on and off; buttons
/// and the wheel go to the snapping pointer; everything else goes on to
/// the layout.
#[derive(Debug, Default)]
pub struct SnapInput {
    pub snapper: Snapper,
    ctrl: bool,
    /// Alt is held (its press repeats).
    alt: bool,
    /// This Alt press turned the mode on (another key with it undoes that).
    alt_entered: bool,
    /// Alt's press went to the driver (another key with it, or Ctrl held).
    alt_sent: bool,
    /// Esc's press turned the mode off (its release is not the driver's).
    esc: bool,
    /// Whether the system pointer is to be hidden, when that changes.
    hide_pointer: Option<bool>,
    /// Mouse motion edits this slider while the snapping pointer stays put.
    slider: Option<SliderEdit>,
    /// Initial selection and confirm/cancel presses never reach the driver.
    swallowed_buttons: [bool; 3],
    /// The driver's command waiting for a place of the world, as of the
    /// last scene seen (`scene::Interface::targeting`).
    targeting: bool,
    /// The slider last interacted with (pressed in the driver, edited or
    /// stepped in the Alt mode): arrow keys step it ([`SnapInput::selected_slider`]).
    selected: Option<u64>,
    /// The slider arrow keys step now, as of the last input or tick.
    shown: Option<u64>,
    /// Arrow keys whose press stepped a slider (their releases are ours).
    stepped_keys: Vec<u32>,
    shift: bool,
    /// The mode comes on by itself while a menu is open
    /// (`viewer_settings::Input::alt_in_menus`, [`SnapInput::set_alt_in_menus`]).
    alt_in_menus: bool,
    /// The mode is on because a menu is open (it goes off with the last).
    by_menu: bool,
    /// The player turned the mode off while menus were open: not on again
    /// by itself until they are all closed.
    menu_declined: bool,
    /// The attractors drawn (window pixels; whether a click captures), as
    /// of the last update: while the mode is on and the mouse is free.
    attractors: Vec<(Point, bool)>,
    /// Lists the viewer shows scrolled away from the driver's rows
    /// (`smooth_menus`): their rectangles and how far down the rows show.
    list_shifts: Vec<([i32; 4], i32)>,
}

/// An action as the driver gets it (nothing for a point it does not take
/// input at).
/// Moves the entries in each of `shifts`' rectangles (x, y, w, h) down by
/// its amount, clipped to it (an entry shown only in part is a target
/// where it shows).
fn shift_list_entries(items: &mut Vec<Interactable>, shifts: &[([i32; 4], i32)]) {
    items.retain_mut(|i| {
        if i.kind != InteractableKind::ListItem { return true }
        let middle = (i.rect[0] + i.rect[2] / 2, i.rect[1] + i.rect[3] / 2);
        let Some(&([x, y, w, h], dy)) = shifts.iter().find(|([x, y, w, h], _)| middle.0 >= *x && middle.0 < x + w && middle.1 >= *y && middle.1 < y + h) else { return true };
        let (top, bottom) = ((i.rect[1] + dy).max(y), (i.rect[1] + i.rect[3] + dy).min(y + h));
        if bottom - top < 2 || i.rect[0] >= x + w || i.rect[0] + i.rect[2] <= x { return false }
        i.rect[1] = top;
        i.rect[3] = bottom - top;
        if let Some(anchor) = &mut i.anchor { anchor[1] = (anchor[1] + dy).clamp(top, bottom - 1) }
        i.region = i.region.map(|[rx, ry, rw, rh]| [rx, ry + dy, rw, rh]);
        true
    });
}

fn to_input(action: Action, layout: &Layout) -> Option<Input> {
    let whole = |(x, y): Point| (x.round() as i32, y.round() as i32);
    Some(match action {
        Action::Move(p) => {
            let (x, y) = layout.route_window(whole(p))?;
            Input::MouseMove { x, y }
        }
        Action::Button { button, down, at } => {
            let (x, y) = layout.route_window(whole(at))?;
            Input::MouseButton { button, down, x, y }
        }
        Action::Wheel { delta, at } => {
            let (x, y) = layout.route_window(whole(at))?;
            Input::Wheel { delta, x, y }
        }
        Action::Capture(_) => return None,
    })
}

impl SnapInput {
    /// The snap mode is on (its pointer is the only cursor; it is the
    /// pointer while it lands too).
    pub fn active(&self) -> bool {
        self.snapper.holds_pointer()
    }

    /// Lists the viewer shows scrolled by `shifts` (rectangle, rows moved
    /// down) from where the driver has their rows: their entries are
    /// targets where they show.
    pub fn set_list_shifts(&mut self, shifts: Vec<([i32; 4], i32)>) {
        self.list_shifts = shifts;
    }

    /// [`SnapInput::targets`] with the lists' entries where they show.
    fn shown_targets(&self, scene: &scene::Scene, layout: &Layout) -> Vec<Target> {
        if self.list_shifts.is_empty() { return Self::targets(scene, layout) }
        let mut shown = scene.interactables.clone();
        shift_list_entries(&mut shown, &self.list_shifts);
        Self::targets_of(&shown, layout)
    }

    /// The targets in `scene` as `layout` shows them, and the view a click
    /// captures the mouse in.
    fn targets(scene: &scene::Scene, layout: &Layout) -> Vec<Target> {
        Self::targets_of(&scene.interactables, layout)
    }

    fn targets_of(interactables: &[Interactable], layout: &Layout) -> Vec<Target> {
        // (Where the viewer shows each piece of the driver's interface.)
        let mut out = targets_mapped(interactables, |r| layout.driver_rect_to_window(r), layout.window());
        // What lies in a part of the interface drawn elsewhere by the layout
        // (the corner's buttons, a popup moved into a view) is there, unless
        // the viewer arranged the interface itself (then the pieces as shown
        // say so).
        if layout.interface_shown.is_none() {
            for t in &mut out {
                let c = t.center();
                let moved = layout.moved_parts().iter().find(|([x, y, w, h], (dx, dy))| {
                    let (ox, oy) = ((x - dx) as f64, (y - dy) as f64);
                    c.0 >= ox && c.1 >= oy && c.0 < ox + *w as f64 && c.1 < oy + *h as f64
                });
                if let Some((_, (dx, dy))) = moved {
                    t.rect[0] += *dx as f64;
                    t.rect[1] += *dy as f64;
                    t.area[0] += *dx as f64;
                    t.area[1] += *dy as f64;
                    if let Some(at) = &mut t.anchor {
                        at.0 += *dx as f64;
                        at.1 += *dy as f64;
                    }
                    match &mut t.kind {
                        TargetKind::List { scroll: rect, .. } | TargetKind::Slider { track: rect, .. } => {
                            rect[0] += *dx as f64;
                            rect[1] += *dy as f64;
                        }
                        _ => {}
                    }
                }
            }
        }
        clip_targets(&mut out, |target| {
            interactables.iter().find(|item| item.id == target.id)
                .and_then(|item| layout.driver_target_clip(item.rect))
        });
        // The first queued report's button (`Layout::queued_targets`): the
        // viewer's own, over everything of the driver's.
        for (k, [x, y, w, h]) in layout.queued_targets().into_iter().enumerate() {
            let rect = [x as f64, y as f64, w as f64, h as f64];
            out.push(Target { rect, area: rect, spread: false, anchor: None, kind: TargetKind::Button, id: QUEUED_ID - k as u64, alt_right_click: false });
        }
        // Each view's attractor (A's and B's in a split), the capturing ones
        // first; the first keeps the one id of old.
        for (k, a) in layout.attractors().into_iter().enumerate() {
            out.push(Target { id: CAPTURE_ID - k as u64, ..Target::capture(a.rect, a.at) });
        }
        out
    }

    /// The actions as the driver gets them; a click on an attractor ends
    /// the mode and goes back to playing in its view (the mouse captured
    /// there where it may be), nothing going to the driver.
    fn perform(&mut self, actions: Vec<Action>, layout: &mut Layout) -> Vec<Input> {
        let mut out = Vec::new();
        for a in actions {
            match a {
                Action::Capture(p) => {
                    self.snapper.stop();
                    let p = (p.0.round() as i32, p.1.round() as i32);
                    layout.attractor_clicked(p);
                    log::debug!("snap: an attractor clicked: off, the mouse {} at {p:?}", if layout.captured() { "captured" } else { "free" });
                }
                // On the queued reports, the viewer's own: the first is read.
                Action::Button { button, down, at } if layout.queued_click(button, down, (at.0.round() as i32, at.1.round() as i32)).is_some_and(|inputs| { out.extend(inputs); true }) => {}
                // A press on the menu outside the view the mouse is captured
                // in (beside the menu) lets the mouse go; it goes to the menu.
                Action::Button { down: true, at, .. } => {
                    layout.menu_clicked((at.0.round() as i32, at.1.round() as i32));
                    out.extend(to_input(a, layout));
                }
                a => out.extend(to_input(a, layout)),
            }
        }
        out.extend(self.sync(layout));
        out
    }

    /// The layout told whether the snapping pointer is the pointer, the
    /// system pointer hidden as long as it is; where it parked, the pointer
    /// is (with the mouse free the system pointer shows there).
    fn sync(&mut self, layout: &mut Layout) -> Vec<Input> {
        // Landed on an element after the mode: the pointer is there, under
        // the usual rules (the system pointer, the mouse let go if it was
        // captured).
        if let Some(p) = self.snapper.take_parked() {
            let p = (p.0.round() as i32, p.1.round() as i32);
            if layout.captured() {
                layout.let_go_at(p);
            } else {
                layout.place_pointer(p);
            }
            self.snapper.stop();
        }
        let on = self.active();
        if on == layout.snapping() {
            return vec![];
        }
        self.hide_pointer = Some(on);
        layout.set_snapping(on)
    }

    /// Turns the mode on.
    fn enter(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let at = self.snapper.pointer().unwrap_or_else(|| {
            let (x, y) = layout.pointer_in_window();
            (x as f64, y as f64)
        });
        self.snapper.enter(now, at, &self.shown_targets(scene, layout), rates);
        log::debug!("viewer: button snapping on (the mouse {})", if layout.captured() { "captured" } else { "free" });
        self.sync(layout)
    }

    fn slider_preview(&mut self) -> Vec<Input> {
        let Some(edit) = &mut self.slider else { return vec![] };
        if !std::mem::take(&mut edit.dirty) { return vec![] }
        vec![Input::Slider { id: edit.id, action: SliderAction::Preview, value: edit.value.round() as i32 }]
    }

    fn end_slider(&mut self, action: SliderAction) -> Vec<Input> {
        let mut out = if action == SliderAction::Commit { self.slider_preview() } else { vec![] };
        if let Some(edit) = self.slider.take() {
            out.push(Input::Slider { id: edit.id, action, value: edit.value.round() as i32 });
            // Editing motion never becomes a delayed navigation or list scroll.
            self.snapper.moved = (0.0, 0.0);
            self.snapper.scroll = Scroller::default();
        }
        out
    }

    fn validate_slider(&mut self, targets: &[Target]) -> Vec<Input> {
        let Some(edit) = &mut self.slider else { return vec![] };
        if let Some(Target { kind: TargetKind::Slider { track, value }, .. }) = targets.iter().find(|t| t.id == edit.id) {
            edit.track = *track;
            if edit.resync.is_some_and(|before| before != *value) {
                (edit.value, edit.dirty, edit.resync) = ((*value).clamp(0, 1_000_000) as f64, false, None);
            }
            vec![]
        } else {
            self.end_slider(SliderAction::Cancel)
        }
    }

    fn begin_slider(&mut self, target: Target) -> Vec<Input> {
        let TargetKind::Slider { track, value } = target.kind else { return vec![] };
        let at = self.snapper.flight.to;
        self.snapper.flight = Flight::rest(at);
        self.snapper.pos = at;
        self.snapper.arrived = true;
        self.snapper.rested = Some(target.id);
        self.snapper.ghost = Some(at);
        self.snapper.moved = (0.0, 0.0);
        self.slider = Some(SliderEdit { id: target.id, track, value: value.clamp(0, 1_000_000) as f64, dirty: false, resync: None });
        self.selected = Some(target.id);
        vec![Input::Slider { id: target.id, action: SliderAction::Begin, value }]
    }

    /// Turns the mode off (the pointer lands).
    fn leave(&mut self, layout: &mut Layout, why: &str) -> Vec<Input> {
        if !self.snapper.snapping() {
            return vec![];
        }
        let mut out = self.end_slider(SliderAction::Cancel);
        let actions = self.snapper.leave();
        log::debug!("viewer: button snapping off ({why})");
        out.extend(self.perform(actions, layout));
        out
    }

    /// The slider arrow keys step: the Alt mode's slider edit, else (in
    /// the mode) the slider snapped to, else the one last interacted with
    /// ([`SnapInput::selected`]) while it is still shown. Up and Left step
    /// down, Down and Right up, Shift ten steps at a time.
    fn keyboard_slider(&self, targets: &[Target]) -> Option<u64> {
        let slider = |id: u64| targets.iter().any(|t| t.id == id && matches!(t.kind, TargetKind::Slider { .. }));
        if let Some(edit) = &self.slider {
            return Some(edit.id);
        }
        if self.snapper.snapping() {
            return self.snapper.target_id().filter(|&id| slider(id));
        }
        self.selected.filter(|&id| slider(id))
    }

    /// The slider arrow keys step now (it is shown hovered: `looks`).
    pub fn selected_slider(&self) -> Option<u64> {
        self.shown
    }

    /// Window bounds for a keyboard focus border when the driver has no
    /// hover art for the selected slider.
    pub fn keyboard_focus_rect(&self, scene: &scene::Scene, layout: &Layout) -> Option<[i32; 4]> {
        let id = self.selected_slider()?;
        if scene.looks.iter().filter(|looks| looks.id == id).any(|looks| {
            let hover = scene::LookState { pointer: scene::PointerState::Hover, selected: looks.shown.selected };
            looks.views.iter().any(|view| view.state(hover).is_some_and(|sprites| !sprites.is_empty()))
        }) {
            return None;
        }
        let targets = self.shown_targets(scene, layout);
        let slider = targets.iter().find(|target| target.id == id && matches!(target.kind, TargetKind::Slider { .. }))?;
        let [x, y, w, h] = slider.rect;
        let (x0, y0, x1, y1) = (x.floor() as i32, y.floor() as i32, (x + w).ceil() as i32, (y + h).ceil() as i32);
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// The selection goes once its slider is no longer shown (its tab or
    /// screen changed, it was closed).
    fn validate_selection(&mut self, targets: &[Target]) {
        if self.selected.is_some_and(|id| !targets.iter().any(|t| t.id == id && matches!(t.kind, TargetKind::Slider { .. }))) {
            self.selected = None;
        }
    }

    /// Arrow keys on the selected slider (and Esc deselecting it): `None`
    /// if the key is not for that.
    fn keyboard(&mut self, input: &Input, targets: &[Target]) -> Option<Vec<Input>> {
        let Input::Key { vk, down, .. } = *input else { return None };
        if vk == VK_ESCAPE && down {
            // (In the mode Esc also ends the mode, as before.)
            let deselect = self.selected.take().is_some() && !self.snapper.snapping();
            if deselect {
                self.esc = true;
                return Some(vec![]);
            }
            return None;
        }
        if !(VK_LEFT..=VK_DOWN).contains(&vk) {
            return None;
        }
        if !down {
            let k = self.stepped_keys.iter().position(|&k| k == vk)?;
            self.stepped_keys.remove(k);
            return Some(vec![]);
        }
        // (Ctrl+Shift+arrows pan a view; Alt+arrow is a shortcut.)
        if self.ctrl || self.alt {
            return None;
        }
        let id = self.keyboard_slider(targets)?;
        let mut out = self.validate_slider(targets);
        let steps = if self.shift { 10 } else { 1 } * if matches!(vk, VK_LEFT | VK_UP) { -1 } else { 1 };
        out.extend(self.slider_preview());
        let value = targets.iter().find_map(|t| match t.kind { TargetKind::Slider { value, .. } if t.id == id => Some(value), _ => None });
        if let Some(edit) = self.slider.as_mut().filter(|e| e.id == id) {
            edit.resync = value;
        }
        self.selected = Some(id);
        if !self.stepped_keys.contains(&vk) {
            self.stepped_keys.push(vk);
        }
        out.push(Input::Slider { id, action: SliderAction::Step, value: steps });
        Some(out)
    }

    /// A press the driver gets selects the slider it lands on, and
    /// deselects any other; one on the world deselects.
    fn follow_presses(&mut self, out: &[Input], scene: &scene::Scene) {
        for input in out {
            match *input {
                Input::MouseButton { button, down: true, x, y } => {
                    let hit = scene.interactables.iter()
                        .filter(|i| !matches!(i.kind, InteractableKind::List { .. }))
                        .find(|i| { let [ix, iy, w, h] = i.rect; x >= ix && y >= iy && x < ix + w && y < iy + h });
                    let slider = hit.filter(|i| matches!(i.kind, InteractableKind::Slider { .. })).map(|i| i.id);
                    self.selected = if button == Button::Left { slider } else { self.selected.filter(|&id| slider == Some(id)) };
                }
                Input::WorldButton { down: true, .. } => self.selected = None,
                _ => {}
            }
        }
    }

    /// A window input at time `now`: what goes to the driver.
    pub fn route(&mut self, input: Input, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        if let Input::Key { vk: VK_SHIFT, down, .. } = input {
            self.shift = down;
        }
        // (Only keys and presses can select, step or deselect.)
        if !matches!(input, Input::Key { .. } | Input::MouseButton { down: true, .. }) {
            return self.route_input(input, layout, scene, now, rates);
        }
        let targets = self.shown_targets(scene, layout);
        self.validate_selection(&targets);
        // A tab or modal can replace the edited control between animation
        // ticks. Retire that transaction before a key resolves its slider ID.
        let mut out = self.validate_slider(&targets);
        out.extend(match self.keyboard(&input, &targets) {
            Some(out) => out,
            None => self.route_input(input, layout, scene, now, rates),
        });
        self.follow_presses(&out, scene);
        self.shown = self.keyboard_slider(&targets);
        out
    }

    fn route_input(&mut self, input: Input, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let targets = self.shown_targets(scene, layout);
        let mut before = self.validate_slider(&targets);
        // Scene/layout publication can precede the next animation tick.
        // Revalidate a new gesture now; releases retain their press identity.
        if self.snapper.holds_pointer() && matches!(input, Input::MouseButton { down: true, .. } | Input::Wheel { .. }) {
            self.snapper.track(&targets, now);
        }
        if let Input::MouseButton { button, down, .. } = input {
            let b = button_index(button);
            if !down && std::mem::take(&mut self.swallowed_buttons[b]) {
                return before;
            }
            if down && self.slider.is_some() {
                self.swallowed_buttons[b] = true;
                before.extend(self.end_slider(if button == Button::Right { SliderAction::Cancel } else { SliderAction::Commit }));
                return before;
            }
            if down && button == Button::Left && self.snapper.snapping() {
                if let Some(target) = targets.iter().find(|t| Some(t.id) == self.snapper.target_id() && matches!(t.kind, TargetKind::Slider { .. })) {
                    self.swallowed_buttons[b] = true;
                    before.extend(self.begin_slider(*target));
                    return before;
                }
            }
        }
        // The game's C (centre on the active unit), going to the driver:
        // the view following the world cursor centres on the unit too.
        let centre = matches!(input, Input::Key { vk: VK_C, down: true, .. }) && !self.ctrl && !self.alt;
        let out = match input {
            // (Held, it repeats.)
            Input::Key { vk: VK_MENU, down: true, .. } if self.alt => vec![],
            Input::Key { vk: VK_MENU, down: true, .. } => {
                log::debug!("snap: Alt pressed at {now:.3} s (the mode {}, the mouse {})", if self.snapper.snapping() { "on" } else { "off" }, if layout.captured() { "captured" } else { "free" });
                self.alt = true;
                self.alt_entered = false;
                self.alt_sent = false;
                if self.snapper.snapping() {
                    before.extend(self.leave(layout, "Alt"));
                    return before;
                }
                if self.ctrl {
                    log::debug!("viewer: Alt with Ctrl held: the driver's");
                    self.alt_sent = true;
                    before.extend(layout.route(input));
                    return before;
                }
                // (Nothing of the driver's to snap to, e.g. before its first
                // screen: Alt is the driver's.)
                if !self.shown_targets(scene, layout).iter().any(|t| !t.is_capture()) {
                    log::debug!("snap: nothing to snap to: Alt is the driver's");
                    self.alt_sent = true;
                    before.extend(layout.route(input));
                    return before;
                }
                self.alt_entered = true;
                self.enter(layout, scene, now, rates)
            }
            Input::Key { vk: VK_MENU, down: false, .. } => {
                self.alt = false;
                if std::mem::take(&mut self.alt_sent) { layout.route(input) } else { vec![] }
            }
            // On by itself for a menu, Esc is the menu's (it closes it, as
            // without the mode); the mode goes with the last menu.
            Input::Key { vk: VK_ESCAPE, .. } if self.by_menu && layout.menu_open() => layout
                .route(input)
                .into_iter()
                .map(|i| match i {
                    Input::WorldKey { vk, down, text, .. } => Input::Key { vk, down, text },
                    i => i,
                })
                .collect(),
            Input::Key { vk: VK_ESCAPE, down: true, .. } if self.snapper.snapping() => {
                self.esc = true;
                self.leave(layout, "Esc")
            }
            Input::Key { vk: VK_ESCAPE, down: false, .. } if self.esc => {
                self.esc = false;
                vec![]
            }
            Input::Key { vk: VK_CONTROL, down, .. } => {
                self.ctrl = down;
                // Ctrl lets go of the mouse: the mode ends first.
                let mut out = if down { self.leave(layout, "Ctrl") } else { vec![] };
                out.extend(layout.route(input));
                out
            }
            // Another key with Alt held: a shortcut. The driver gets Alt
            // first; the mode this Alt turned on is off again.
            Input::Key { down: true, .. } if self.alt && !self.alt_sent => {
                self.alt_sent = true;
                let mut out = Vec::new();
                if std::mem::take(&mut self.alt_entered) && self.snapper.snapping() {
                    before.extend(self.end_slider(SliderAction::Cancel));
                    self.snapper.stop();
                    log::debug!("viewer: Alt with a key: a shortcut, no button snapping");
                    out.extend(self.sync(layout));
                }
                out.extend(layout.route(Input::Key { vk: VK_MENU, down: true, text: None }));
                out.extend(layout.route(input));
                out
            }
            // (The system pointer's own motion: not while the snapping
            // pointer is the pointer.)
            Input::MouseMove { .. } if self.active() => vec![],
            Input::MouseButton { button, down, x, y } => match self.snapper.click(button, down) {
                Some(actions) => {
                    // (An attractor's click is the viewer's: its release
                    // goes nowhere either.)
                    if down && actions.iter().any(|a| matches!(a, Action::Capture(_))) {
                        self.swallowed_buttons[button_index(button)] = true;
                    }
                    self.perform(actions, layout)
                }
                None => layout.route(Input::MouseButton { button, down, x, y }),
            },
            Input::Wheel { .. } if self.slider.is_some() => vec![],
            Input::Wheel { delta, .. } if !self.ctrl => match self.snapper.wheel(delta) {
                Some(actions) => self.perform(actions, layout),
                None => layout.route(input),
            },
            // Keys while the snapping pointer is the pointer: the interface's
            // (never the world's, whatever the world cursor is over).
            Input::Key { .. } if self.active() => layout
                .route(input)
                .into_iter()
                .map(|i| match i {
                    Input::WorldKey { vk, down, text, .. } => Input::Key { vk, down, text },
                    i => i,
                })
                .collect(),
            Input::Quit => {
                before.extend(self.end_slider(SliderAction::Cancel));
                layout.route(input)
            }
            other => layout.route(other),
        };
        if centre && out.iter().any(|i| matches!(i, Input::Key { vk: VK_C, down: true, .. } | Input::WorldKey { vk: VK_C, down: true, .. })) {
            layout.centre_on_focus(scene.focus);
        }
        before.extend(out);
        before
    }

    /// Raw mouse motion: `None` if it is the layout's (as without
    /// snapping), else what goes to the driver.
    pub fn motion(&mut self, d: (f64, f64)) -> Option<Vec<Input>> {
        if let Some(edit) = &mut self.slider {
            let vertical = edit.track[3] > edit.track[2];
            let (motion, span) = if vertical { (d.1, edit.track[3]) } else { (d.0, edit.track[2]) };
            if motion.is_finite() && span.is_finite() && span > 0.0 {
                let value = (edit.value + motion * SLIDER_GAIN * 1_000_000.0 / span).clamp(0.0, 1_000_000.0);
                edit.dirty |= edit.value.round() != value.round();
                edit.value = value;
            }
            return Some(vec![]);
        }
        self.snapper.motion(d).then(Vec::new)
    }

    /// Time `now`: the pointer moves on; what goes to the driver.
    pub fn tick(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let mut out = self.menu_mode(layout, scene, now, rates);
        out.extend(self.tick_mode(layout, scene, now, rates));
        // (Drawn while the mode is on and the mouse free.)
        self.attractors = if self.active() && !layout.captured() {
            layout.attractors().into_iter().map(|a| ((a.at.0 as f64, a.at.1 as f64), a.captures)).collect()
        } else {
            Vec::new()
        };
        out
    }

    /// Whether the mode comes on by itself while a menu is open
    /// (`viewer_settings::Input::alt_in_menus`; false while the window does
    /// not have the focus, so that nothing holds the pointer of a window in
    /// the background).
    pub fn set_alt_in_menus(&mut self, on: bool) {
        self.alt_in_menus = on;
    }

    /// The attractors to draw: window pixels, and whether a click there
    /// captures the mouse.
    pub fn attractor_points(&self) -> &[(Point, bool)] {
        &self.attractors
    }

    /// With [`SnapInput::set_alt_in_menus`]: a menu opening turns the mode
    /// on (where it has something to snap to), the last one closing turns
    /// it off again, a mode the player had turned on stays as it is. The
    /// player turning it off (Alt, Ctrl, an attractor, focus, capturing the
    /// mouse beside the menu) keeps it off
    /// until the menus are all closed.
    fn menu_mode(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        let menu = layout.menu_open();
        if self.by_menu && !self.snapper.snapping() {
            self.by_menu = false;
            self.menu_declined = menu;
            log::debug!("snap: the mode a menu turned on was turned off");
        }
        if !menu {
            self.menu_declined = false;
            if std::mem::take(&mut self.by_menu) {
                return self.leave(layout, "the menus closed");
            }
            return vec![];
        }
        // The player captured the mouse beside the menu (a click into a
        // view the menu is not over): as the mode turned off, not on again
        // by itself until the menus are all closed.
        if layout.captured() && !self.snapper.snapping() && !self.menu_declined {
            self.menu_declined = true;
            log::debug!("snap: the mouse captured beside a menu: no button snapping by itself");
        }
        if self.alt_in_menus && !self.by_menu && !self.menu_declined && !self.snapper.snapping() && !self.alt && self.shown_targets(scene, layout).iter().any(|t| !t.is_capture()) {
            self.by_menu = true;
            log::debug!("snap: a menu is open: button snapping on by itself");
            return self.enter(layout, scene, now, rates);
        }
        vec![]
    }

    fn tick_mode(&mut self, layout: &mut Layout, scene: &scene::Scene, now: f64, rates: Rates) -> Vec<Input> {
        // A command given in the mode (its button clicked, or its key)
        // waits for a place of the world: the next click is the world's,
        // so the mode ends and the mouse is captured for it.
        let targeting = std::mem::replace(&mut self.targeting, scene.interface.targeting);
        if scene.interface.targeting && !targeting && self.snapper.snapping() {
            return self.capture_for_target(layout);
        }
        let targets = self.shown_targets(scene, layout);
        let mut out = self.validate_slider(&targets);
        self.validate_selection(&targets);
        self.shown = self.keyboard_slider(&targets);
        if self.slider.is_some() {
            self.snapper.last = now;
            out.extend(self.slider_preview());
            return out;
        }
        let actions = self.snapper.tick(now, &targets, rates);
        out.extend(self.perform(actions, layout));
        self.shown = self.keyboard_slider(&targets);
        out
    }

    /// The mode ends at once for a command of the driver's that waits for
    /// a place of the world: the mouse is captured again in the view
    /// following the world cursor, the view and the world cursor staying
    /// put, the tile under it the command's target at once
    /// (`Layout::capture_for_target`). Nothing is clicked: a press still
    /// held is let go on the element it went to.
    fn capture_for_target(&mut self, layout: &mut Layout) -> Vec<Input> {
        let mut out = self.end_slider(SliderAction::Cancel);
        self.snapper.stop();
        let target = layout.capture_for_target();
        log::debug!("snap: a command waits for a place of the world: off, {}", if target.is_some() { "the mouse captured again" } else { "no view to capture the mouse in" });
        out.extend(self.sync(layout));
        out.extend(target.into_iter().flatten());
        out
    }

    /// The window lost the keyboard: no key is held any more, the mode ends.
    pub fn focus_lost(&mut self, layout: &mut Layout) -> Vec<Input> {
        let mut out = self.end_slider(SliderAction::Cancel);
        self.snapper.stop();
        if self.alt_sent {
            out.extend(layout.route(Input::Key { vk: VK_MENU, down: false, text: None }));
        }
        (self.ctrl, self.alt, self.alt_entered, self.alt_sent, self.esc, self.shift) = (false, false, false, false, false, false);
        self.stepped_keys.clear();
        out.extend(self.sync(layout));
        out
    }

    /// Whether to hide the system pointer, when that changes.
    pub fn take_hide_pointer(&mut self) -> Option<bool> {
        self.hide_pointer.take()
    }

    /// The pointers as sprites in camera `overlay` (over the whole window
    /// at 1:1, on top of every view: window pixels): the ghost, then the
    /// pointer over it; the driver's arrow if the description has it. At
    /// the art's own size ([`SnapInput::compose`] at any cursor size).
    pub fn sprites(&self, compiled: &render_desc::Compiled, overlay: u32) -> Vec<tile_render::PlacedSprite> {
        self.compose(compiled, overlay, overlay + 1, crate::cursor::Scale::default(), (0, 0)).sprites
    }

    /// The snapping pointer's art: the driver's own pointer (`driver`, a
    /// sprite set with pixels; `current`: it answers the last pointer move
    /// sent to the driver, `scene::Scene::cursor_at`) while the pointer
    /// rests on one of its elements and the driver has handled the move
    /// there, so that it tells
    /// what a click does as the driver's pointer would (the game's
    /// Civilopedia book over an entry a click opens the Civilopedia on);
    /// else, and for a system icon the viewer has no art of, the driver's
    /// arrow (else the viewer's own pointer). In flight, and at rest until
    /// the driver has handled the move there, it is the arrow: the driver's
    /// pointer is still the one for where it was before, never shown where
    /// it does not belong.
    pub fn pointer_set(&self, compiled: &render_desc::Compiled, driver: scene::Cursor, current: bool) -> Option<tile_render::SpriteSetId> {
        let arrow = || [ARROW_SPRITE, POINTER_SPRITE, crate::CURSOR_SPRITE].iter().find_map(|n| compiled.set(n).map(|(s, _)| s));
        let answered = self.snapper.resting_on_driver() && current;
        let own = match driver {
            scene::Cursor::Sprite(raw) if answered => Some(tile_render::SpriteSetId::from_raw(raw)),
            _ => None,
        };
        own.filter(|&s| crate::cursor::SetArt::of(compiled, s).is_some()).or_else(arrow)
    }

    /// The pointers at the cursor's size `scale`, as the viewer's cursor is
    /// drawn (`crate::cursor::compose`: the same whole canvas pixels and hot
    /// spot): 1:1 in camera `overlay`, else each in a camera of its own,
    /// numbered from `next` on (cameras to add after the others on top),
    /// as long as the renderer draws that many views. `window`: the
    /// canvas.
    pub fn compose(&self, compiled: &render_desc::Compiled, overlay: u32, next: u32, scale: crate::cursor::Scale, window: (u32, u32)) -> crate::cursor::Composed {
        self.compose_with_busy(compiled, &crate::cursor::Art::default(), overlay, next, scale, window, crate::host_status::Responsiveness::Answering, (scene::Cursor::Default, false))
    }

    /// As [`SnapInput::compose`], with the busy badge for `status` on the
    /// pointer, and `driver` the pointer the driver shows (as
    /// `crate::cursor::Art::shown` makes it) and whether it answers the
    /// last pointer move sent to the driver: while the pointer rests on one
    /// of the driver's elements and the driver has handled the move there,
    /// that pointer's art ([`SnapInput::pointer_set`]).
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_busy(&self, compiled: &render_desc::Compiled, pixels: &crate::cursor::Art, overlay: u32, next: u32, scale: crate::cursor::Scale, window: (u32, u32), status: crate::host_status::Responsiveness, driver: (scene::Cursor, bool)) -> crate::cursor::Composed {
        let mut out = crate::cursor::Composed::default();
        let Some(pointer) = self.snapper.pointer() else { return out };
        let set = |names: &[&str]| names.iter().find_map(|n| compiled.set(n).map(|(s, _)| s));
        let pointer_set = self.pointer_set(compiled, driver.0, driver.1);
        // (Under the pointer it would not be seen.)
        let ghost = self.snapper.ghost().filter(|g| (g.0 - pointer.0).hypot(g.1 - pointer.1) >= 2.0).map(|g| (set(&[ARROW_GHOST_SPRITE, GHOST_SPRITE]), g));
        let solid = Some((pointer_set, pointer));
        // Under them, the attractors: the translucent crosshair (a click
        // there goes back into the game), in each view's middle.
        let prefixed = |prefix: &str| compiled.sets_with_prefix(prefix).map(|(_, s)| s).next();
        let attractor = prefixed(ATTRACTOR_SPRITES).or_else(|| prefixed(crate::cursor::CAPTURE_SPRITES)).or_else(|| set(&[ARROW_GHOST_SPRITE, GHOST_SPRITE]));
        let attractors = self.attractors.iter().map(|&(at, _)| (attractor, at));
        let reserve = |is_attractor: bool| if is_attractor { 2 } else { 0 };
        for (k, (set, (x, y))) in attractors.chain(ghost).chain(solid).enumerate() {
            let Some(art) = set.and_then(|set| crate::cursor::SetArt::of(compiled, set)) else { continue };
            let marks = crate::cursor::Marks { window: vec![(x.round() as i32, y.round() as i32)], ..Default::default() };
            let c = crate::cursor::compose(art, &marks, &[], &[], scale, window, overlay, (0, 0));
            // Its own cameras after those taken already (an attractor only
            // where the ghost and the pointer still find theirs).
            let first = next + out.top.len() as u32;
            if !c.top.is_empty() && (first as usize + c.top.len() + reserve(k < self.attractors.len())) > tile_render::MAX_VIEWS {
                continue;
            }
            out.sprites.extend(c.sprites.into_iter().map(|mut s| {
                if let Some(v) = s.view.filter(|&v| v != overlay) {
                    s.view = Some(v - (overlay + 1) + first);
                }
                s
            }));
            out.top.extend(c.top);
        }
        if status != crate::host_status::Responsiveness::Answering {
            // (Placed on the solid pointer as on the system cursor.)
            let badge = pointer_set
                .and_then(|set| crate::cursor::SetArt::of(compiled, set))
                .and_then(|art| pixels.badge_for(art.set.raw(), art.size, art.hotspot));
            if let Some(badge) = badge {
                crate::cursor::compose_busy_badge(&mut out, compiled, (pointer.0.round() as i32, pointer.1.round() as i32), badge, None, scale, window, next);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "snap/content_anchors.rs"]
mod content_anchors;

#[cfg(test)]
#[path = "snap/sliders.rs"]
mod sliders;

#[cfg(test)]
#[path = "snap/status_controls.rs"]
mod status_controls;

#[cfg(test)]
#[path = "snap/targeting.rs"]
mod targeting;

#[cfg(test)]
#[path = "snap/menus.rs"]
mod menus;

#[cfg(test)]
#[path = "snap/cursor_size.rs"]
mod cursor_size;

#[cfg(test)]
#[path = "snap/city_work.rs"]
mod city_work;

#[cfg(test)]
#[path = "snap/regions.rs"]
mod regions;

#[cfg(test)]
#[path = "snap/queued_reports.rs"]
mod queued_reports;
