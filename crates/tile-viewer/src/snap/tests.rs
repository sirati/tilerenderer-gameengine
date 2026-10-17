use super::*;
use scene::{Scene, View, ViewState};

const IDENTITY: ((i32, i32), (u32, u32)) = ((0, 0), (1, 1));
const WINDOW: (u32, u32) = (1000, 800);
const TICK: f64 = 1.0 / 120.0;

fn button(id: u64, rect: [i32; 4]) -> Interactable {
    Interactable { rect, kind: InteractableKind::Button, id }
}

fn item(id: u64, rect: [i32; 4]) -> Interactable {
    Interactable { rect, kind: InteractableKind::ListItem, id }
}

fn list(id: u64, rect: [i32; 4], step: i32) -> Interactable {
    Interactable { rect, kind: InteractableKind::List { scroll: rect, step }, id }
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
        out.extend(s.tick(t, targets, WINDOW, Rates::default()).into_iter().map(|a| (t, a)));
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

// --- the hidden pointer ---

#[test]
fn hidden_pointer_follows_the_mouse_and_rests_ahead_by_its_speed_over_friction() {
    let mut g = Glide::at((0.0, 0.0));
    // 10 counts a tick at 120 ticks a second: 1200 px/s.
    for _ in 0..60 {
        g.push((10.0, 0.0));
        g.step(TICK);
    }
    assert_eq!(g.pos, (600.0, 0.0));
    assert!((g.vel.0 - 1200.0).abs() < 1.0, "{:?}", g.vel);
    // A throw at 1200 px/s comes to rest 200 px on.
    assert!((g.rest().0 - 800.0).abs() < 0.5, "{:?}", g.rest());
    // Gliding keeps the rest point and gets there.
    let rest = g.rest();
    for k in 0..360 {
        g.step(TICK);
        if k == 20 {
            assert!((g.rest().0 - rest.0).abs() < 1e-6);
            assert!(g.pos.0 > 600.0 && g.pos.0 < rest.0);
        }
    }
    assert!((g.pos.0 - rest.0).abs() < 1e-3 && g.vel.0.abs() < 0.01);
}

#[test]
fn slow_moves_rest_close_and_fast_ones_far() {
    let throw = |counts: f64| {
        let mut g = Glide::at((0.0, 0.0));
        for _ in 0..24 {
            g.push((counts, 0.0));
            g.step(TICK);
        }
        g.rest().0 - g.pos.0
    };
    // 0.5 counts a tick (60 px/s): 10 px ahead; 10 (1200 px/s): about 200.
    let (slow, fast) = (throw(0.5), throw(10.0));
    assert!(slow < 11.0 && fast > 190.0, "{slow} {fast}");
}

#[test]
fn hidden_pointer_stops_at_the_window_edges() {
    let mut g = Glide::at((5.0, 5.0));
    g.push((-20.0, 0.0));
    g.step(TICK);
    g.clamp(WINDOW);
    assert_eq!((g.pos.0, g.vel.0), (0.0, 0.0));
    assert!(g.vel.1 == 0.0 && g.pos.1 == 5.0);
    g.push((2000.0, 2000.0));
    g.clamp(WINDOW);
    assert_eq!(g.pos, (999.0, 799.0));
}

// --- the visible pointer ---

/// A spring jump of 300 px from rest: when it first reaches the target, how
/// far it overshoots, its speed after one update interval against its top
/// speed, and when it comes to rest.
fn jump(rate: f64) -> (f64, f64, f64, f64) {
    let omega = Spring::omega(rate);
    let mut s = Spring::at((0.0, 0.0));
    s.target = (300.0, 0.0);
    let (mut t, mut arrival, mut peak, mut top, mut at_update, mut rest) = (0.0, None, 0.0f64, 0.0f64, 0.0, None);
    let dt = 1.0 / 1000.0;
    while t < 3.0 {
        s.step(dt, omega);
        t += dt;
        if arrival.is_none() && s.pos.0 >= 300.0 {
            arrival = Some(t);
        }
        peak = peak.max(s.pos.0);
        top = top.max(s.vel.0);
        if (t - 1.0 / rate).abs() < dt / 2.0 {
            at_update = s.vel.0;
        }
        if rest.is_none() && s.settle() {
            rest = Some(t);
        }
    }
    (arrival.unwrap(), (peak - 300.0) / 300.0, at_update / top, rest.unwrap())
}

#[test]
fn the_pointer_arrives_at_a_third_past_the_update_interval_overshooting_slightly() {
    let (arrival, overshoot, still_moving, rest) = jump(3.0);
    // 4/3 of 1/3 s.
    assert!((arrival - 4.0 / 9.0).abs() < 0.01, "{arrival}");
    // e^(−πζ/√(1−ζ²)) = 2.8 % at ζ 0.75.
    assert!(overshoot > 0.02 && overshoot < 0.04, "{overshoot}");
    // Still going at over 40 % of its top speed when the next update comes.
    assert!(still_moving > 0.4, "{still_moving}");
    // Settled back within about three intervals.
    assert!(rest > arrival && rest < 1.2, "{rest}");
    // The timing follows the rate.
    let (arrival, overshoot, _, _) = jump(6.0);
    assert!((arrival - 4.0 / 18.0).abs() < 0.01 && (overshoot - 0.028).abs() < 0.008, "{arrival} {overshoot}");
}

#[test]
fn a_new_target_steers_the_pointer_without_stopping_it() {
    let omega = Spring::omega(3.0);
    let mut s = Spring::at((0.0, 0.0));
    s.target = (300.0, 0.0);
    for _ in 0..40 {
        s.step(TICK, omega);
    }
    let before = s;
    s.target = (300.0, 200.0);
    s.step(TICK, omega);
    // Its speed carries on (only the new pull bends it).
    assert!(s.vel.0 > 0.9 * before.vel.0 && s.pos.0 > before.pos.0);
    assert!(s.vel.1 > 0.0 && s.vel.1 < 0.3 * before.vel.0, "{s:?} {before:?}");
    for _ in 0..240 {
        s.step(TICK, omega);
    }
    assert!(s.settle() && s.pos == (300.0, 200.0));
}

// --- snapping ---

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
    // A list's own step wins, scaled with the screen; a lone entry has none.
    let t = targets(&[list(1, [0, 0, 100, 100], 12), item(2, [0, 300, 50, 10])], ((0, 0), (2, 1)), WINDOW);
    assert_eq!(t[0].kind, TargetKind::List { scroll: [0.0, 0.0, 200.0, 200.0], step: 24.0, has_items: false });
    assert_eq!(t[1].kind, TargetKind::ListItem { list: None });
    // Without entries or a step: a guess.
    let t = targets(&[list(1, [0, 0, 100, 100], 0)], IDENTITY, WINDOW);
    assert!(matches!(t[0].kind, TargetKind::List { step, .. } if step == DEFAULT_SCROLL_STEP));
}

#[test]
fn the_nearest_is_the_one_under_the_point_else_the_closest_edge() {
    let t = row();
    assert_eq!(nearest(&t, (130.0, 105.0)), Some(0));
    // 160 px right of A's edge, 140 left of B's: B.
    assert_eq!(nearest(&t, (160.0 + 140.0 - 1.0 + 20.0, 110.0)), Some(1));
    assert_eq!(nearest(&t, (215.0, 400.0)), Some(0));
    assert_eq!(nearest(&t, (225.0, 400.0)), Some(1));
    // A list with entries is no target itself, its entries are.
    let t = with_list();
    assert_eq!(nearest(&t, (600.0, 101.0)), Some(2));
    // One without is.
    let t = targets(&[list(1, [0, 0, 100, 100], 0)], IDENTITY, WINDOW);
    assert_eq!(nearest(&t, (50.0, 50.0)), Some(0));
    assert_eq!(nearest(&[], (0.0, 0.0)), None);
}

#[test]
fn entering_snaps_to_the_nearest_and_starts_the_hidden_pointer_there() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (200.0, 300.0), &t, Rates::default());
    assert!(s.snapping());
    assert_eq!(s.target(), Some((120.0, 110.0)));
    assert_eq!(s.glide().pos, (120.0, 110.0));
    // The visible pointer starts where the pointer was and flies there.
    assert_eq!(s.pointer(), Some((200.0, 300.0)));
    assert_eq!(s.ghost(), Some((120.0, 110.0)));
}

#[test]
fn the_target_changes_only_at_updates_and_the_ghost_shows_it_at_once() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    // 200 counts right over 0.1 s: rests well past B, short of C.
    let actions = run(&mut s, &t, 0.0, 0.05, (8.0, 0.0), |_| true);
    // (Only A's hover: the pointer started on it.)
    assert!(actions.iter().all(|(_, a)| *a == Action::Move((120.0, 110.0))));
    assert_eq!(s.target(), Some((120.0, 110.0)));
    // The ghost: at the next of its 30 frames a second.
    assert_ne!(s.ghost(), Some((120.0, 110.0)));
    let ghost = s.ghost().unwrap();
    run(&mut s, &t, 0.05, 0.32, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((120.0, 110.0)));
    // The update at 1/3 s.
    run(&mut s, &t, 0.32, 0.34, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some(ghost));
    assert_eq!(ghost, (320.0, 110.0));
}

#[test]
fn updates_come_at_the_configured_rate() {
    let t = row();
    let rates = Rates { updates_per_second: 10.0, ghost_fps: 30.0 };
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, rates);
    s.motion((200.0, 0.0));
    s.tick(0.05, &t, WINDOW, rates);
    assert_eq!(s.target(), Some((120.0, 110.0)));
    s.tick(0.1, &t, WINDOW, rates);
    assert_eq!(s.target(), Some((320.0, 110.0)));
    // From the settings, kept sane.
    let r = Rates::of(&viewer_settings::Input { snap_updates_per_second: 0.0, snap_ghost_fps: 1000.0 });
    assert_eq!((r.updates_per_second, r.ghost_fps), (0.5, 240.0));
    assert_eq!(Rates::default(), Rates { updates_per_second: 3.0, ghost_fps: 30.0 });
}

#[test]
fn hover_is_sent_once_the_pointer_rests_on_the_element_which_becomes_the_origin() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    // Already on A: at rest at once.
    let actions = run(&mut s, &t, 0.0, 0.02, (0.0, 0.0), |_| false);
    assert_eq!(moves(&actions), [(TICK, (120.0, 110.0))]);
    let actions = run(&mut s, &t, 0.02, 2.0, (8.0, 0.0), |t| t < 0.1);
    let m = moves(&actions);
    assert_eq!(m.len(), 1, "{m:?}");
    let (at, p) = m[0];
    assert_eq!(p, (320.0, 110.0));
    // Flying from the update at 1/3 s: arrives ~0.44 s later, rests after.
    assert!(at > 1.0 / 3.0 + 0.44 && at < 1.0 / 3.0 + 1.2, "{at}");
    // Nothing else went to the driver while it flew.
    assert!(actions.iter().all(|(_, a)| matches!(a, Action::Move(_))));
    // The hidden pointer goes on from B now.
    assert!((s.glide().pos.0 - 320.0).abs() < 0.5, "{:?}", s.glide());
}

#[test]
fn moving_again_in_flight_steers_the_same_pointer_to_the_new_target() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.34, (8.0, 0.0), |t| t < 0.1);
    assert_eq!(s.target(), Some((320.0, 110.0)));
    // Half way, a big throw right: C at the next update, the speed kept.
    let mut actions = run(&mut s, &t, 0.34, 0.6, (0.0, 0.0), |_| false);
    let before = *s.spring();
    assert!(before.vel.0 > 100.0 && before.pos.0 < 320.0, "{before:?}");
    actions.extend(run(&mut s, &t, 0.6, 0.67, (16.0, 0.0), |_| true));
    assert_eq!(s.target(), Some((920.0, 110.0)));
    assert!(s.spring().vel.0 >= before.vel.0 * 0.8, "{:?}", s.spring());
    actions.extend(run(&mut s, &t, 0.67, 3.0, (0.0, 0.0), |_| false));
    // B never showed its hover; C did, once.
    assert_eq!(moves(&actions).iter().map(|m| m.1).collect::<Vec<_>>(), [(920.0, 110.0)]);
}

#[test]
fn a_click_clicks_the_element_snapped_to() {
    let t = row();
    let mut s = Snapper::default();
    assert_eq!(s.click(Button::Left, true), None);
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.4, (8.0, 0.0), |t| t < 0.1);
    // In flight to B: the click goes to B, the pointer is put there.
    assert_ne!(s.pointer(), Some((320.0, 110.0)));
    let down = s.click(Button::Left, true).unwrap();
    let at = (320.0, 110.0);
    assert_eq!(down, [Action::Move(at), Action::Button { button: Button::Left, down: true, at }]);
    assert_eq!(s.pointer(), Some(at));
    // At rest there, hover sent: nothing more.
    assert!(moves(&run(&mut s, &t, 0.4, 0.6, (0.0, 0.0), |_| false)).is_empty());
    // The release goes where the press went, even after snapping ended.
    s.leave();
    s.stop();
    assert_eq!(s.click(Button::Left, false), Some(vec![Action::Button { button: Button::Left, down: false, at }]));
    assert_eq!(s.click(Button::Left, false), None);
}

#[test]
fn leaving_keeps_the_pointer_where_it_is_until_the_mouse_moves() {
    let t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    run(&mut s, &t, 0.0, 0.5, (8.0, 0.0), |t| t < 0.1);
    let flying = s.pointer().unwrap();
    let left = s.leave();
    let p = (flying.0.round(), flying.1.round());
    // In flight: the driver gets the pointer where it stays.
    assert_eq!(left, [Action::Move(p)]);
    assert!(!s.snapping() && s.ghost().is_none());
    assert_eq!(s.pointer(), Some(p));
    assert!(s.tick(0.6, &t, WINDOW, Rates::default()).is_empty());
    assert_eq!(s.pointer(), Some(p));
    assert_eq!(s.click(Button::Right, true), Some(vec![Action::Move(p), Action::Button { button: Button::Right, down: true, at: p }]));
    assert_eq!(s.wheel(120), Some(vec![Action::Wheel { delta: 120, at: p }]));
    // The mouse takes it up again.
    assert!(!s.motion((1.0, 0.0)));
    assert_eq!(s.pointer(), None);
    assert_eq!(s.wheel(120), None);
    // At rest on an element whose hover it showed: nothing to send.
    s.enter(1.0, (120.0, 110.0), &t, Rates::default());
    s.tick(1.01, &t, WINDOW, Rates::default());
    assert!(s.leave().is_empty());
}

#[test]
fn the_target_follows_its_element_when_it_moves() {
    let mut t = row();
    let mut s = Snapper::default();
    s.enter(0.0, (120.0, 110.0), &t, Rates::default());
    s.tick(TICK, &t, WINDOW, Rates::default());
    t[0].rect[1] += 50.0;
    let out = run(&mut s, &t, TICK, 2.0, (0.0, 0.0), |_| false);
    assert_eq!(s.target(), Some((120.0, 160.0)));
    assert_eq!(moves(&out).last().map(|m| m.1), Some((120.0, 160.0)));
}

// --- lists ---

#[test]
fn landing_in_the_list_scrolls_it_smoothly_and_the_pointer_stays() {
    let t = with_list();
    let mut s = Snapper::default();
    // On the third entry (600, 150).
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    assert_eq!(s.target(), Some((600.0, 150.0)));
    let mut actions = run(&mut s, &t, 0.0, 0.3, (0.0, 3.0), |t| t < 0.1);
    // (The rest point no longer moves while it glides.)
    let by = s.glide().rest().1 - 150.0;
    assert!(by > 20.0 && by < 150.0, "{by}");
    actions.extend(run(&mut s, &t, 0.3, 1.5, (0.0, 0.0), |_| false));
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
    assert_eq!(wheel.len() as f64, (by / 20.0).round(), "{wheel:?}");
    assert!(wheel.iter().all(|w| w.1 == -120));
    // Spread out, not all at once.
    assert!(wheel.windows(2).all(|w| w[1].0 - w[0].0 >= 1.0 / MAX_NOTCHES_PER_SECOND - 1e-9));
    assert!(wheel.last().unwrap().0 - wheel[0].0 > 0.05);
    // The pointer stayed; the hidden one is level with it again.
    assert_eq!(s.pointer(), Some((600.0, 150.0)));
    assert_eq!(s.glide().pos.1, 150.0);
    // Once scrolled, the driver gets the pointer (for the hover of what is
    // under it now).
    let after = moves(&actions).into_iter().filter(|m| m.0 > wheel.last().unwrap().0).collect::<Vec<_>>();
    assert_eq!(after.iter().map(|m| m.1).collect::<Vec<_>>(), [(600.0, 150.0)]);
}

#[test]
fn moving_up_the_list_scrolls_it_up() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 350.0), &t, Rates::default());
    let actions = run(&mut s, &t, 0.0, 1.5, (0.0, -1.0), |t| t < 0.1);
    let deltas: Vec<i32> = actions.iter().filter_map(|(_, a)| if let Action::Wheel { delta, .. } = a { Some(*delta) } else { None }).collect();
    assert!(!deltas.is_empty() && deltas.iter().all(|&d| d == 120), "{deltas:?}");
}

#[test]
fn an_update_landing_outside_the_list_leaves_it() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    // Right, past the list's edge at 700: the button at (850, 210).
    let actions = run(&mut s, &t, 0.0, 0.4, (8.0, 0.0), |t| t < 0.1);
    assert_eq!(s.target(), Some((850.0, 210.0)));
    assert!(actions.iter().all(|(_, a)| !matches!(a, Action::Wheel { .. })));
}

#[test]
fn the_entry_under_the_pointer_is_snapped_to_after_the_list_scrolled() {
    let t = with_list();
    let mut s = Snapper::default();
    s.enter(0.0, (600.0, 150.0), &t, Rates::default());
    s.tick(TICK, &t, WINDOW, Rates::default());
    // The driver numbers entries by what they show: a scroll gives new ids.
    let scrolled: Vec<Target> = t.iter().map(|x| if x.id >= 100 { Target { id: x.id + 1000, ..*x } } else { *x }).collect();
    s.tick(2.0 * TICK, &scrolled, WINDOW, Rates::default());
    assert_eq!(s.target(), Some((600.0, 150.0)));
    // Still in the list: landing in it scrolls.
    let actions = run(&mut s, &scrolled, 2.0 * TICK, 1.5, (0.0, 1.0), |t| t < 0.1);
    assert!(actions.iter().any(|(_, a)| matches!(a, Action::Wheel { .. })));
}

#[test]
fn a_list_without_entries_is_snapped_to_and_scrolled_at_its_middle() {
    let t = targets(&[list(1, [100, 100, 200, 400], 10)], IDENTITY, WINDOW);
    let mut s = Snapper::default();
    s.enter(0.0, (0.0, 0.0), &t, Rates::default());
    assert_eq!(s.target(), Some((200.0, 300.0)));
    run(&mut s, &t, 0.0, 2.0, (0.0, 0.0), |_| false);
    let actions = run(&mut s, &t, 2.0, 3.5, (0.0, 1.0), |t| t < 2.1);
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
    // Most within a third of a second (three time constants).
    let (_, sent) = total(4.0);
    assert!(sent[2] < 0.34, "{sent:?}");
    // Many: at most the rate.
    let (sum, sent) = total(20.0);
    assert_eq!(sum, 20);
    assert!(*sent.last().unwrap() >= 19.0 / MAX_NOTCHES_PER_SECOND);
}

// --- with the layout ---

const ALT: u32 = 0x12;
const CTRL: u32 = 0x11;

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

/// A layout with its main view following the world cursor, the mouse
/// captured.
fn captured() -> (Layout, Scene) {
    let s = scene();
    let mut l = Layout::default();
    l.configure("main=pointer").unwrap();
    frame(&mut l, &s);
    l.route(Input::MouseButton { button: Button::Left, down: true, x: 400, y: 300 });
    l.route(Input::MouseButton { button: Button::Left, down: false, x: 400, y: 300 });
    assert!(l.captured());
    frame(&mut l, &s);
    (l, s)
}

fn key(vk: u32, down: bool) -> Input {
    Input::Key { vk, down, text: None }
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

#[test]
fn alt_snaps_only_while_the_mouse_is_captured() {
    let s = scene();
    let mut l = Layout::default();
    frame(&mut l, &s);
    let mut snap = SnapInput::default();
    assert_eq!(snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default()), [key(ALT, true)]);
    assert!(!snap.snapper.snapping());
    assert_eq!(snap.route(key(ALT, false), &mut l, &s, 0.1, Rates::default()), [key(ALT, false)]);
    assert_eq!(snap.take_hide_pointer(), None);
}

#[test]
fn snapping_hovers_and_clicks_the_driver_s_buttons() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    // Alt: the driver does not get it; the pointer snaps to B, the nearer to
    // the world cursor in the middle of the window.
    assert!(snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default()).is_empty());
    assert_eq!(snap.take_hide_pointer(), Some(true));
    assert_eq!(snap.snapper.target(), Some((330.0, 535.0)));
    // Key repeat changes nothing.
    assert!(snap.route(key(ALT, true), &mut l, &s, 0.05, Rates::default()).is_empty());
    // The mouse moves the hidden pointer, not the world cursor.
    assert_eq!(snap.motion((-1.0, 0.0)), Some(vec![]));
    let out = ticks(&mut snap, &mut l, &s, 0.0, 2.0);
    assert_eq!(out, [Input::MouseMove { x: 330, y: 535 }]);
    // A click goes to B.
    let down = snap.route(Input::MouseButton { button: Button::Left, down: true, x: 0, y: 0 }, &mut l, &s, 2.0, Rates::default());
    assert_eq!(down, [Input::MouseMove { x: 330, y: 535 }, Input::MouseButton { button: Button::Left, down: true, x: 330, y: 535 }]);
    let up = snap.route(Input::MouseButton { button: Button::Left, down: false, x: 0, y: 0 }, &mut l, &s, 2.1, Rates::default());
    assert_eq!(up, [Input::MouseButton { button: Button::Left, down: false, x: 330, y: 535 }]);
    // So does the wheel.
    let wheel = snap.route(Input::Wheel { delta: -120, x: 0, y: 0 }, &mut l, &s, 2.2, Rates::default());
    assert_eq!(wheel, [Input::Wheel { delta: -120, x: 330, y: 535 }]);
    // Releasing Alt: the pointer stays on B (the driver has it there).
    assert!(snap.route(key(ALT, false), &mut l, &s, 2.3, Rates::default()).is_empty());
    assert_eq!(snap.take_hide_pointer(), Some(false));
    assert!(!snap.snapper.snapping());
    let click = snap.route(Input::MouseButton { button: Button::Left, down: true, x: 0, y: 0 }, &mut l, &s, 2.4, Rates::default());
    assert_eq!(click[1], Input::MouseButton { button: Button::Left, down: true, x: 330, y: 535 });
    snap.route(Input::MouseButton { button: Button::Left, down: false, x: 0, y: 0 }, &mut l, &s, 2.5, Rates::default());
    // Moving the mouse gives it back to the world cursor.
    assert_eq!(snap.motion((1.0, 0.0)), None);
    assert_eq!(snap.snapper.pointer(), None);
}

#[test]
fn ctrl_ends_snapping_and_keeps_it_from_starting() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    ticks(&mut snap, &mut l, &s, 0.0, 2.0);
    // Ctrl (which lets go of the mouse) goes on to the layout; snapping
    // ends, the pointer where it was.
    let out = snap.route(key(CTRL, true), &mut l, &s, 2.0, Rates::default());
    assert_eq!(out, [key(CTRL, true)]);
    assert!(!snap.snapper.snapping());
    assert_eq!(snap.snapper.pointer(), Some((330.0, 535.0)));
    // Alt let go after: nothing for the driver, which never had it.
    assert!(snap.route(key(ALT, false), &mut l, &s, 2.1, Rates::default()).is_empty());
    // With Ctrl held, Alt is a key for the driver (Ctrl+Alt+...).
    let out = snap.route(key(ALT, true), &mut l, &s, 2.2, Rates::default());
    assert_eq!(out.len(), 1);
    assert!(!snap.snapper.snapping());
}

#[test]
fn another_key_with_alt_held_reaches_the_driver_with_alt() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    let vk = |i: &Input| match i {
        Input::Key { vk, down, .. } | Input::WorldKey { vk, down, .. } => (*vk, *down),
        other => panic!("{other:?}"),
    };
    let out = snap.route(key(0x41, true), &mut l, &s, 0.1, Rates::default());
    assert_eq!(out.iter().map(vk).collect::<Vec<_>>(), [(ALT, true), (0x41, true)]);
    let out = snap.route(key(0x41, false), &mut l, &s, 0.2, Rates::default());
    assert_eq!(out.iter().map(vk).collect::<Vec<_>>(), [(0x41, false)]);
    // Still snapping; Alt's release ends it and reaches the driver.
    assert!(snap.snapper.snapping());
    let out = snap.route(key(ALT, false), &mut l, &s, 0.3, Rates::default());
    assert_eq!(out.iter().filter(|i| matches!(i, Input::Key { .. } | Input::WorldKey { .. })).map(vk).collect::<Vec<_>>(), [(ALT, false)]);
}

#[test]
fn losing_the_capture_or_the_focus_ends_snapping() {
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    l.released();
    ticks(&mut snap, &mut l, &s, 0.0, 0.1);
    assert_eq!(snap.snapper.pointer(), None);
    let (mut l, s) = captured();
    let mut snap = SnapInput::default();
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    snap.focus_lost(&mut l);
    assert_eq!(snap.snapper.pointer(), None);
    // (Alt's release after that is the driver's.)
    assert_eq!(snap.route(key(ALT, false), &mut l, &s, 0.1, Rates::default()).len(), 1);
}

#[test]
fn pointers_are_drawn_in_the_interface_view_ghost_below() {
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
        group = "Viewer@pointer"
        [sheets.g]
        image = "g"
        cell = [14, 22]
        group = "Viewer@pointer_ghost"
        "#,
    )
    .unwrap()
    .compile(&NoFiles, 256)
    .unwrap();
    let (mut l, s) = captured();
    let f = frame(&mut l, &s);
    let mut snap = SnapInput::default();
    assert!(snap.sprites(&compiled, &s, &f, &l).is_empty());
    snap.route(key(ALT, true), &mut l, &s, 0.0, Rates::default());
    let sprites = snap.sprites(&compiled, &s, &f, &l);
    assert_eq!(sprites.len(), 2);
    let interface = f.view_cameras[1];
    assert!(sprites.iter().all(|p| p.screen && p.view == interface && p.view.is_some()));
    assert_eq!(sprites[0].set, compiled.set(GHOST_SPRITE).unwrap().0);
    assert_eq!((sprites[0].x, sprites[0].y), (330, 535));
    assert_eq!(sprites[1].set, compiled.set(POINTER_SPRITE).unwrap().0);
    let (px, py) = l.pointer_in_window();
    assert_eq!((sprites[1].x, sprites[1].y), (px, py));
    // A part of the interface drawn on its own over it (the corner's
    // buttons, a popup): still in the whole interface's view, not the
    // part's, which shows only its rectangle.
    let mut s = s.clone();
    let part = scene::ViewState { id: 256, view: scene::View { interface_only: true, origin: (0, 0), viewport: (150, 50), ..Default::default() }, overlay: vec![] };
    s.views.push(part);
    s.interface.roaming = Some(256);
    let f = frame(&mut l, &s);
    let sprites = snap.sprites(&compiled, &s, &f, &l);
    assert!(sprites.iter().all(|p| p.view == f.view_cameras[1] && p.view.is_some()), "{sprites:?} {:?}", f.view_cameras);
}

struct NoFiles;
impl render_desc::ImageLoader for NoFiles {
    fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
        Err(format!("no file {source}"))
    }
}
