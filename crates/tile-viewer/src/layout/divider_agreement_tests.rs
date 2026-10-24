// Included into `layout::tests`: the dividers as drawn against where the
// pointer is held, hit and scrolled, over many arrangements.

use std::collections::BTreeSet;
use viewer_settings::MapMovement;

/// One arrangement to compare.
#[derive(Clone, Debug)]
struct Case {
    canvas: (u32, u32),
    split: S,
    width: u32,
    pixel_scale: f64,
    zoom: Zoom,
    place: Place,
    orders: bool,
    across: bool,
    movement: MapMovement,
    /// The interface's scale (its orders and screens that much larger).
    ui: i32,
    glass: bool,
}

fn cases(split: S, glass: bool) -> Vec<Case> {
    let mut out = Vec::new();
    for canvas in [(800, 600), (1023, 767)] {
        for width in [1, 4, 13] {
            for pixel_scale in [1.0, 2.0] {
                for (zoom, place) in [(Zoom::Off, Place::Floating), (Zoom::Floating131, Place::Floating), (Zoom::TopStrip, Place::Floating), (Zoom::TopStrip, Place::LeftOrBottom), (Zoom::TopStrip, Place::RightOrTop)] {
                    for (orders, across) in [(false, false), (true, false), (true, true)] {
                        for movement in [MapMovement::EdgeScroll, MapMovement::Drag] {
                            for ui in [1, 2] {
                                out.push(Case { canvas, split, width, pixel_scale, zoom, place, orders, across, movement, ui, glass });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// The layout of `c` with one frame drawn, and that frame.
fn layout_of(c: &Case) -> (Layout, Frame) {
    let mut l = Layout::default();
    l.apply_settings(&settings(|s| {
        s.split = c.split;
        s.separator_width = c.width;
        s.zoom = c.zoom;
        s.zoom_place = c.place;
        glass_dividers_setting(s, c.glass);
    }));
    l.window_pixel_scale = c.pixel_scale;
    l.set_map_movement(c.movement);
    let mut s = scene();
    let (w, h) = (c.canvas.0 as i32, c.canvas.1 as i32);
    let vertical = c.split != S::Horizontal1To1;
    // The unit's orders: low in the window, over the gap where it is.
    if c.orders {
        s.interface.reachable = Some(if vertical { [w / 3, h - 40 * c.ui - 20, w / 3 + 30 * c.ui, 30 * c.ui] } else { [w - 70 * c.ui - 10, h / 3, 60 * c.ui, h / 3] });
    }
    // A screen of the driver's across the divider (and the strip) high up.
    let interface = if c.across { vec![if vertical { [w / 4, h / 9, w / 2, 40 * c.ui] } else { [20, h / 4, 50 * c.ui, h / 2] }] } else { vec![] };
    let f = l.frame(&s, c.canvas, (0, 0, 0, 0), interface, BOUNDS);
    (l, f)
}

fn pixels(rects: impl IntoIterator<Item = [i32; 4]>) -> BTreeSet<(i32, i32)> {
    let mut out = BTreeSet::new();
    for [x, y, w, h] in rects {
        for py in y..y + h {
            for px in x..x + w {
                out.insert((px, py));
            }
        }
    }
    out
}

/// Where the interface lies (what is drawn under it is not seen).
fn covered(l: &Layout, p: (i32, i32)) -> bool {
    l.interface.iter().chain(&l.driver_rects).any(|&r| inside(r, p)) || l.orders_shown().is_some_and(|r| inside(r, p))
}

/// Whether a left-to-right (top-to-bottom) move across the split's gap at
/// `along`, from `back` pixels before it to as far after it, gets across.
fn crosses(l: &mut Layout, along: i32, back: i32) -> bool {
    let Some(((g0, g1), _)) = l.gap() else { return true };
    let vertical = l.split.is_some_and(|s| s.axis == Axis::Vertical);
    let at = |v: i32| if vertical { (v, along) } else { (along, v) };
    let (from, to) = (at(g0 - back), at(g1 - 1 + back));
    l.pointer = from;
    l.raw_pointer = from;
    l.side = Some(Side::A);
    l.against = None;
    mv(l, to);
    l.pointer == to
}

/// Every way the drawn dividers and the pointer's boundaries disagree in
/// case `c` (empty: they agree).
fn disagreements(c: &Case) -> Vec<String> {
    let (mut l, f) = layout_of(c);
    let mut out = Vec::new();
    let drawn: BTreeSet<(i32, i32)> = pixels(drawn_dividers(&l, &f)).into_iter().filter(|&p| !covered(&l, p)).collect();
    // The walls the system holds the pointer out of, bar the zoom strip's
    // own area (a reserved zoom, not a divider).
    let strip = l.strip().map(|(r, _)| r);
    let walls: BTreeSet<(i32, i32)> = pixels(l.pointer_walls().unwrap_or_default()).into_iter()
        .filter(|&p| !strip.is_some_and(|r| inside(r, p)) && !covered(&l, p)).collect();
    let free_everywhere = l.pointer_free_everywhere();
    if !free_everywhere {
        // (Told apart: the split's gap, and the rest, the strip's divider.)
        let on_split = |p: &(i32, i32)| l.gap().is_some_and(|((g0, g1), _)| {
            let v = if l.split.is_some_and(|s| s.axis == Axis::Vertical) { p.0 } else { p.1 };
            v >= g0 && v < g1
        });
        for (name, split) in [("split", true), ("strip", false)] {
            let drawn: BTreeSet<_> = drawn.iter().filter(|p| on_split(p) == split).copied().collect();
            let walls: BTreeSet<_> = walls.iter().filter(|p| on_split(p) == split).copied().collect();
            let only_drawn: Vec<_> = drawn.difference(&walls).take(3).collect();
            let only_walls: Vec<_> = walls.difference(&drawn).take(3).collect();
            if !only_drawn.is_empty() || !only_walls.is_empty() {
                out.push(format!("[drawn vs walls, {name} divider] drawn {} px, walls {} px: drawn but no wall at {only_drawn:?} ({} px), wall but not drawn at {only_walls:?} ({} px)",
                    drawn.len(), walls.len(), drawn.difference(&walls).count(), walls.difference(&drawn).count()));
            }
        }
    }
    // Moving across the split's gap: held exactly where it is drawn.
    if let (Some(((g0, _), (s0, s1))), false) = (l.gap(), free_everywhere) {
        let vertical = l.split.is_some_and(|s| s.axis == Axis::Vertical);
        let at = |along: i32| if vertical { (g0, along) } else { (along, g0) };
        // Every row within a few of where the drawing or the walls change,
        // and every seventh in between.
        let changes: Vec<i32> = (s0 + 1..s1).filter(|&a| drawn.contains(&at(a)) != drawn.contains(&at(a - 1)) || walls.contains(&at(a)) != walls.contains(&at(a - 1))).collect();
        let rows: Vec<i32> = (s0..s1).filter(|&a| a % 7 == 0 || changes.iter().any(|&c| (a - c).abs() <= 6)).collect();
        for back in [3, 60] {
            let mut wrong = Vec::new();
            for &along in &rows {
                let p = at(along);
                if covered(&l, p) || strip.is_some_and(|r| inside(r, p)) {
                    continue;
                }
                let held_at = drawn.contains(&p);
                if crosses(&mut l, along, back) == held_at {
                    wrong.push(along);
                }
            }
            if !wrong.is_empty() {
                out.push(format!("[crossing {back} px] a move {back} px across the gap: {} rows where it is {} contrary to the drawing, first {:?}",
                    wrong.len(), if drawn.contains(&at(wrong[0])) { "let through" } else { "held" }, &wrong[..wrong.len().min(4)]));
            }
        }
    }
    // Pushing up at the first row below a zoom strip scrolls up.
    if let Some((s, _)) = l.strip() {
        l.against = None;
        let (a, b) = l.panes();
        for pane in std::iter::once(a).chain(b) {
            let x = pane[0] + pane[2] / 2;
            // (The panes right below it; not B below A.)
            if x < s[0] || x >= s[0] + s[2] || pane[1] > s[1] + s[3] + 32 {
                continue;
            }
            let p = (x, pane[1]);
            match l.view_edge(p) {
                Some((_, e)) if e.1 < 0 => {}
                e => out.push(format!("[strip edge scroll] pushing up at {p:?}, the first row below the strip (its divider to {}): {e:?}, no upward edge scroll", s[1] + s[3] + l.separator_gap())),
            }
        }
    }
    // No pixel of a drawn divider counts as being in a view (every fifth).
    let in_view: Vec<_> = drawn.iter().step_by(5).filter(|&&p| matches!(l.hit(p), Hit::Main | Hit::Pane) || l.side_of(p).is_some() && l.gap().is_some_and(|((g0, g1), _)| {
        let v = if l.split.is_some_and(|s| s.axis == Axis::Vertical) { p.0 } else { p.1 };
        v >= g0 && v < g1
    })).take(3).collect();
    if !in_view.is_empty() {
        out.push(format!("[hit test] drawn divider pixels hit as a view: {in_view:?}"));
    }
    out
}

fn agree(split: S) {
    // (class, arrangements, first examples)
    let mut classes: Vec<(String, usize, Vec<String>)> = Vec::new();
    let mut failing = 0;
    let mut total = 0;
    for glass in [false, true] {
        for c in cases(split, glass) {
            total += 1;
            let d = disagreements(&c);
            failing += usize::from(!d.is_empty());
            for message in d {
                let class = message[..message.find(']').map_or(0, |i| i + 1)].to_string();
                let i = classes.iter().position(|k| k.0 == class).unwrap_or_else(|| {
                    classes.push((class, 0, Vec::new()));
                    classes.len() - 1
                });
                classes[i].1 += 1;
                if classes[i].2.len() < 3 {
                    classes[i].2.push(format!("{c:?}\n        {message}"));
                }
            }
        }
    }
    let report: Vec<String> = classes.iter().map(|(class, n, examples)| format!("{class} in {n} arrangements, e.g.\n      {}", examples.join("\n      "))).collect();
    assert!(failing == 0, "{failing} of {total} arrangements disagree:\n  {}", report.join("\n  "));
}

#[test]
fn the_drawn_dividers_are_where_the_pointer_is_held_hit_and_scrolled_side_by_side() {
    agree(S::Vertical1To1);
}

#[test]
fn the_drawn_dividers_are_where_the_pointer_is_held_hit_and_scrolled_two_to_one() {
    agree(S::Vertical2To1);
}

#[test]
fn the_drawn_dividers_are_where_the_pointer_is_held_hit_and_scrolled_stacked() {
    agree(S::Horizontal1To1);
}

// (As the code stood before the single divider geometry.)
fn glass_dividers_setting(_: &mut Settings, _: bool) {}
fn drawn_dividers(_: &Layout, f: &Frame) -> Vec<[i32; 4]> {
    f.divider.iter().map(|d| d.1).chain(f.separators.iter().map(|s| s.1)).collect()
}
