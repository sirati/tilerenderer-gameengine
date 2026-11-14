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
    // The piece's orders: low in the window, over the gap where it is.
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

fn glass_dividers_setting(s: &mut Settings, glass: bool) {
    s.glass_dividers = glass;
}

/// The pixels the frame draws as dividers: the frosted strips, the strip's
/// separator, or the glass bars where they are shown (their lens within
/// their clip, in the window).
fn drawn_dividers(l: &Layout, f: &Frame) -> Vec<[i32; 4]> {
    let window = [0, 0, l.window.0 as i32, l.window.1 as i32];
    let glass = f.glass_dividers.iter().map(|&(at, _)| {
        let c = f.cameras[at as usize];
        let [x0, y0, x1, y1] = c.target_clip.unwrap();
        let r = [c.origin.0.max(x0).max(0), c.origin.1.max(y0).max(0), 0, 0];
        let right = (c.origin.0 + c.viewport.0 as i32).min(x1).min(window[2]);
        let bottom = (c.origin.1 + c.viewport.1 as i32).min(y1).min(window[3]);
        [r[0], r[1], (right - r[0]).max(0), (bottom - r[1]).max(0)]
    });
    f.divider.iter().map(|d| d.1).chain(f.separators.iter().map(|s| s.1)).chain(glass).collect()
}

/// With glass dividers every segment is a glass bar exactly as thick as its
/// wall, round (its radius half that) and showing that round end only
/// where the divider stops in the open: at the window's edge it runs on
/// past it, against another divider it runs on under it and is cut off at
/// its side.
#[test]
fn glass_bars_are_as_thick_as_their_walls_and_round_only_at_free_ends() {
    let (mut free, mut edge, mut junction) = (0, 0, 0);
    for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
        for c in cases(split, true) {
            let (l, f) = layout_of(&c);
            let window = (l.window.0 as i32, l.window.1 as i32);
            let dividers = l.dividers();
            assert_eq!(f.glass_dividers.len(), dividers.segments.len(), "{c:?}");
            for &(at, s) in &f.glass_dividers {
                let camera = f.cameras[at as usize];
                let lens = camera.effect.map(RoundedFrame::of).unwrap_or_else(|| panic!("{c:?}: no lens"));
                let t = s.band.thickness();
                assert_eq!(t, l.divider_thickness(), "{c:?}");
                assert!(lens.raised && lens.radius == t as f32 / 2.0 && lens.rim == t as f32 / 2.0, "{c:?}: {lens:?}");
                // Across: exactly the wall.
                let (across, along) = if s.band.vertical { ((camera.origin.0, camera.viewport.0), (camera.origin.1, camera.viewport.1)) } else { ((camera.origin.1, camera.viewport.1), (camera.origin.0, camera.viewport.0)) };
                assert_eq!((across.0, across.0 + across.1 as i32), s.band.across, "{c:?}");
                let (a0, a1) = (along.0, along.0 + along.1 as i32);
                let clip = camera.target_clip.unwrap();
                let (c0, c1) = if s.band.vertical { (clip[1], clip[3]) } else { (clip[0], clip[2]) };
                let extent = if s.band.vertical { window.1 } else { window.0 };
                for (k, end) in s.ends.iter().enumerate() {
                    let (lens_end, clip_end, at) = if k == 0 { (a0, c0, s.along.0) } else { (a1, c1, s.along.1) };
                    let outward = |v: i32| if k == 0 { at - v } else { v - at };
                    match end {
                        DividerEnd::Free => {
                            // The round cap inside the segment, shown.
                            free += 1;
                            assert_eq!(lens_end, at, "{c:?}: free end {k} of {s:?}");
                            assert!(outward(clip_end) > 0, "{c:?}: its halo shows");
                            assert!(at > 0 && at < extent, "{c:?}");
                        }
                        DividerEnd::Edge => {
                            // The cap past the window's edge.
                            edge += 1;
                            assert!(at <= 0 || at >= extent, "{c:?}");
                            assert!(outward(lens_end) >= t, "{c:?}: edge end {k} of {s:?}: {lens_end}");
                        }
                        DividerEnd::Junction => {
                            // The cap under the other divider, cut off at its side.
                            junction += 1;
                            assert!(outward(lens_end) * 2 >= t, "{c:?}: junction end {k} of {s:?}");
                            assert_eq!(clip_end, at, "{c:?}: junction end {k} of {s:?}");
                            let other = if s.band.kind == DividerKind::Split { dividers.strip } else { dividers.split }.unwrap();
                            assert!(at == other.across.0 || at == other.across.1, "{c:?}");
                        }
                    }
                }
            }
            // The ones others end against go over them.
            let firsts: Vec<bool> = f.glass_dividers.iter().map(|(_, s)| s.ends.contains(&DividerEnd::Junction)).collect();
            assert!(firsts.windows(2).all(|w| w[0] || !w[1]), "{c:?}: {firsts:?}");
        }
    }
    assert!(free > 100 && edge > 100 && junction > 10, "{free} {edge} {junction}");
}

/// With glass dividers one view's picture goes on under each glass bar
/// (`Dividers::under`, from `Layout::dividers`), and only drawing goes on
/// there: every pixel a bar bends (its lens in the window, run on under
/// the divider it meets too) lies under exactly one stretch, of a view
/// showing the world (no background); the split's under one view (no seam
/// in it), the strip's changing view only under the split's bar, at its
/// far side; the frame draws exactly those stretches, with the pictures of
/// their views, and no gap halves. The views' rectangles and the sides
/// stay out of the gap (the walls: `agree`, with glass too).
#[test]
fn under_the_glass_one_views_picture_goes_on_and_only_the_drawing_extends() {
    let mut checked = (0, 0);
    for split in [S::Vertical1To1, S::Vertical2To1, S::Horizontal1To1] {
        for c in cases(split, true) {
            let (l, f) = layout_of(&c);
            let dividers = l.dividers();
            let window = [0, 0, l.window.0 as i32, l.window.1 as i32];
            let under = &dividers.under;
            assert_eq!(under.iter().filter(|u| u.kind == DividerKind::Split).count(), usize::from(dividers.split.is_some()), "{c:?}");
            assert_eq!(under.is_empty(), dividers.split.is_none() && dividers.strip.is_none(), "{c:?}");
            // Each stretch lies in its divider's band.
            for u in under {
                let band = if u.kind == DividerKind::Split { dividers.split } else { dividers.strip }.unwrap();
                let [bx, by, bw, bh] = band.rect();
                let [x, y, w, h] = u.rect;
                assert!(x >= bx && y >= by && x + w <= bx + bw && y + h <= by + bh && w > 0 && h > 0, "{c:?}: {u:?} in {band:?}");
                assert!(u.side.is_some(), "{c:?}: {u:?} the background under the glass");
            }
            let split_side = under.iter().find(|u| u.kind == DividerKind::Split).and_then(|u| u.side);
            assert!(split_side.is_none() || split_side == Some(Side::A), "{c:?}: the first view under the split's divider");
            // What each bar bends: exactly one stretch's view under every
            // pixel of its lens in the window.
            for (_, bar) in dividers.glass_order() {
                let [x, y, w, h] = bar.rect;
                let (x0, y0) = (x.max(0), y.max(0));
                let (x1, y1) = ((x + w).min(window[2]), (y + h).min(window[3]));
                for py in (y0..y1).step_by(3) {
                    for px in (x0..x1).step_by(3) {
                        let at: Vec<_> = under.iter().filter(|u| inside(u.rect, (px, py))).collect();
                        assert_eq!(at.len(), 1, "{c:?}: {:?} under ({px}, {py}) of {bar:?}", at);
                        checked.0 += 1;
                    }
                }
            }
            // The strip's stretches change view only at the split's far side.
            let strip: Vec<_> = under.iter().filter(|u| u.kind == DividerKind::Strip).collect();
            for pair in strip.windows(2) {
                assert_ne!(pair[0].side, pair[1].side, "{c:?}: merged");
                let at = pair[1].rect[0];
                assert_eq!(Some(at), dividers.split.filter(|s| s.vertical).map(|s| s.across.1), "{c:?}: {strip:?}");
            }
            // Drawn: one camera for each stretch, its view's picture going on.
            for u in under {
                let view = l.shown.iter().find(|(h, c)| *h == if u.side == Some(Side::A) { Hit::Main } else { Hit::Pane } && !c.stacked).unwrap().1;
                let (x, y) = world_at_floor(&view, (u.rect[0], u.rect[1]));
                let drawn = f.cameras.iter().filter(|cam| rect_of(cam) == u.rect && cam.effect.is_none()).collect::<Vec<_>>();
                assert!(drawn.iter().any(|cam| (cam.x, cam.y, cam.zoom, cam.cells) == (x, y, view.zoom, true)), "{c:?}: {u:?} not drawn as its view: {drawn:?}");
                checked.1 += 1;
            }
            assert!(f.divider.is_empty(), "{c:?}: no frost");
            if let Some(((g0, g1), (s0, s1))) = l.gap() {
                let half = (g1 - g0) / 2;
                let vertical = dividers.split.unwrap().vertical;
                let a_half = if vertical { [g0, s0, half, s1 - s0] } else { [s0, g0, s1 - s0, half] };
                assert!(!f.cameras.iter().any(|cam| rect_of(cam) == a_half && half > 0), "{c:?}: no gap halves");
                // Only drawing: the views' rectangles, the walls and the sides
                // are as without glass under it.
                let (a, b) = l.panes();
                for (hit, pane) in [(Hit::Main, a), (Hit::Pane, b.unwrap())] {
                    let Some((_, view)) = l.shown.iter().find(|(h, _)| *h == hit) else { continue };
                    let r = rect_of(view);
                    let (c0, c1) = if vertical { (r[0], r[0] + r[2]) } else { (r[1], r[1] + r[3]) };
                    assert!(c1 <= g0 || c0 >= g1, "{c:?}: {hit:?} at {r:?} reaches into the gap ({pane:?})");
                }
                for along in [(s0 + s1) / 2, s0 + 1, s1 - 2] {
                    for v in [g0, (g0 + g1) / 2, g1 - 1] {
                        let p = if vertical { (v, along) } else { (along, v) };
                        assert_eq!(l.side_of(p), None, "{c:?}: {p:?} in the gap");
                    }
                }
            }
        }
    }
    assert!(checked.0 > 10_000 && checked.1 > 1000, "{checked:?}");
}

/// Without glass dividers nothing goes on under them but the gap halves.
#[test]
fn without_glass_dividers_nothing_goes_on_under_them() {
    for split in [S::Vertical1To1, S::Horizontal1To1] {
        for c in cases(split, false).into_iter().step_by(7) {
            let (l, _) = layout_of(&c);
            assert!(l.dividers().under.is_empty(), "{c:?}");
        }
    }
}

/// The pictures under the glass, drawn (the CPU reference renderer): each
/// stretch shows exactly what its view would show there if the view went
/// on that far, continuing the view's own picture without a seam, and no
/// background anywhere under the bars.
#[test]
fn the_picture_under_the_glass_continues_its_views_picture() {
    use tile_render::{AtlasBuilder, Grid, LayerSpec, RgbaImage, SpriteSets, TileWorld};
    const CLEAR: [u8; 4] = [255, 0, 255, 255];
    let mut atlas = AtlasBuilder::new(256);
    let tiles: Vec<_> = (0..5u32).map(|k| atlas.add(&RgbaImage::from_fn(8, 8, |x, y| [(40 * k + 8 * x) as u8, (30 + 20 * y + 7 * k) as u8, (200 - 30 * k) as u8, 255]), (0, 0)).unwrap()).collect();
    let atlas = atlas.finish();
    let mut sets = SpriteSets::default();
    let tiles: Vec<_> = tiles.into_iter().map(|t| sets.single(t)).collect();
    let grid = Grid { cols: 256, rows: 256, layout: tile_render::Layout::Orthogonal, wrap_x: true, wrap_y: true, step: (8, 8) };
    let mut world = TileWorld::new(grid, sets);
    let layer = world.add_layer(LayerSpec::new("ground", tiles));
    for y in 0..grid.rows { for x in 0..grid.cols { world.set(layer, x, y, (x * 3 + y * 7 + x * y) % 5); } }
    // (Each into its own picture, from its top-left.)
    let render = |c: &Camera| tile_render::reference::render(&world, &atlas, &Camera { origin: (0, 0), ..*c }, 0, CLEAR);
    let mut checked = 0;
    for split in [S::Vertical1To1, S::Horizontal1To1] {
        for zoom in [Zoom::Off, Zoom::TopStrip] {
            for (width, pixel_scale) in [(4, 1.0), (13, 1.0), (4, 2.0)] {
                let c = Case { canvas: (480, 360), split, width, pixel_scale, zoom, place: Place::Floating, orders: false, across: false,
                    movement: MapMovement::EdgeScroll, ui: 1, glass: true };
                let (l, f) = layout_of(&c);
                let dividers = l.dividers();
                assert!(!dividers.under.is_empty(), "{c:?}");
                for u in &dividers.under {
                    let hit = if u.side == Some(Side::A) { Hit::Main } else { Hit::Pane };
                    let view = l.shown.iter().find(|(h, _)| *h == hit).unwrap().1;
                    let drawn = *f.cameras.iter().find(|cam| rect_of(cam) == u.rect && cam.effect.is_none()).unwrap_or_else(|| panic!("{c:?}: {u:?} not drawn"));
                    // The view going on as far as the stretch.
                    let (v, r) = (rect_of(&view), u.rect);
                    let (x0, y0) = (v[0].min(r[0]), v[1].min(r[1]));
                    let (x1, y1) = ((v[0] + v[2]).max(r[0] + r[2]), (v[1] + v[3]).max(r[1] + r[3]));
                    let (x, y) = world_at_floor(&view, (x0, y0));
                    let whole = Camera { x, y, origin: (x0, y0), viewport: ((x1 - x0) as u32, (y1 - y0) as u32), ..view };
                    let (whole, under, own) = (render(&whole), render(&drawn), render(&view));
                    for py in r[1]..r[1] + r[3] {
                        for px in r[0]..r[0] + r[2] {
                            let got = under.get((px - r[0]) as u32, (py - r[1]) as u32);
                            assert_ne!(got, CLEAR, "{c:?}: the background under the glass at ({px}, {py})");
                            assert_eq!(got, whole.get((px - x0) as u32, (py - y0) as u32), "{c:?}: {u:?} at ({px}, {py})");
                            checked += 1;
                        }
                    }
                    // The view's own picture is that picture too (no seam
                    // where it meets the stretch).
                    for py in v[1]..v[1] + v[3] {
                        for px in [v[0], v[0] + v[2] - 1] {
                            assert_eq!(own.get((px - v[0]) as u32, (py - v[1]) as u32), whole.get((px - x0) as u32, (py - y0) as u32), "{c:?}: {hit:?} at ({px}, {py})");
                        }
                    }
                    for px in v[0]..v[0] + v[2] {
                        for py in [v[1], v[1] + v[3] - 1] {
                            assert_eq!(own.get((px - v[0]) as u32, (py - v[1]) as u32), whole.get((px - x0) as u32, (py - y0) as u32), "{c:?}: {hit:?} at ({px}, {py})");
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 10_000, "{checked}");
}
