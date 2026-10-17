use super::*;
use scene::{Scene, View, ViewState};

const IDENTITY: ((i32, i32), (u32, u32)) = ((0, 0), (1, 1));
const WINDOW: (u32, u32) = (1000, 800);
const TICK: f64 = 1.0 / 120.0;
/// A flight at the default 3 updates a second: 1.33 intervals.
const D: f64 = 4.0 / 9.0;

fn button(id: u64, rect: [i32; 4]) -> Interactable {
    Interactable { rect, kind: InteractableKind::Button, id }
}

fn item(id: u64, rect: [i32; 4]) -> Interactable {
    Interactable { rect, kind: InteractableKind::ListItem, id }
}

fn list(id: u64, rect: [i32; 4], step: i32) -> Interactable {
    Interactable { rect, kind: InteractableKind::List { scroll: rect, step }, id }
}

/// A 20×20 button with its middle at (x, y).
fn at(id: u64, x: i32, y: i32) -> Interactable {
    button(id, [x - 10, y - 10, 20, 20])
}

/// Two buttons in a row, A at (120, 110), B at (320, 110), and C far right
/// at (920, 110).
fn row() -> Vec<Target> {
    targets(&[button(1, [100, 100, 40, 20]), button(2, [300, 100, 40, 20]), button(3, [900, 100, 40, 20])], IDENTITY, WINDOW)
}

/// A list (id 10) at (500, 100), 200×300, its entries 20 high (ids 100..),
/// and a button (id 4) right of it at (850, 210).
fn with_list() -> Vec<Target> {
    let mut items = vec![list(10, [500, 100, 200, 300], 0), button(4, [830, 200, 40, 20])];
    items.extend((0..15).map(|k| item(100 + k, [500, 100 + 20 * k as i32, 200, 20])));
    targets(&items, IDENTITY, WINDOW)
}

/// Runs the snapper from `from` to `to` seconds in ticks, `motion` raw
/// counts per tick while `moving(t)`; the actions with their times.
fn run(s: &mut Snapper, targets: &[Target], from: f64, to: f64, motion: Point, moving: impl Fn(f64) -> bool) -> Vec<(f64, Action)> {
    let mut out = Vec::new();
    let mut t = from;
    while t < to - 1e-9 {
        t += TICK;
        if moving(t) {
            s.motion(motion);
        }
        out.extend(s.tick(t, targets, Rates::default()).into_iter().map(|a| (t, a)));
    }
    out
}

fn moves(actions: &[(f64, Action)]) -> Vec<(f64, Point)> {
    actions
        .iter()
        .filter_map(|(t, a)| match a {
            Action::Move(p) => Some((*t, *p)),
            _ => None,
        })
        .collect()
}

fn close(a: Point, b: Point, eps: f64) -> bool {
    (a.0 - b.0).abs() <= eps && (a.1 - b.1).abs() <= eps
}

// --- the flight ---

#[test]
fn a_flight_lasts_1_33_update_intervals_and_lands_exactly_overshooting_a_little() {
    for rate in [3.0, 6.0, 1.0] {
        let rates = Rates { updates_per_second: rate, ghost_fps: 30.0 };
        assert!((rates.flight() - 4.0 / 3.0 / rate).abs() < 1e-12);
        let d = rates.flight();
        let f = Flight { from: (0.0, 0.0), vel: (0.0, 0.0), to: (300.0, 0.0), start: 1.0, duration: d };
        assert_eq!(f.position(1.0), (0.0, 0.0));
        assert_eq!(f.end(), 1.0 + d);
        // On the target exactly at its end, still; not before.
        assert_eq!(f.position(1.0 + d), (300.0, 0.0));
        assert_eq!(f.velocity(1.0 + d), (0.0, 0.0));
        assert!(!f.done(1.0 + d - 1e-6) && f.done(1.0 + d));
        let (mut crossed, mut peak, mut last) = (None, 0.0f64, 0.0);
        let n = 2000;
        for k in 0..n {
            let t = 1.0 + d * k as f64 / n as f64;
            let p = f.position(t);
            assert!(p.1 == 0.0);
            if crossed.is_none() && p.0 >= 300.0 {
                crossed = Some((t - 1.0) / d);
            }
            if crossed.is_none() {
                // Smoothly faster on the way there.
                assert!(p.0 >= last - 1e-9, "{t} {p:?}");
            }
            last = p.0;
            peak = peak.max(p.0);
        }
        // Past the target about half way, by a few per cent, then back.
        let crossed = crossed.unwrap();
        assert!(crossed > 0.4 && crossed < 0.6, "{crossed}");
        let overshoot = (peak - 300.0) / 300.0;
        assert!(overshoot > 0.01 && overshoot < 0.06, "{overshoot}");
        // Near the end it is close and slow.
        assert!((f.position(1.0 + 0.98 * d).0 - 300.0).abs() < 2.0);
    }
}

#[test]
fn a_new_target_in_flight_keeps_the_velocity_and_lands_on_time() {
    let mut f = Flight { from: (0.0, 0.0), vel: (0.0, 0.0), to: (300.0, 0.0), start: 0.0, duration: D };
    let (p, v) = (f.position(0.2), f.velocity(0.2));
    assert!(v.0 > 300.0, "{v:?}");
    f.retarget((300.0, 200.0), 0.2, D);
    // Where it was, as fast, the same way.
    assert!(close(f.position(0.2), p, 1e-9));
    assert!(close(f.velocity(0.2), v, 1e-6), "{:?} {v:?}", f.velocity(0.2));
    // Just after: still going right, now also down.
    let w = f.velocity(0.21);
    assert!(w.0 > 0.8 * v.0 && w.1 > 0.0, "{w:?}");
    assert_eq!(f.position(0.2 + D), (300.0, 200.0));
    assert_eq!(f.velocity(0.2 + D), (0.0, 0.0));
    // A flight's velocity is its position's rate.
    let h = 1e-6;
    let num = ((f.position(0.3 + h).0 - f.position(0.3 - h).0) / (2.0 * h), (f.position(0.3 + h).1 - f.position(0.3 - h).1) / (2.0 * h));
    assert!(close(num, f.velocity(0.3), 1e-3), "{num:?} {:?}", f.velocity(0.3));
}

// --- where it goes ---

#[test]
fn only_elements_within_45_degrees_of_the_motion_are_picked() {
    // From (500, 400): right at 0°, at 44° and at 47° (down right), and
    // one up.
    let t = targets(&[at(1, 600, 400), at(2, 600, 497), at(3, 600, 509), at(4, 500, 300)], IDENTITY, WINDOW);
    let from = (500.0, 400.0);
    assert_eq!(pick(&t, from, None, (20.0, 0.0)), Some(0));
    // Down right: 0 is 45° off, 1 about 1°.
    assert_eq!(pick(&t, from, None, (20.0, 20.0)), Some(1));
    // Right, without the one straight ahead: 44° yes, 47° no.
    let t2: Vec<Target> = t.iter().copied().filter(|x| x.id != 1).collect();
    assert_eq!(t2[pick(&t2, from, None, (20.0, 0.0)).unwrap()].id, 2);
    let t3: Vec<Target> = t.iter().copied().filter(|x| x.id == 3 || x.id == 4).collect();
    assert_eq!(pick(&t3, from, None, (20.0, 0.0)), None);
    // Left: nothing there.
    assert_eq!(pick(&t, from, None, (-20.0, 0.0)), None);
    // Up.
    assert_eq!(t[pick(&t, from, None, (0.0, -20.0)).unwrap()].id, 4);
    // Never the one snapped to.
    assert_eq!(t[pick(&t, (600.0, 400.0), Some(1), (-20.0, -1.0)).unwrap()].id, 4);
}

#[test]
fn the_direction_counts_far_more_than_the_distance() {
    // Right of (500, 400): one straight ahead 150 px away, one 18° off at
    // 63 px, one 30° off at 58 px.
    let t = targets(&[at(1, 650, 400), at(2, 560, 420), at(3, 550, 371)], IDENTITY, WINDOW);
    assert_eq!(t[pick(&t, (500.0, 400.0), None, (15.0, 0.0)).unwrap()].id, 1);
    // Along a row: the one about as far as the mouse went.
    let r = targets(&[at(1, 200, 100), at(2, 300, 100), at(3, 400, 100)], IDENTITY, WINDOW);
    let from = (100.0, 100.0);
    assert_eq!(r[pick(&r, from, None, (20.0, 0.0)).unwrap()].id, 1);
    assert_eq!(r[pick(&r, from, None, (190.0, 5.0)).unwrap()].id, 2);
    assert_eq!(r[pick(&r, from, None, (320.0, -10.0)).unwrap()].id, 3);
}

#[test]
fn motion_within_the_dead_zone_picks_nothing() {
    let t = row();
    assert_eq!(pick(&t, (120.0, 110.0), Some(1), (DEAD_ZONE - 1.0, 0.0)), None);
    assert_eq!(pick(&t, (120.0, 110.0), Some(1), (DEAD_ZONE, 0.0)), Some(1));
    // Jitter back and forth for a second: the pointer stays.
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    let mut t0 = 0.0;
    for k in 0..120 {
        t0 += TICK;
        s.motion(if k % 2 == 0 { (4.0, 1.0) } else { (-4.0, -1.0) });
        s.tick(t0, &t, Rates::default());
    }
    assert_eq!(s.target(), Some((120.0, 110.0)));
    // A nudge, a pause, a nudge: forgotten in between.
    s.motion((8.0, 0.0));
    run(&mut s, &t, t0, t0 + 0.4, (0.0, 0.0), |_| false);
    s.motion((8.0, 0.0));
    run(&mut s, &t, t0 + 0.4, t0 + 0.5, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((120.0, 110.0)));
    // Slow but steady motion adds up.
    run(&mut s, &t, t0 + 0.5, t0 + 1.0, (0.25, 0.0), |_| true);
    assert_eq!(s.target(), Some((320.0, 110.0)));
}

#[test]
fn without_an_element_that_way_the_pointer_stays() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 1.0, (0.0, 4.0), |t| t < 0.5);
    assert_eq!(s.target(), Some((120.0, 110.0)));
    assert_eq!(s.pointer(), Some((120.0, 110.0)));
}

// --- the mode ---

#[test]
fn alt_snaps_at_once_to_the_nearest_and_lands_there_on_time() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (200.0, 300.0), &t, Rates::default());
    assert!(s.snapping());
    assert_eq!(s.target(), Some((120.0, 110.0)));
    // Flying from where the pointer was, from now on.
    assert_eq!(s.pointer(), Some((200.0, 300.0)));
    assert_eq!((s.flight().start, s.flight().end()), (0.0, D));
    let actions = run(&mut s, &t, 0.0, 1.0, (0.0, 0.0), |_| false);
    // The driver gets the pointer once it rests there, at the flight's end.
    let m = moves(&actions);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].1, (120.0, 110.0));
    assert!(m[0].0 >= D && m[0].0 < D + TICK + 1e-9, "{m:?}");
    assert_eq!(s.pointer(), Some((120.0, 110.0)));
    assert_eq!(s.rested(), Some(1));
}

#[test]
fn each_move_flies_1_33_intervals_and_hovers_only_at_rest() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    // Already on A: at rest at once.
    let actions = run(&mut s, &t, 0.0, 0.02, (0.0, 0.0), |_| false);
    assert_eq!(moves(&actions), [(TICK, (120.0, 110.0))]);
    // A short move right: B, picked as the motion passes the dead zone.
    let actions = run(&mut s, &t, 0.02, 1.5, (4.0, 0.0), |t| t < 0.06);
    let picked = s.flight().start;
    assert!(picked > 0.02 && picked < 0.06, "{picked}");
    assert_eq!(s.flight().end(), picked + D);
    let m = moves(&actions);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].1, (320.0, 110.0));
    assert!(m[0].0 >= picked + D && m[0].0 < picked + D + TICK + 1e-9, "{m:?} {picked}");
    assert!(actions.iter().all(|(_, a)| matches!(a, Action::Move(_))));
}

#[test]
fn updates_come_at_most_at_the_configured_rate() {
    let t = targets(&(0..8).map(|k| at(k + 1, 100 + 100 * k as i32, 100)).collect::<Vec<_>>(), IDENTITY, WINDOW);
    let mut s = Snapper::default();
    s.enter(0.0, (100.0, 100.0), &t, Rates::default());
    let mut picks = Vec::new();
    let mut last = s.flight().start;
    let mut now = 0.0;
    while now < 1.2 {
        now += TICK;
        s.motion((3.0, 0.0));
        s.tick(now, &t, Rates::default());
        if s.flight().start != last {
            last = s.flight().start;
            picks.push(last);
        }
    }
    assert!(picks.len() >= 3, "{picks:?}");
    assert!(picks.windows(2).all(|w| w[1] - w[0] >= 1.0 / 3.0 - 1e-9), "{picks:?}");
    // From the settings, kept sane.
    let r = Rates::of(&viewer_settings::Input { snap_updates_per_second: 0.0, snap_ghost_fps: 1000.0 });
    assert_eq!((r.updates_per_second, r.ghost_fps), (0.5, 240.0));
    assert_eq!(Rates::default(), Rates { updates_per_second: 3.0, ghost_fps: 30.0 });
}

#[test]
fn a_new_pick_in_flight_steers_the_moving_pointer() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.1, (4.0, 0.0), |t| t < 0.05);
    assert_eq!(s.target(), Some((320.0, 110.0)));
    // On the way to B, a long throw right: C at the next update.
    let mut actions = run(&mut s, &t, 0.1, 0.3, (0.0, 0.0), |_| false);
    let before = *s.flight();
    assert!(before.velocity(0.3).0.abs() > 1.0, "{before:?}");
    let mut now = 0.3;
    while s.flight().start == before.start {
        now += TICK;
        s.motion((40.0, 0.0));
        actions.extend(s.tick(now, &t, Rates::default()).into_iter().map(|a| (now, a)));
        assert!(now < 0.5);
    }
    assert_eq!(s.target(), Some((920.0, 110.0)));
    // Picked before it reached B; from where it was, at the speed it had.
    let start = s.flight().start;
    assert!(start < before.end(), "{start} {before:?}");
    assert!(close(s.flight().from, before.position(start), 1e-9));
    assert!(close(s.flight().vel, before.velocity(start), 1e-9), "{:?} {before:?}", s.flight());
    assert!(s.flight().vel != (0.0, 0.0));
    assert_eq!(s.flight().end(), start + D);
    actions.extend(run(&mut s, &t, now, 0.45, (0.0, 0.0), |_| false));
    actions.extend(run(&mut s, &t, 0.45, 2.0, (0.0, 0.0), |_| false));
    // B never showed its hover; C did, once, when it landed.
    let m = moves(&actions);
    assert_eq!(m.iter().map(|m| m.1).collect::<Vec<_>>(), [(920.0, 110.0)]);
    assert!(m[0].0 >= start + D && m[0].0 < start + D + TICK + 1e-9);
}

#[test]
fn the_ghost_shows_the_next_pick_and_never_lies_off_an_element() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.1, (4.0, 0.0), |t| t < 0.05);
    assert_eq!(s.target(), Some((320.0, 110.0)));
    // More motion right within the update interval: the ghost is on C
    // before the pointer heads there.
    run(&mut s, &t, 0.1, 0.2, (40.0, 0.0), |t| t < 0.15);
    assert_eq!(s.target(), Some((320.0, 110.0)));
    assert_eq!(s.ghost(), Some((920.0, 110.0)));
    let elements: Vec<Point> = t.iter().map(Target::center).collect();
    for _ in 0..10 {
        assert!(elements.contains(&s.ghost().unwrap()));
        run(&mut s, &t, 0.2, 0.25, (1.0, 0.0), |_| true);
    }
}

#[test]
fn a_click_clicks_the_element_snapped_to() {
    let t = row();
    let mut s = Snapper::default();
    assert_eq!(s.click(Button::Left, true), None);
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.2, (4.0, 0.0), |t| t < 0.05);
    // In flight to B: the click goes to B, the pointer is put there.
    assert_ne!(s.pointer(), Some((320.0, 110.0)));
    let down = s.click(Button::Left, true).unwrap();
    let at = (320.0, 110.0);
    assert_eq!(down, [Action::Move(at), Action::Button { button: Button::Left, down: true, at }]);
    assert_eq!(s.pointer(), Some(at));
    // At rest there, hover sent: nothing more.
    assert!(moves(&run(&mut s, &t, 0.2, 0.6, (0.0, 0.0), |_| false)).is_empty());
    // The release goes where the press went, even after snapping ended.
    s.leave();
    s.stop();
    assert_eq!(s.click(Button::Left, false), Some(vec![Action::Button { button: Button::Left, down: false, at }]));
    assert_eq!(s.click(Button::Left, false), None);
}

#[test]
fn with_nothing_snapped_to_a_click_goes_nowhere() {
    let mut s = Snapper::default();
    s.enter(0.0, (500.0, 400.0), &[], Rates::default());
    assert!(s.snapping());
    assert_eq!(s.click(Button::Left, true), Some(vec![]));
    assert_eq!(s.click(Button::Left, false), None);
    assert_eq!(s.wheel(120), Some(vec![]));
}

#[test]
fn turned_off_in_flight_it_lands_then_stays_until_the_mouse_moves() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.2, (4.0, 0.0), |t| t < 0.05);
    let end = s.flight().end();
    assert!(s.leave().is_empty());
    assert!(!s.snapping() && s.holds_pointer() && s.ghost().is_none());
    // Motion while it lands is dropped; it lands on B.
    assert!(s.motion((50.0, 50.0)));
    let actions = run(&mut s, &t, 0.2, 1.0, (0.0, 0.0), |_| false);
    let m = moves(&actions);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].1, (320.0, 110.0));
    assert!(m[0].0 >= end && m[0].0 < end + TICK + 1e-9);
    assert_eq!(s.pointer(), Some((320.0, 110.0)));
    assert_eq!(s.take_parked(), Some((320.0, 110.0)));
    // Parked: clicks and the wheel go there.
    let p = (320.0, 110.0);
    assert_eq!(s.click(Button::Right, true), Some(vec![Action::Move(p), Action::Button { button: Button::Right, down: true, at: p }]));
    assert_eq!(s.wheel(120), Some(vec![Action::Wheel { delta: 120, at: p }]));
    // The mouse takes it up again.
    assert!(!s.motion((1.0, 0.0)));
    assert_eq!(s.pointer(), None);
    assert_eq!(s.wheel(120), None);
}

#[test]
fn turned_on_again_it_goes_on_from_the_element_it_rested_on() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 1.0, (4.0, 0.0), |t| t < 0.05);
    assert_eq!(s.rested(), Some(2));
    s.leave();
    s.stop();
    // Far from B now (the world cursor moved): back on B, not near A.
    s.enter(2.0, (100.0, 700.0), &t, Rates::default());
    assert_eq!(s.target(), Some((320.0, 110.0)));
    assert_eq!(s.pointer(), Some((320.0, 110.0)));
    // B gone: the nearest to the pointer.
    let without_b: Vec<Target> = t.iter().copied().filter(|x| x.id != 2).collect();
    s.leave();
    s.stop();
    s.enter(3.0, (100.0, 700.0), &without_b, Rates::default());
    assert_eq!(s.target(), Some((120.0, 110.0)));
}

#[test]
fn the_target_follows_its_element_when_it_moves() {
    let mut t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    s.tick(TICK, &t, Rates::default());
    t[0].rect[1] += 50.0;
    let out = run(&mut s, &t, TICK, 2.0, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((120.0, 160.0)));
    assert_eq!(moves(&out).last().map(|m| m.1), Some((120.0, 160.0)));
}

#[test]
fn when_its_element_goes_the_pointer_goes_on_to_the_nearest() {
    // (A screen opened over the buttons: other targets now.)
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.5, (0.0, 0.0), |_| false);
    let screen = targets(&[at(7, 500, 600), at(8, 900, 700)], IDENTITY, WINDOW);
    let actions = run(&mut s, &screen, 0.5, 1.5, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((500.0, 600.0)));
    assert_eq!(moves(&actions).iter().map(|m| m.1).collect::<Vec<_>>(), [(500.0, 600.0)]);
    // An entry gone with nothing of its list under it: the same.
    let l = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &l, Rates::default());
    run(&mut s, &screen, 0.0, 1.0, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((500.0, 600.0)));
    // Nothing at all for a moment (the scene in between): it stays.
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &[], 0.0, 0.5, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((120.0, 110.0)));
}

#[test]
fn targets_map_into_the_window() {
    let items = [button(1, [10, 20, 30, 40]), button(2, [2000, 20, 30, 40]), button(3, [5, 5, 0, 10])];
    let t = targets(&items, IDENTITY, WINDOW);
    // Off the window or empty: left out.
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].rect, t[0].center(), t[0].id), ([10.0, 20.0, 30.0, 40.0], (25.0, 40.0), 1));
    // The driver's screen fitted at half size into a pane at (100, 50).
    let t = targets(&items[..1], ((100, 50), (1, 2)), WINDOW);
    assert_eq!(t[0].rect, [105.0, 60.0, 15.0, 20.0]);
}

#[test]
fn entries_belong_to_their_list_and_set_its_step() {
    let t = with_list();
    let TargetKind::List { step, has_items, scroll } = t[0].kind else { panic!() };
    assert_eq!((step, has_items, scroll), (20.0, true, [500.0, 100.0, 200.0, 300.0]));
    assert!(t[2..].iter().all(|x| x.kind == TargetKind::ListItem { list: Some(0) }));
    assert_eq!(t[1].kind, TargetKind::Button);
    let t = targets(&[list(1, [0, 0, 100, 100], 12), item(2, [0, 300, 50, 10])], ((0, 0), (2, 1)), WINDOW);
    assert_eq!(t[0].kind, TargetKind::List { scroll: [0.0, 0.0, 200.0, 200.0], step: 24.0, has_items: false });
    assert_eq!(t[1].kind, TargetKind::ListItem { list: None });
    let t = targets(&[list(1, [0, 0, 100, 100], 0)], IDENTITY, WINDOW);
    assert!(matches!(t[0].kind, TargetKind::List { step, .. } if step == DEFAULT_SCROLL_STEP));
}

#[test]
fn the_nearest_is_the_one_under_the_point_else_the_closest_edge() {
    let t = row();
    assert_eq!(nearest(&t, (130.0, 105.0)), Some(0));
    assert_eq!(nearest(&t, (160.0 + 140.0 - 1.0 + 20.0, 110.0)), Some(1));
    assert_eq!(nearest(&t, (215.0, 400.0)), Some(0));
    assert_eq!(nearest(&t, (225.0, 400.0)), Some(1));
    let t = with_list();
    assert_eq!(nearest(&t, (600.0, 101.0)), Some(2));
    let t = targets(&[list(1, [0, 0, 100, 100], 0)], IDENTITY, WINDOW);
    assert_eq!(nearest(&t, (50.0, 50.0)), Some(0));
    assert_eq!(nearest(&[], (0.0, 0.0)), None);
    // Never the capture view, even under the point.
    let mut t = row();
    t.push(Target::capture([0, 0, 1000, 800], (500, 400)));
    assert_eq!(nearest(&t, (500.0, 400.0)), Some(1));
}

// --- lists ---

#[test]
fn moving_down_the_list_scrolls_it_smoothly_and_the_pointer_stays() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    assert_eq!(s.target(), Some((600.0, 150.0)));
    // 60 px down in a tenth of a second.
    let actions = run(&mut s, &t, 0.0, 1.5, (0.0, 5.0), |t| t < 0.1);
    let wheel: Vec<(f64, i32)> = actions
        .iter()
        .filter_map(|(t, a)| match a {
            Action::Wheel { delta, at } => {
                assert_eq!(*at, (600.0, 150.0));
                Some((*t, *delta))
            }
            _ => None,
        })
        .collect();
    // Down the list: wheel notches towards the user, one per 20 px.
    assert_eq!(wheel.len(), 3, "{wheel:?}");
    assert!(wheel.iter().all(|w| w.1 == -120));
    assert!(wheel.windows(2).all(|w| w[1].0 - w[0].0 >= 1.0 / MAX_NOTCHES_PER_SECOND - 1e-9));
    assert!(wheel.last().unwrap().0 - wheel[0].0 > 0.05);
    assert_eq!(s.pointer(), Some((600.0, 150.0)));
    // Once scrolled, the driver gets the pointer (the hover of what is
    // under it now).
    let after = moves(&actions).into_iter().filter(|m| m.0 >= wheel.last().unwrap().0).collect::<Vec<_>>();
    assert_eq!(after.iter().map(|m| m.1).collect::<Vec<_>>(), [(600.0, 150.0)]);
}

#[test]
fn moving_up_the_list_scrolls_it_up() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 350.0), &t, Rates::default());
    let actions = run(&mut s, &t, 0.0, 1.5, (0.0, -2.0), |t| t < 0.1);
    let deltas: Vec<i32> = actions.iter().filter_map(|(_, a)| if let Action::Wheel { delta, .. } = a { Some(*delta) } else { None }).collect();
    assert!(!deltas.is_empty() && deltas.iter().all(|&d| d == 120), "{deltas:?}");
}

#[test]
fn from_the_first_entry_shown_moving_up_scrolls_and_a_long_move_leaves() {
    // A header button just above the list.
    let mut items = vec![list(10, [500, 100, 200, 300], 0), at(5, 600, 80)];
    items.extend((0..15).map(|k| item(100 + k, [500, 100 + 20 * k as i32, 200, 20])));
    let t = targets(&items, IDENTITY, WINDOW);
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 110.0), &t, Rates::default());
    let actions = run(&mut s, &t, 0.0, 1.0, (0.0, -5.0), |t| t < 0.03);
    assert!(actions.iter().any(|(_, a)| matches!(a, Action::Wheel { delta: 120, .. })), "{actions:?}");
    assert_eq!(s.target(), Some((600.0, 110.0)));
    // A long move up in one go: the header.
    s.motion((0.0, -60.0));
    run(&mut s, &t, 1.0, 1.1, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((600.0, 80.0)));
}

#[test]
fn moving_sideways_out_of_the_list_leaves_it() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    let actions = run(&mut s, &t, 0.0, 0.4, (4.0, 0.0), |t| t < 0.05);
    assert_eq!(s.target(), Some((850.0, 210.0)));
    assert!(actions.iter().all(|(_, a)| !matches!(a, Action::Wheel { .. })));
}

#[test]
fn the_entry_under_the_pointer_is_snapped_to_after_the_list_scrolled() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    s.tick(TICK, &t, Rates::default());
    let scrolled: Vec<Target> = t.iter().map(|x| if x.id >= 100 { Target { id: x.id + 1000, ..*x } } else { *x }).collect();
    s.tick(2.0 * TICK, &scrolled, Rates::default());
    assert_eq!(s.target(), Some((600.0, 150.0)));
    assert_eq!(s.target_id(), Some(1102));
    let actions = run(&mut s, &scrolled, 2.0 * TICK, 1.5, (0.0, 2.0), |t| t < 0.1);
    assert!(actions.iter().any(|(_, a)| matches!(a, Action::Wheel { .. })));
}

#[test]
fn a_list_without_entries_is_snapped_to_and_scrolled_at_its_middle() {
    let t = targets(&[list(1, [100, 100, 200, 400], 10)], IDENTITY, WINDOW);
    let mut s = Snapper::default();
    s.enter(0.0, (0.0, 0.0), &t, Rates::default());
    assert_eq!(s.target(), Some((200.0, 300.0)));
    run(&mut s, &t, 0.0, 2.0, (0.0, 0.0), |_| false);
    let actions = run(&mut s, &t, 2.0, 3.5, (0.0, 2.0), |t| t < 2.1);
    assert!(actions.iter().any(|(_, a)| matches!(a, Action::Wheel { delta: -120, at: (200.0, 300.0) })));
}

#[test]
fn scrolling_sends_whole_notches_rounded_and_spaced() {
    let total = |notches: f64| {
        let mut s = Scroller::default();
        s.add(notches);
        let (mut sum, mut sent, mut t) = (0, Vec::new(), 0.0);
        for _ in 0..200 {
            t += TICK;
            let n = s.step(TICK);
            if n != 0 {
                sum += n;
                sent.push(t);
            }
        }
        assert!(s.idle());
        assert!(sent.windows(2).all(|w| w[1] - w[0] >= 1.0 / MAX_NOTCHES_PER_SECOND - 1e-9));
        (sum, sent)
    };
    assert_eq!(total(3.0).0, 3);
    assert_eq!(total(-5.0).0, -5);
    assert_eq!(total(0.4).0, 0);
    assert_eq!(total(-0.6).0, -1);
    assert_eq!(total(2.5).0, 3);
    let (_, sent) = total(4.0);
    assert!(sent[2] < 0.34, "{sent:?}");
    let (sum, sent) = total(20.0);
    assert_eq!(sum, 20);
    assert!(*sent.last().unwrap() >= 19.0 / MAX_NOTCHES_PER_SECOND);
}

// --- with the layout ---

const ALT: u32 = 0x12;
const CTRL: u32 = 0x11;
const ESC: u32 = 0x1B;

/// The driver's main view and its interface, a strip at the bottom with
/// two buttons, A at (130, 535) and B at (330, 535).
fn scene() -> Scene {
    let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
    let ui = View { interface_only: true, ..Default::default() };
    Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        interactables: vec![button(1, [100, 520, 60, 30]), button(2, [300, 520, 60, 30])],
        ..Default::default()
    }
}

fn frame(l: &mut Layout, s: &Scene) -> crate::layout::Frame {
    let bounds = crate::layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
    l.frame(s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], Some(bounds))
}

/// A layout with its main view following the world cursor (the capture
/// view, the world cursor in its middle at (400, 300)), the mouse captured.
fn captured() -> (Layout, Scene) {
    let (mut l, s) = free_capturing();
    l.route(Input::MouseButton { button: Button::Left, down: true, x: 400, y: 300 });
    l.route(Input::MouseButton { button: Button::Left, down: false, x: 400, y: 300 });
    assert!(l.captured());
    frame(&mut l, &s);
    (l, s)
}

/// The same with the mouse free.
fn free_capturing() -> (Layout, Scene) {
    let s = scene();
    let mut l = Layout::default();
    l.configure("main=pointer").unwrap();
    frame(&mut l, &s);
    (l, s)
}

fn key(vk: u32, down: bool) -> Input {
    Input::Key { vk, down, text: None }
}

fn click(snap: &mut SnapInput, l: &mut Layout, s: &Scene, now: f64) -> Vec<Input> {
    let mut out = snap.route(Input::MouseButton { button: Button::Left, down: true, x: 0, y: 0 }, l, s, now, Rates::default());
    out.extend(snap.route(Input::MouseButton { button: Button::Left, down: false, x: 0, y: 0 }, l, s, now + 0.05, Rates::default()));
    out
}

fn ticks(snap: &mut SnapInput, l: &mut Layout, s: &Scene, from: f64, to: f64) -> Vec<Input> {
    let mut out = Vec::new();
    let mut t = from;
    while t < to {
        t += TICK;
        out.extend(snap.tick(l, s, t, Rates::default()));
    }
    out
}

/// Ticks with `motion` raw counts a tick while `t < until`.
fn steer(snap: &mut SnapInput, l: &mut Layout, s: &Scene, from: f64, to: f64, motion: Point, until: f64) -> Vec<Input> {
    let mut out = Vec::new();
    let mut t = from;
    while t < to {
        t += TICK;
        if t < until {
            assert_eq!(snap.motion(motion), Some(vec![]));
        }
        out.extend(snap.tick(l, s, t, Rates::default()));
    }
    out
}

fn no_world(inputs: &[Input]) -> bool {
    inputs.iter().all(|i| !matches!(i, Input::WorldButton { .. } | Input::Look { .. } | Input::WorldKey { .. }))
}

#[test]
fn alt_turns_the_mode_on_and_alt_again_off() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    // On: the driver does not get Alt; the pointer snaps to B, the nearer
    // to the world cursor in the middle of the window.
    assert!(snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default()).is_empty());
    assert!(snap.active() && l.snapping());
    assert_eq!(snap.take_hide_pointer(), Some(true));
    assert_eq!(snap.snapper.target(), Some((330.0, 535.0)));
    // Key repeat, and letting go: still on.
    assert!(snap.route(key(ALT, true), &mut l, &s, 0.05, Rates::default()).is_empty());
    assert!(snap.route(key(ALT, false), &mut l, &s, 0.1, Rates::default()).is_empty());
    assert!(snap.active());
    let out = ticks(&mut snap, &mut l, &s, 0.1, 1.0);
    assert_eq!(out, [Input::MouseMove { x: 330, y: 535 }]);
    // Alt again: off. It ended on B: the mouse is let go there (the usual
    // rules: the system pointer on B, the driver has it there already).
    assert!(snap.route(key(ALT, true), &mut l, &s, 1.0, Rates::default()).is_empty());
    assert!(snap.route(key(ALT, false), &mut l, &s, 1.1, Rates::default()).is_empty());
    assert!(!snap.active() && !l.snapping());
    assert_eq!(snap.take_hide_pointer(), Some(false));
    assert_eq!(snap.snapper.pointer(), None);
    assert!(!l.captured());
    assert_eq!((l.take_grab(), l.take_warp(), l.pointer_in_window()), (Some(false), Some((330, 535)), (330, 535)));
    // A click there goes to B, as any click of the free mouse.
    let down = snap.route(Input::MouseButton { button: Button::Left, down: true, x: 330, y: 535 }, &mut l, &s, 1.2, Rates::default());
    assert_eq!(down, [Input::MouseButton { button: Button::Left, down: true, x: 330, y: 535 }]);
    snap.route(Input::MouseButton { button: Button::Left, down: false, x: 330, y: 535 }, &mut l, &s, 1.25, Rates::default());
    assert_eq!(snap.motion((1.0, 0.0)), None);
    // On a third time.
    snap.route(key(ALT, true), &mut l, &s, 2.0, Rates::default());
    assert!(snap.active());
}

#[test]
fn with_nothing_of_the_driver_s_to_snap_to_alt_is_the_driver_s() {
    let (mut l, mut s) = captured();
    s.interactables.clear();
    let mut snap = SnapInput::default();
    // (Only the capture view.)
    assert_eq!(snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default()).len(), 1);
    assert!(!snap.active() && !l.snapping());
    assert_eq!(snap.route(key(ALT, false), &mut l, &s, 0.1, Rates::default()).len(), 1);
}

#[test]
fn esc_and_ctrl_turn_the_mode_off() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 0.1, Rates::default());
    ticks(&mut snap, &mut l, &s, 0.1, 1.0);
    // Esc: off, and not the driver's.
    assert!(snap.route(key(ESC, true), &mut l, &s, 1.0, Rates::default()).is_empty());
    assert!(snap.route(key(ESC, false), &mut l, &s, 1.1, Rates::default()).is_empty());
    assert!(!snap.active());
    // Esc without the mode is the driver's.
    assert_eq!(snap.route(key(ESC, true), &mut l, &s, 1.2, Rates::default()).len(), 1);
    snap.route(key(ESC, false), &mut l, &s, 1.3, Rates::default());
    // Ctrl (which lets go of the mouse) goes on to the layout; off first.
    snap.route(key(ALT, true), &mut l, &s, 2.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 2.1, Rates::default());
    assert!(snap.active());
    let out = snap.route(key(CTRL, true), &mut l, &s, 2.2, Rates::default());
    assert_eq!(out.last(), Some(&key(CTRL, true)));
    assert!(!snap.snapper.snapping());
    // With Ctrl held, Alt is a key for the driver (Ctrl+Alt+...).
    let out = snap.route(key(ALT, true), &mut l, &s, 2.3, Rates::default());
    assert_eq!(out.len(), 1);
    assert!(!snap.snapper.snapping());
    assert_eq!(snap.route(key(ALT, false), &mut l, &s, 2.4, Rates::default()).len(), 1);
}

#[test]
fn another_key_with_alt_held_is_a_shortcut_for_the_driver() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    assert!(snap.active());
    let vk = |i: &Input| match i {
        Input::Key { vk, down, .. } | Input::WorldKey { vk, down, .. } => (*vk, *down),
        other => panic!("{other:?}"),
    };
    let out = snap.route(key(0x41, true), &mut l, &s, 0.1, Rates::default());
    assert_eq!(out.iter().map(vk).collect::<Vec<_>>(), [(ALT, true), (0x41, true)]);
    // Not snapping after all.
    assert!(!snap.active() && !l.snapping());
    let out = snap.route(key(0x41, false), &mut l, &s, 0.2, Rates::default());
    assert_eq!(out.iter().map(vk).collect::<Vec<_>>(), [(0x41, false)]);
    let out = snap.route(key(ALT, false), &mut l, &s, 0.3, Rates::default());
    assert_eq!(out.iter().map(vk).collect::<Vec<_>>(), [(ALT, false)]);
}

#[test]
fn turned_on_again_it_resumes_on_the_last_element() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    // Left, onto A.
    steer(&mut snap, &mut l, &s, 0.0, 1.0, (-4.0, 0.0), 0.05);
    assert_eq!(snap.snapper.target(), Some((130.0, 535.0)));
    snap.route(key(ALT, true), &mut l, &s, 1.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 1.0, Rates::default());
    // The world cursor moves on (the mouse took it up again).
    assert_eq!(snap.motion((5.0, 5.0)), None);
    l.motion((50.0, 0.0));
    // On again: on A, not B (the nearer to the world cursor).
    snap.route(key(ALT, true), &mut l, &s, 2.0, Rates::default());
    assert_eq!(snap.snapper.target(), Some((130.0, 535.0)));
    assert_eq!(snap.snapper.pointer(), Some((130.0, 535.0)));
}

#[test]
fn clicks_in_the_mode_never_reach_the_world() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    let mut out = ticks(&mut snap, &mut l, &s, 0.0, 1.0);
    // On B: a click is B's.
    let c = click(&mut snap, &mut l, &s, 1.0);
    assert_eq!(c, [Input::MouseMove { x: 330, y: 535 }, Input::MouseButton { button: Button::Left, down: true, x: 330, y: 535 }, Input::MouseButton { button: Button::Left, down: false, x: 330, y: 535 }]);
    out.extend(c);
    // Wherever the mouse goes, clicks and keys reach the interface only.
    for (k, d) in [(0.0, -6.0), (6.0, 0.0), (-6.0, 6.0), (0.0, 6.0), (-6.0, 0.0)].into_iter().enumerate() {
        let t0 = 1.1 + k as f64;
        out.extend(steer(&mut snap, &mut l, &s, t0, t0 + 0.2, d, t0 + 0.1));
        if snap.snapper.target_id() == Some(CAPTURE_ID) {
            continue;
        }
        out.extend(click(&mut snap, &mut l, &s, t0 + 0.2));
        out.extend(snap.route(key(0x41, true), &mut l, &s, t0 + 0.3, Rates::default()));
        out.extend(snap.route(key(0x41, false), &mut l, &s, t0 + 0.35, Rates::default()));
    }
    assert!(no_world(&out), "{out:?}");
    // (Clicks went to A or B only.)
    for i in &out {
        if let Input::MouseButton { x, y, .. } = i {
            assert!([(130, 535), (330, 535)].contains(&(*x, *y)), "{i:?}");
        }
    }
}

#[test]
fn the_capture_view_is_a_target_and_clicking_it_captures_the_mouse() {
    // Captured: up from B to the map, a click there: the mode is off, the
    // world cursor the pointer again, nothing for the driver.
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    ticks(&mut snap, &mut l, &s, 0.0, 1.0);
    let out = steer(&mut snap, &mut l, &s, 1.0, 2.0, (0.0, -6.0), 1.05);
    assert_eq!(snap.snapper.target_id(), Some(CAPTURE_ID));
    assert_eq!(snap.snapper.target(), Some((400.0, 300.0)));
    // (No hover for the world.)
    assert!(out.is_empty(), "{out:?}");
    assert!(click(&mut snap, &mut l, &s, 2.0).is_empty());
    assert!(!snap.active() && l.captured() && !l.snapping());
    assert_eq!(snap.snapper.pointer(), None);
    // Its motion moves the world cursor again.
    assert_eq!(snap.motion((1.0, 0.0)), None);

    // Free: the mode on anyway (any screen); a click on the capture view
    // captures the mouse.
    let (mut l, s) = free_capturing();
    l.route(Input::MouseMove { x: 300, y: 560 });
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    assert!(snap.active());
    assert_eq!(snap.snapper.target(), Some((330.0, 535.0)));
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    ticks(&mut snap, &mut l, &s, 0.0, 1.0);
    steer(&mut snap, &mut l, &s, 1.0, 2.0, (0.0, -6.0), 1.05);
    assert_eq!(snap.snapper.target_id(), Some(CAPTURE_ID));
    assert!(click(&mut snap, &mut l, &s, 2.0).is_empty());
    assert!(l.captured() && !snap.active());
    assert_eq!(l.take_grab(), Some(true));
}

#[test]
fn snapping_works_with_the_mouse_free_and_hands_the_pointer_back_where_it_ended() {
    let s = scene();
    let mut l = Layout::default();
    frame(&mut l, &s);
    assert!(!l.captured());
    // The pointer at the window's left edge: the driver scrolls.
    assert_eq!(l.route(Input::MouseMove { x: 0, y: 300 }), [Input::MouseMove { x: 0, y: 300 }, Input::Edge { x: -1, y: 0 }]);
    let mut snap = SnapInput::default();
    // On: no edge any more.
    let out = snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    assert_eq!(out, [Input::Edge { x: 0, y: 0 }]);
    assert!(snap.active() && l.snapping());
    assert_eq!(snap.snapper.target(), Some((130.0, 535.0)));
    snap.route(key(ALT, false), &mut l, &s, 0.0, Rates::default());
    // The system pointer's own motion is not the layout's meanwhile (no
    // edge, no divider, no moving it back).
    assert!(snap.route(Input::MouseMove { x: 0, y: 599 }, &mut l, &s, 0.1, Rates::default()).is_empty());
    assert_eq!(l.take_warp(), None);
    steer(&mut snap, &mut l, &s, 0.1, 1.0, (6.0, 0.0), 0.15);
    assert_eq!(snap.snapper.target(), Some((330.0, 535.0)));
    // Off: the system pointer shows where the snapping pointer ended.
    snap.route(key(ALT, true), &mut l, &s, 1.0, Rates::default());
    assert!(!snap.active() && !l.snapping());
    assert_eq!(snap.snapper.pointer(), None);
    assert_eq!(l.take_warp(), Some((330, 535)));
    assert_eq!(l.pointer_in_window(), (330, 535));
    // Under the usual rules again: at the edge, the driver scrolls.
    assert_eq!(l.route(Input::MouseMove { x: 0, y: 300 }), [Input::MouseMove { x: 0, y: 300 }, Input::Edge { x: -1, y: 0 }]);
}

#[test]
fn losing_the_capture_keeps_the_mode_and_losing_the_focus_ends_it() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    // (A screen opening lets go of the mouse.)
    l.released();
    ticks(&mut snap, &mut l, &s, 0.0, 0.1);
    assert!(snap.active());
    snap.focus_lost(&mut l);
    assert!(!snap.active() && !l.snapping());
    assert_eq!(snap.snapper.pointer(), None);
    // (Alt's release after that is nothing; the next press turns it on.)
    assert!(snap.route(key(ALT, false), &mut l, &s, 0.2, Rates::default()).is_empty());
    snap.route(key(ALT, true), &mut l, &s, 0.3, Rates::default());
    assert!(snap.active());
}

#[test]
fn in_the_mode_only_the_snapping_pointer_and_its_ghost_are_drawn() {
    use crate::cursor::SnapMode;
    let compiled = render_desc::Description::parse(
        r#"
        grid = { layout = "orthogonal", step = [8, 8] }
        [images.p]
        source = "pointer:255,255,255,255:14x22"
        [images.g]
        source = "pointer:255,255,255,128:14x22"
        [sheets.p]
        image = "p"
        cell = [14, 22]
        group = "Viewer@cursors"
        [sheets.g]
        image = "g"
        cell = [14, 22]
        group = "Viewer@cursor_ghost"
        [sheets.q]
        image = "p"
        cell = [14, 22]
        group = "Viewer@pointer"
        "#,
    )
    .unwrap()
    .compile(&NoFiles, 256)
    .unwrap();
    let (mut l, s) = captured();
    let f = frame(&mut l, &s);
    let mut snap = SnapInput::default();
    assert!(snap.sprites(&compiled, 7).is_empty());
    assert!(!snap.snap_mode_active());
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    assert!(snap.snap_mode_active());
    // The cursor's rules: no system pointer, no world cursor, nothing else.
    let world_cursor: Vec<(i32, i32)> = f.markers.iter().filter_map(|m| if let crate::layout::Marker::Cursor(p) = m { Some(*p) } else { None }).collect();
    assert!(!world_cursor.is_empty());
    let facts = crate::cursor::Facts { captured: true, snap_mode: snap.snap_mode_active(), world_cursor, cursor_view: l.world_cursor_view(), pointer: (400, 300), pointer_inside: true, ..Default::default() };
    let plan = crate::cursor::Machine::default().step(&facts);
    assert!(plan.hidden && plan.marks == crate::cursor::Marks::default(), "{plan:?}");
    // The snapping pointer: the driver's arrow, where it is in window
    // pixels, in the camera on top; its ghost (below it) on B, where it
    // flies.
    let sprites = snap.sprites(&compiled, 7);
    assert_eq!(sprites.len(), 2);
    assert_eq!(sprites[0].set, compiled.set(ARROW_GHOST_SPRITE).unwrap().0);
    assert_eq!((sprites[0].x, sprites[0].y), (330, 535));
    assert_eq!(sprites[1].set, compiled.set(ARROW_SPRITE).unwrap().0);
    assert_eq!((sprites[1].x, sprites[1].y, sprites[1].view, sprites[1].screen), (400, 300, Some(7), true));
    assert!(sprites.iter().all(|p| p.view == Some(7) && p.screen));
    // Landed: the ghost under the pointer is not drawn.
    ticks(&mut snap, &mut l, &s, 0.0, 1.0);
    let sprites = snap.sprites(&compiled, 7);
    assert_eq!(sprites.len(), 1);
    assert_eq!((sprites[0].x, sprites[0].y), (330, 535));
    // Off (landed): the cursor's rules again.
    snap.route(key(ALT, false), &mut l, &s, 1.0, Rates::default());
    snap.route(key(ALT, true), &mut l, &s, 1.1, Rates::default());
    assert!(!snap.snap_mode_active());
}

#[test]
fn targets_in_parts_drawn_elsewhere_are_where_they_are_drawn() {
    // (No part moved here: the targets are the driver's, and the capture
    // view.)
    let (l, s) = captured();
    let t = SnapInput::targets(&s, &l);
    assert_eq!(t.iter().map(|x| x.id).collect::<Vec<_>>(), [1, 2, CAPTURE_ID]);
    assert!(l.moved_parts().is_empty());
}

struct NoFiles;
impl render_desc::ImageLoader for NoFiles {
    fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
        Err(format!("no file {source}"))
    }
}
