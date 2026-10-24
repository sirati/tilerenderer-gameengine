use super::*;
use scene::{Interactable, ViewState};
use tile_render::SpriteSetId;

const ROW: i32 = 20;
const FRAME: f64 = 1.0 / 60.0;
/// Where each row's name ends (its entry's anchor, as the bridge sets it).
const TEXT_END: i32 = 52;

/// The driver's list: `count` rows of 20 pixels, three shown from `first`
/// (1-based ids, sprite set = row id) in [10, 10, 100, 60].
fn frame(first: u64, count: u64) -> (Scene, Vec<PlacedSprite>, Vec<Span>) {
    let mut scene = Scene::default();
    scene.views.push(ViewState { id: 7, ..Default::default() });
    scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id: 99, rect: [10, 10, 100, 60], kind: InteractableKind::List { scroll: [10, 10, 100, 60], step: ROW } });
    let mut free = Vec::new();
    for k in 0..3 {
        let id = first + k;
        if id > count { break }
        let y = 10 + k as i32 * ROW;
        scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: Some([TEXT_END, y + ROW / 2]), id, rect: [10, y, 100, ROW], kind: InteractableKind::ListItem });
        free.push(PlacedSprite { screen: true, view: Some(0), clip: Some([10, 10, 110, 70]), ..PlacedSprite::new(SpriteSetId::from_raw(id as u32), 10, y) });
    }
    let spans = vec![Span { view: 7, start: 0, len: free.len(), cameras: vec![0; free.len()] }];
    (scene, free, spans)
}

fn rect(s: &PlacedSprite) -> Option<[i32; 4]> {
    // (The bar's track: the whole bar between its arrows.)
    let r = if s.set.raw() == 900 { bounds([s.x, s.y, 14, 44]) } else { bounds([s.x, s.y, 100, ROW]) };
    let r = s.clip.map_or(Some(r), |c| intersect(r, c))?;
    Some([r[0], r[1], r[2] - r[0], r[3] - r[1]])
}

fn pixel(sprites: &[PlacedSprite], at: (i32, i32)) -> Option<u32> {
    sprites.iter().filter(|s| s.screen && rect(s).is_some_and(|r| inside(bounds(r), at))).last().map(|s| s.set.raw())
}

/// The game and the viewer, a frame at a time: what the viewer sends
/// reaches the game after the frame, its drawing the next frame.
struct Sim {
    state: SmoothMenus,
    first: u64,
    count: u64,
    now: f64,
    snap: bool,
    shown: Vec<PlacedSprite>,
    notches: Vec<i32>,
    /// The list has the game's scroll bar ([`with_bar`]), its track drawn
    /// around its thumb if `segmented`.
    bar: bool,
    segmented: bool,
}

impl Sim {
    fn new(count: u64, snap: bool) -> Self {
        let mut sim = Sim { state: SmoothMenus::default(), first: 1, count, now: 0.0, snap, shown: vec![], notches: vec![], bar: false, segmented: false };
        sim.frame();
        sim
    }

    fn apply(&mut self, inputs: Vec<Input>) {
        for input in inputs {
            if let Input::Wheel { delta, .. } = input {
                self.notches.push(delta);
                let last = self.count.saturating_sub(2).max(1);
                self.first = if delta < 0 { (self.first + 1).min(last) } else { self.first.saturating_sub(1).max(1) };
            }
        }
    }

    fn frame(&mut self) {
        let (scene, free, spans) = if self.bar { with_bar(self.first, self.count, self.segmented) } else { frame(self.first, self.count) };
        self.shown = self.state.draw(&scene, &free, &spans, self.now, true, self.snap, 1, None, rect).unwrap_or(free);
        let out = self.state.take_inputs();
        self.apply(out);
        self.now += FRAME;
    }

    fn frames(&mut self, n: usize) { for _ in 0..n { self.frame() } }

    fn route(&mut self, input: Input) -> Vec<Input> {
        let out = self.state.route(input, false);
        self.apply(out.clone());
        out
    }

    fn wheel(&mut self, notches: i32) { assert!(self.route(Input::Wheel { delta: -notches * NOTCH, x: 20, y: 30 }).is_empty()) }

    fn pos(&self) -> f64 { self.state.lists[0].pos }
    fn shown(&self) -> i64 { self.state.lists[0].shown }
}

#[test]
fn the_wheel_moves_the_list_continuously_and_new_notches_add_at_once() {
    let mut sim = Sim::new(20, true);
    sim.wheel(1);
    let mut seen = vec![sim.pos()];
    for _ in 0..12 {
        sim.frame();
        seen.push(sim.pos());
    }
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "never back: {seen:?}");
    assert!(seen.iter().any(|&p| p > 0.5 && (p % ROW as f64) > 0.5 && (p % ROW as f64) < 19.5), "between items: {seen:?}");
    let before = sim.pos();
    assert!(before < ROW as f64, "still on the way: {before}");
    // A second notch mid-way: the target grows at once, the motion goes on
    // without a stop and ends two items down.
    sim.wheel(1);
    assert_eq!(sim.state.lists[0].target, 2.0 * ROW as f64);
    sim.frame();
    assert!(sim.pos() > before);
    sim.frames(60);
    assert_eq!(sim.pos(), 2.0 * ROW as f64);
    assert_eq!(sim.first, 3, "the driver shows the list's items");
    assert!(!sim.state.running());
    assert_eq!(pixel(&sim.shown, (20, 15)), Some(3));
}

#[test]
fn a_notch_against_the_motion_turns_it_at_once() {
    let mut sim = Sim::new(20, true);
    sim.wheel(2);
    sim.frames(8);
    let at = sim.pos();
    assert!(at > 0.0 && at < 2.0 * ROW as f64);
    sim.wheel(-2);
    sim.frame();
    assert!(sim.pos() < at, "turned within a frame: {at} -> {}", sim.pos());
    sim.frames(60);
    assert_eq!(sim.pos(), 0.0);
    assert_eq!(sim.first, 1);
}

#[test]
fn rows_show_at_every_pixel_on_the_way_from_both_drawings() {
    let mut sim = Sim::new(20, true);
    sim.wheel(1);
    for _ in 0..30 {
        sim.frame();
        let p = sim.shown() as i32;
        // Every pixel of the list shows the row the content has there.
        for y in 10..70 {
            let want = 1 + (y - 10 + p) / ROW;
            assert_eq!(pixel(&sim.shown, (20, y)), Some(want as u32), "at offset {p}, y {y}");
        }
    }
}

fn trackpad(sim: &mut Sim, pixels: i32) {
    // (A finger moving up scrolls down: negative dy.)
    assert!(sim.route(Input::Scroll { dx: 0, dy: -pixels * 1000, pixels: true, x: 20, y: 30 }).is_empty());
    assert!(sim.route(Input::Wheel { delta: -NOTCH, x: 20, y: 30 }).is_empty(), "the scroll's own notch is not sent too");
}

#[test]
fn a_trackpad_stops_anywhere_and_snaps_only_after_the_input_rests() {
    let mut sim = Sim::new(20, true);
    trackpad(&mut sim, 5);
    sim.frames(4);
    assert_eq!(sim.pos(), 5.0, "at the finger's pixel, not an item");
    sim.frames(((IDLE - 4.0 * FRAME) / FRAME) as usize - 1);
    assert_eq!(sim.pos(), 5.0, "no snap while input may go on");
    sim.frames(60);
    assert_eq!(sim.pos(), 0.0, "a quarter of an item on: back to its edge");
    assert_eq!(sim.first, 1);
    trackpad(&mut sim, 9);
    sim.frames(60);
    assert_eq!(sim.pos(), ROW as f64, "beyond a third in the direction of travel: on to the next");
    assert_eq!(sim.first, 2);
    assert!(!sim.state.running());
}

#[test]
fn without_snapping_a_list_rests_between_items() {
    let mut sim = Sim::new(20, false);
    trackpad(&mut sim, 9);
    sim.frames(60);
    assert_eq!(sim.pos(), 9.0);
    assert!(!sim.state.running());
    // The driver holds the nearer whole position, the rest is drawn from
    // the drawing it made on the way.
    assert_eq!(sim.first, 1);
    assert_eq!(pixel(&sim.shown, (20, 15)), Some(1));
    assert_eq!(pixel(&sim.shown, (20, 25)), Some(2));
    assert_eq!(pixel(&sim.shown, (20, 62)), Some(4));
    let entries = &sim.state.shown_entries()[0];
    assert_eq!(entries.0, [10, 10, 100, 60]);
    assert_eq!(entries.1.iter().map(|e| (e.id, e.rect[1], e.rect[3])).collect::<Vec<_>>(), [(1, 10, 11), (2, 21, 20), (3, 41, 20), (4, 61, 9)],
        "every row shown is a target where it shows, the one the driver does not hold too");
}

/// Row `id`'s rectangle (x, y, w, h) as shown at content offset `p`,
/// clipped to the list.
fn shown_row(id: u64, p: i64) -> Option<[i32; 4]> {
    let top = 10 + (id as i32 - 1) * ROW - p as i32;
    let (t, b) = (top.max(10), (top + ROW).min(70));
    (b - t >= 2).then_some([10, t, 100, b - t])
}

fn check_targets(sim: &Sim) {
    let p = sim.shown();
    let entries: Vec<Interactable> = sim.state.shown_entries().into_iter().flat_map(|(_, e)| e).collect();
    if p == sim.state.lists[0].drawn {
        assert!(entries.is_empty(), "the driver's own entries are the targets at {p}");
        return;
    }
    let want: Vec<(u64, [i32; 4])> = (1..=20).filter_map(|id| shown_row(id, p).map(|r| (id, r))).collect();
    assert_eq!(entries.iter().map(|e| (e.id, e.rect)).collect::<Vec<_>>(), want, "targets as shown at {p}");
    for e in &entries {
        let [x, y, w, h] = e.rect;
        let [ax, ay] = e.anchor.unwrap();
        assert_eq!(ax, TEXT_END, "at the end of the row's name");
        assert!(ax >= x && ax < x + w && ay >= y && ay < y + h, "inside its row {e:?}");
        if h == ROW { assert_eq!(ay, y + ROW / 2, "in the middle of the row") }
    }
}

#[test]
fn at_every_offset_each_row_shown_is_an_alt_target_where_it_shows() {
    let mut sim = Sim::new(20, false);
    let mut offsets = std::collections::BTreeSet::new();
    for _ in 0..4 {
        sim.wheel(1);
        for _ in 0..40 {
            sim.frame();
            offsets.insert(sim.shown());
            check_targets(&sim);
        }
    }
    for px in [3, 7, 5, 11, 2] {
        trackpad(&mut sim, px);
        for _ in 0..30 {
            sim.frame();
            offsets.insert(sim.shown());
            check_targets(&sim);
        }
    }
    assert!(offsets.iter().filter(|p| *p % i64::from(ROW) != 0).count() >= 15, "many offsets between items: {offsets:?}");
}

#[test]
fn in_alt_mode_the_wheel_scrolls_and_the_targets_follow() {
    let mut sim = Sim::new(20, true);
    assert!(sim.state.route(Input::Wheel { delta: -NOTCH, x: 20, y: 30 }, true).is_empty());
    let mut between = false;
    for _ in 0..60 {
        sim.frame();
        between |= sim.shown() % i64::from(ROW) != 0;
        check_targets(&sim);
    }
    assert!(between);
    assert_eq!(sim.pos(), ROW as f64);
    assert_eq!(sim.first, 2);
}

#[test]
fn pointer_input_goes_to_the_row_shown_at_a_fractional_offset() {
    let mut sim = Sim::new(20, false);
    trackpad(&mut sim, 9);
    sim.frames(60);
    // Row 2 shows at 21..41; the driver has it at 30..50.
    assert_eq!(sim.route(Input::MouseMove { x: 20, y: 25 }), vec![Input::MouseMove { x: 20, y: 34 }]);
    // Row 4 (51..70 shown) is not the driver's yet: the press brings it
    // in with a notch, then lands on it.
    let press = sim.route(Input::MouseButton { button: Button::Left, down: true, x: 20, y: 65 });
    assert_eq!(press, vec![Input::Wheel { delta: -NOTCH, x: 60, y: 40 }, Input::MouseButton { button: Button::Left, down: true, x: 20, y: 54 }]);
    assert_eq!(sim.first, 2);
    sim.frames(30);
    assert_eq!(sim.first, 2, "the driver's position stays while the button is held");
    let release = sim.route(Input::MouseButton { button: Button::Left, down: false, x: 20, y: 65 });
    assert_eq!(release, vec![Input::MouseButton { button: Button::Left, down: false, x: 20, y: 54 }]);
    // Row 4 in the driver's list at first = 2: 50..70.
    assert!((50..70).contains(&54));
}

#[test]
fn the_list_stops_where_the_driver_does_at_its_end() {
    let mut sim = Sim::new(5, true);
    sim.wheel(6);
    sim.frames(120);
    assert_eq!(sim.first, 3);
    assert_eq!(sim.pos(), 2.0 * ROW as f64, "never past the last row drawn");
    assert_eq!(sim.state.lists[0].target, 2.0 * ROW as f64, "and goes no further");
    assert!(!sim.state.running());
    assert_eq!(pixel(&sim.shown, (20, 65)), Some(5));
    // Back up: free to go.
    sim.wheel(-1);
    sim.frames(60);
    assert_eq!(sim.pos(), ROW as f64);
}

#[test]
fn the_scroll_bar_tells_the_ends_at_once() {
    let mut state = SmoothMenus::default();
    let (mut scene, free, spans) = frame(1, 20);
    scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id: 500, rect: [96, 10, 14, 60], kind: InteractableKind::Slider { track: [96, 24, 14, 32], value: 0 } });
    state.draw(&scene, &free, &spans, 0.0, true, true, 1, None, rect);
    assert!(state.route(Input::Wheel { delta: NOTCH, x: 20, y: 30 }, false).is_empty());
    assert_eq!(state.lists[0].target, 0.0, "nothing above the top");
    for k in 1..20 {
        state.draw(&scene, &free, &spans, k as f64 * FRAME, true, true, 1, None, rect);
        assert!(state.take_inputs().is_empty());
    }
    assert!(!state.running());
}

#[test]
fn a_list_the_driver_scrolls_itself_glides_after_it() {
    let mut sim = Sim::new(20, true);
    // (The scroll bar's arrow, in the driver.)
    sim.first = 2;
    sim.frames(3);
    assert!(sim.pos() > 0.0 && sim.pos() < ROW as f64, "on the way: {}", sim.pos());
    sim.frames(60);
    assert_eq!(sim.pos(), ROW as f64);
    assert!(sim.notches.is_empty(), "the driver is not sent back");
}

#[test]
fn snap_points_follow_the_direction_of_travel() {
    assert_eq!(snap_point(5.0, 0.0, 20.0, 1.0), 0.0);
    assert_eq!(snap_point(7.0, 0.0, 20.0, 1.0), 20.0);
    // Up from 20: a quarter of an item back stays, a third goes on.
    assert_eq!(snap_point(15.0, 0.0, 20.0, -1.0), 20.0);
    assert_eq!(snap_point(13.0, 0.0, 20.0, -1.0), 0.0);
    assert_eq!(snap_point(31.0, 10.0, 20.0, 0.0), 30.0);
}

#[test]
fn alt_mode_notches_scroll_and_its_window_scroll_is_dropped() {
    let mut sim = Sim::new(20, true);
    assert!(sim.state.route(Input::Scroll { dx: 0, dy: -5000, pixels: true, x: 20, y: 30 }, true).is_empty());
    assert_eq!(sim.state.lists[0].target, 0.0);
    assert!(sim.state.route(Input::Wheel { delta: -NOTCH, x: 20, y: 30 }, true).is_empty());
    assert_eq!(sim.state.lists[0].target, ROW as f64);
}

#[test]
fn input_away_from_lists_and_with_the_option_off_is_unchanged() {
    let mut sim = Sim::new(20, true);
    let wheel = Input::Wheel { delta: -NOTCH, x: 300, y: 300 };
    assert_eq!(sim.state.route(wheel.clone(), false), vec![wheel.clone()]);
    let mut off = SmoothMenus::default();
    let (scene, free, spans) = frame(1, 20);
    assert!(off.draw(&scene, &free, &spans, 0.0, false, true, 1, None, rect).is_none());
    let inside = Input::Wheel { delta: -NOTCH, x: 20, y: 30 };
    assert_eq!(off.route(inside.clone(), false), vec![inside]);
}

#[test]
fn a_scroll_bar_in_the_list_stays_put_while_the_rows_glide() {
    let bar = |thumb_y: i32, first: u64| {
        let (mut scene, mut free, _) = frame(first, 20);
        scene.interactables.push(Interactable { region: None, alt_right_click: false, anchor: None, id: 500, rect: [96, 10, 14, 60], kind: InteractableKind::Slider { track: [96, 24, 14, 32], value: 300_000 } });
        let track = PlacedSprite { screen: true, view: Some(0), clip: Some([96, 10, 110, 70]), ..PlacedSprite::new(SpriteSetId::from_raw(900), 96, 10) };
        let thumb = PlacedSprite { screen: true, view: Some(0), clip: Some([96, thumb_y, 110, thumb_y + 10]), ..PlacedSprite::new(SpriteSetId::from_raw(901), 96, thumb_y) };
        free.extend([track, thumb]);
        let spans = vec![Span { view: 7, start: 0, len: free.len(), cameras: vec![0; free.len()] }];
        (scene, free, spans, [track, thumb])
    };
    let mut state = SmoothMenus::default();
    let (scene, free, spans, _) = bar(20, 1);
    state.draw(&scene, &free, &spans, 0.0, true, true, 1, None, rect);
    assert!(state.route(Input::Wheel { delta: -NOTCH, x: 20, y: 30 }, false).is_empty());
    state.draw(&scene, &free, &spans, FRAME, true, true, 1, None, rect);
    assert_eq!(state.take_inputs().len(), 1);
    let (scene, free, spans, still) = bar(30, 2);
    let animated = state.draw(&scene, &free, &spans, 3.0 * FRAME, true, true, 1, None, rect).expect("glides");
    assert_eq!(pixel(&animated, (20, 12)), Some(1), "the rows glide");
    for s in still { assert!(animated.contains(&s), "{s:?} unchanged"); }
    assert!(!animated.iter().any(|s| matches!(s.set.raw(), 900 | 901) && !still.contains(s)), "a moved copy of the bar");
}

#[test]
fn map_sprites_and_controls_outside_the_list_stay_unchanged() {
    let mut sim = Sim::new(20, false);
    trackpad(&mut sim, 9);
    sim.frames(30);
    let (scene, mut free, spans) = frame(sim.first, 20);
    let world = PlacedSprite { view: None, screen: false, ..free[0] };
    let button = PlacedSprite { x: 150, y: 150, clip: Some([150, 150, 250, 170]), ..free[0] };
    free.extend([world, button]);
    let shown = sim.state.draw(&scene, &free, &spans, sim.now, true, false, 1, None, rect).expect("between items");
    assert!(shown.contains(&world));
    assert!(shown.contains(&button));
}

#[test]
fn a_new_screen_starts_afresh() {
    let mut sim = Sim::new(20, false);
    trackpad(&mut sim, 9);
    sim.frames(30);
    let (mut scene, free, spans) = frame(sim.first, 20);
    scene.interface.popup = Some(5);
    assert!(sim.state.draw(&scene, &free, &spans, sim.now, true, false, 1, None, rect).is_none());
    assert!(!sim.state.running());
}

// --- the scroll bar ---

const BAR: u64 = 500;
const THUMB: u32 = 901;

/// The bar's value with `first` shown of `count`.
fn bar_value(first: u64, count: u64) -> i32 { ((first - 1) as f64 * 1e6 / (count - 3) as f64).round() as i32 }

/// The game's thumb's top with that value: its middle travels 23..57.
fn thumb_top(value: i32) -> i32 { 23 + (f64::from(value) * 34.0 / 1e6).round() as i32 - 5 }

/// [`frame`] with the game's scroll bar at the right: arrows (buttons)
/// 10..18 and 62..70, the track 18..62 drawn whole, a 10 px thumb.
fn with_bar(first: u64, count: u64, segmented: bool) -> (Scene, Vec<PlacedSprite>, Vec<Span>) {
    let (mut scene, mut free, _) = frame(first, count);
    let value = bar_value(first, count);
    let button = |id, y| Interactable { region: None, alt_right_click: false, anchor: None, id, rect: [96, y, 14, 8], kind: InteractableKind::Button };
    scene.interactables.extend([
        Interactable { region: None, alt_right_click: false, anchor: None, id: BAR, rect: [96, 10, 14, 60], kind: InteractableKind::Slider { track: [103, 23, 1, 34], value } },
        button(501, 10), button(502, 62),
    ]);
    let ty = thumb_top(value);
    let track = |clip| PlacedSprite { screen: true, view: Some(0), clip: Some(clip), ..PlacedSprite::new(SpriteSetId::from_raw(900), 96, 18) };
    if segmented {
        // (As the game: the track above and below the thumb, none under it.)
        free.extend([track([96, 18, 110, ty]), track([96, ty + 10, 110, 62])].into_iter().filter(|s| s.clip.is_some_and(|c| c[3] > c[1])));
    } else {
        free.push(track([96, 18, 110, 62]));
    }
    free.push(PlacedSprite { screen: true, view: Some(0), clip: Some([96, ty, 110, ty + 10]), ..PlacedSprite::new(SpriteSetId::from_raw(THUMB), 96, ty) });
    let spans = vec![Span { view: 7, start: 0, len: free.len(), cameras: vec![0; free.len()] }];
    (scene, free, spans)
}

fn shown_thumb(sim: &Sim) -> i32 {
    sim.shown.iter().find(|s| s.set.raw() == THUMB).expect("the thumb").y
}

/// A list with its bar whose scale is known: one notch down and back.
fn sim_with_bar(snap: bool) -> Sim {
    let mut sim = Sim::new(20, snap);
    sim.bar = true;
    sim.frames(2);
    sim.wheel(1);
    sim.frames(40);
    sim.wheel(-1);
    sim.frames(40);
    assert_eq!((sim.first, sim.pos()), (1, 0.0));
    assert!(sim.state.lists[0].range().is_some(), "the bar's scale is known");
    sim
}

#[test]
fn the_thumb_moves_with_the_list_by_pixels_and_lands_on_the_games() {
    let mut sim = sim_with_bar(true);
    sim.wheel(3);
    let mut tops = vec![];
    for _ in 0..60 {
        sim.frame();
        let top = shown_thumb(&sim);
        // At the list's position, the bar's scale: 340 content pixels a
        // 34 pixel travel.
        let want = 18 + (sim.shown() as f64 * 34.0 / 340.0).round() as i32;
        assert!((top - want).abs() <= 1, "thumb at {top}, list at {} ({want})", sim.shown());
        tops.push(top);
    }
    assert!(tops.windows(2).all(|w| w[1] >= w[0]), "never back: {tops:?}");
    let distinct: std::collections::BTreeSet<_> = tops.iter().collect();
    assert!(distinct.len() >= 5, "moves every few frames, not in item jumps: {tops:?}");
    // At rest: the game's own thumb, unmoved.
    assert_eq!(sim.first, 4);
    assert_eq!(shown_thumb(&sim), thumb_top(bar_value(4, 20)));
}

#[test]
fn a_press_in_the_track_glides_the_list_there_and_snaps() {
    let mut sim = sim_with_bar(true);
    // (The thumb's middle travels 23..57: 52 puts it 29/34 down, content
    // 290, snapped on to 300 in the direction it went.)
    assert!(sim.route(Input::MouseButton { button: Button::Left, down: true, x: 103, y: 52 }).is_empty(), "the press is the viewer's");
    assert!(sim.route(Input::MouseButton { button: Button::Left, down: false, x: 103, y: 52 }).is_empty());
    let mut seen = vec![];
    for _ in 0..90 {
        sim.frame();
        seen.push(sim.pos());
    }
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "one glide: {seen:?}");
    assert!(seen.iter().filter(|p| **p > 0.0 && **p < 300.0).count() >= 6, "a glide, not a jump: {seen:?}");
    assert_eq!(sim.pos(), 300.0);
    assert_eq!(sim.first, 16, "the game's position in step");
    assert!(sim.notches.iter().all(|&d| d == -NOTCH || d == NOTCH));
}

#[test]
fn the_thumb_dragged_moves_the_list_by_pixels() {
    let mut sim = sim_with_bar(false);
    let top = shown_thumb(&sim);
    assert!(sim.route(Input::MouseButton { button: Button::Left, down: true, x: 103, y: top + 5 }).is_empty());
    for k in 1..=10 {
        assert!(sim.route(Input::MouseMove { x: 103, y: top + 5 + k }).is_empty());
        sim.frames(3);
    }
    sim.frames(30);
    // Ten pixels of the bar: 100 of the list.
    assert_eq!(sim.state.lists[0].target, 100.0);
    assert_eq!(sim.pos(), 100.0, "stays where it was let go");
    assert!(sim.route(Input::MouseButton { button: Button::Left, down: false, x: 103, y: top + 15 }).is_empty());
    sim.frames(30);
    assert_eq!(sim.pos(), 100.0);
    assert_eq!(sim.first, 6, "the game's position in step");
}

#[test]
fn the_alt_modes_edit_of_the_bar_glides_the_list_and_cancel_returns() {
    let mut sim = sim_with_bar(false);
    let edit = |action, value| Input::Slider { id: BAR, action, value };
    assert!(sim.route(edit(SliderAction::Begin, 0)).is_empty());
    assert!(sim.route(edit(SliderAction::Preview, 500_000)).is_empty());
    assert_eq!(sim.state.lists[0].target, 170.0);
    sim.frames(5);
    assert!(sim.pos() > 0.0 && sim.pos() < 170.0, "gliding: {}", sim.pos());
    assert!(sim.route(edit(SliderAction::Cancel, 500_000)).is_empty());
    sim.frames(60);
    assert_eq!(sim.pos(), 0.0);
    assert!(sim.route(edit(SliderAction::Commit, 250_000)).is_empty());
    sim.frames(60);
    assert_eq!(sim.pos(), 85.0);
    // (A step is the game's.)
    assert_eq!(sim.route(edit(SliderAction::Step, 1)), vec![edit(SliderAction::Step, 1)]);
}

#[test]
fn before_the_bars_scale_is_known_its_presses_are_the_games() {
    let mut sim = Sim::new(20, true);
    sim.bar = true;
    sim.frames(2);
    let press = Input::MouseButton { button: Button::Left, down: true, x: 103, y: 50 };
    assert_eq!(sim.route(press.clone()), vec![press]);
    // The arrows are always the game's.
    let mut sim = sim_with_bar(true);
    let arrow = Input::MouseButton { button: Button::Left, down: true, x: 103, y: 65 };
    assert_eq!(sim.route(arrow.clone()), vec![arrow]);
}

#[test]
fn where_the_games_thumb_was_its_track_shows() {
    let mut sim = Sim::new(20, true);
    (sim.bar, sim.segmented) = (true, true);
    sim.frames(2);
    sim.wheel(1);
    sim.frames(40);
    sim.wheel(-1);
    sim.frames(40);
    sim.wheel(4);
    let mut moved = 0;
    for _ in 0..60 {
        sim.frame();
        if shown_thumb(&sim) != thumb_top(bar_value(sim.first, 20)) { moved += 1 }
        for y in 18..62 {
            assert!(matches!(pixel(&sim.shown, (100, y)), Some(900 | 901)), "a hole in the bar at {y}, list at {}", sim.shown());
        }
    }
    assert!(moved >= 5, "the thumb was shown away from the game's: {moved}");
}
