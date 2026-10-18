//! Interaction feedback the viewer draws itself: the hover, pressed and
//! selected looks of the driver's interactables (`scene::Looks`).
//!
//! The driver's interface stays the driver's: what is on screen and what a
//! click does. Only how an element looks under the pointer is drawn here,
//! at the viewer's frame rate: the moment the driver gets the pointer over
//! an element (or a press on it, or leaves it), the element's sprites in the
//! state that asks for are drawn in place of those the driver's interface
//! draws for it, until the driver's own drawing shows the same state (its
//! sprites then look the same, and are drawn again). A click on an element
//! that a click selects (a list's entry, a tab, a check box) shows it
//! selected (or flipped) at once, for as long as the driver takes to show it
//! too ([`PREDICTION`] at most: past that, the driver's own state shows).
//!
//! What the driver gets is what decides: the pointer and buttons are
//! followed as the input sent to the driver has them (in its window
//! pixels), so what is drawn here is what the driver will draw once it has
//! taken that input (also for the button snapping's pointer, which reaches
//! the driver once it rests on an element).

use scene::{Button, FreeSprite, Input, Interactable, InteractableKind, LookState, Looks, PointerState, Scene, Select};

/// How long a click's selection is shown before the driver shows it
/// (seconds): past that, the driver's own state is shown again (a click the
/// driver did not take as a selection).
pub const PREDICTION: f64 = 1.0;

/// Sprites of a view's overlay drawn in place of others: `range` (start,
/// end) of view `view`'s overlay are replaced by `sprites`.
#[derive(Clone, Debug, PartialEq)]
pub struct Replacement<'a> {
    pub view: u32,
    pub range: (u32, u32),
    pub sprites: &'a [FreeSprite],
}

/// What of a scene the looks go by: its interactables and their looks (of
/// the scene the frame drawn was made from).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookScene {
    pub interactables: Vec<Interactable>,
    pub looks: Vec<Looks>,
}

impl LookScene {
    pub fn of(scene: &Scene) -> Self {
        LookScene { interactables: scene.interactables.clone(), looks: scene.looks.clone() }
    }

    /// Refreshes interaction metadata independently of world or sprite edits.
    pub fn refresh(&mut self, scene: &Scene) -> bool {
        if self.interactables == scene.interactables && self.looks == scene.looks {
            return false;
        }
        *self = Self::of(scene);
        true
    }
}

/// A click's selection, until the driver shows it.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Prediction {
    id: u64,
    selected: bool,
    /// When it was made (seconds).
    at: f64,
}

/// A state drawn for an element (for the latency log): since when it was
/// wanted, and whether it was drawn yet.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Wanted {
    id: u64,
    state: LookState,
    since: f64,
    drawn: bool,
    /// Whether the driver showed it yet.
    caught_up: bool,
    /// Drawn, or shown by the driver, in the frame being made (logged once
    /// it is presented).
    log_drawn: bool,
    log_caught_up: bool,
}

#[derive(Default)]
pub struct Feedback {
    /// Off: the driver's drawing only (`TILE_VIEWER_LOOKS=off`, to compare).
    off: bool,
    /// Where the driver last got the pointer (its window pixels); `None`:
    /// not over its window.
    pointer: Option<(i32, i32)>,
    /// The left button is down (as the driver got it), pressed on this
    /// interactable (`None`: on none of them).
    left: Option<Option<u64>>,
    predictions: Vec<Prediction>,
    /// What each element is wanted to look like, if not as the driver shows
    /// it (or until the driver catches up, for the log).
    wanted: Vec<Wanted>,
    /// The elements and states drawn in the last frame.
    drawn: Vec<(u64, LookState)>,
    /// When what the pointer is over or presses last changed (seconds).
    changed_at: f64,
}

/// The interactable at `p` that has looks (topmost first, lists being
/// what their entries lie in).
fn hit(scene: &LookScene, p: (i32, i32)) -> Option<u64> {
    scene
        .interactables
        .iter()
        .filter(|i| !matches!(i.kind, InteractableKind::List { .. }))
        .find(|i| {
            let [x, y, w, h] = i.rect;
            p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
        })
        .map(|i| i.id)
}

impl Feedback {
    /// As `TILE_VIEWER_LOOKS` says (`off`: none drawn).
    pub fn from_env() -> Self {
        let off = std::env::var("TILE_VIEWER_LOOKS").is_ok_and(|v| v == "off");
        if off {
            log::info!("viewer: interaction looks off (TILE_VIEWER_LOOKS=off)");
        }
        Feedback { off, ..Default::default() }
    }

    /// Input on its way to the driver; whether what the pointer is over or
    /// presses changed (the looks may then change).
    pub fn sent(&mut self, input: &Input, scene: &LookScene, now: f64) -> bool {
        let before = (self.pointer.and_then(|p| hit(scene, p)), self.left, self.predictions.len());
        self.take(input, scene, now);
        let after = (self.pointer.and_then(|p| hit(scene, p)), self.left, self.predictions.len());
        let changed = before != after || matches!(input, Input::MouseButton { button: Button::Left, .. });
        if changed {
            self.changed_at = now;
            if before.0 != after.0 {
                let looks = after.0.and_then(|id| scene.looks.iter().find(|l| l.id == id));
                log::trace!("looks: the pointer is over {:?} (shown {:?})", after.0.map(|id| format!("{id:#x}")), looks.map(|l| l.shown));
            }
        }
        changed
    }

    fn take(&mut self, input: &Input, scene: &LookScene, now: f64) {
        match *input {
            Input::MouseMove { x, y } | Input::Wheel { x, y, .. } => self.pointer = Some((x, y)),
            Input::MouseButton { button: Button::Left, down: true, x, y } => {
                self.pointer = Some((x, y));
                self.left = Some(hit(scene, (x, y)));
            }
            Input::MouseButton { button: Button::Left, down: false, x, y } => {
                self.pointer = Some((x, y));
                let pressed = self.left.take().flatten();
                if let Some(id) = pressed.filter(|&id| hit(scene, (x, y)) == Some(id)) {
                    self.clicked(scene, id, now);
                }
            }
            Input::MouseButton { x, y, .. } => self.pointer = Some((x, y)),
            _ => {}
        }
    }

    /// The pointer no longer reaches the driver's window (it left the
    /// window, or went where the driver's window does not take it).
    pub fn pointer_gone(&mut self, now: f64) {
        if self.pointer.take().is_some() {
            self.changed_at = now;
        }
    }

    /// The left button went up without the driver getting it.
    pub fn released(&mut self, now: f64) {
        if self.left.take().is_some() {
            self.changed_at = now;
        }
    }

    /// A click on `id`: its selection as the click makes it.
    fn clicked(&mut self, scene: &LookScene, id: u64, now: f64) {
        let Some(looks) = scene.looks.iter().find(|l| l.id == id) else { return };
        let now_selected = self.predictions.iter().find(|p| p.id == id).map_or(looks.shown.selected, |p| p.selected);
        let mut predict = |id: u64, selected: bool| {
            self.predictions.retain(|p| p.id != id);
            self.predictions.push(Prediction { id, selected, at: now });
        };
        match looks.select {
            Select::None => {}
            Select::Toggle => predict(id, !now_selected),
            Select::Pick { .. } => {
                for other in scene.looks.iter().filter(|l| l.id != id && l.select == looks.select) {
                    predict(other.id, false);
                }
                predict(id, true);
            }
        }
    }

    /// The state `looks` is wanted in now.
    fn state(&self, looks: &Looks, under: Option<u64>) -> LookState {
        let pointer = match (under == Some(looks.id), self.left) {
            // Pressed on it, the pointer off it: it still holds the pointer
            // (until the button comes up), hovered.
            (false, Some(on)) if on == Some(looks.id) => PointerState::Hover,
            (false, _) => PointerState::Normal,
            (true, None) => PointerState::Hover,
            (true, Some(on)) if on == Some(looks.id) => PointerState::Pressed,
            // Pressed elsewhere: the driver's pointer is someone else's.
            (true, Some(_)) => PointerState::Normal,
        };
        let selected = self.predictions.iter().find(|p| p.id == looks.id).map_or(looks.shown.selected, |p| p.selected);
        LookState { pointer, selected }
    }

    /// What to draw in place of what the driver's interface draws now, for
    /// `scene` at time `now` (seconds): each element whose wanted state is
    /// not the one shown, and that has art for it (else the closest state
    /// with art: pressed shown as hover, hover as normal, of the same
    /// selection).
    pub fn replacements<'a>(&mut self, scene: &'a LookScene, now: f64) -> Vec<Replacement<'a>> {
        // Selections the driver shows now, or that it did not take.
        self.predictions.retain(|p| {
            let shown = scene.looks.iter().find(|l| l.id == p.id).map(|l| l.shown.selected);
            shown.is_some_and(|s| s != p.selected) && now - p.at < PREDICTION
        });
        let under = self.pointer.and_then(|p| hit(scene, p));
        let mut out = Vec::new();
        let mut drawn = Vec::new();
        let mut wanted = Vec::new();
        for looks in &scene.looks {
            let want = self.state(looks, under);
            let before = self.wanted.iter().find(|w| w.id == looks.id && w.state == want).copied();
            let since = self.changed_at.min(now);
            let mut w = before.unwrap_or(Wanted { id: looks.id, state: want, since, drawn: false, caught_up: false, log_drawn: false, log_caught_up: false });
            if want == looks.shown {
                if before.is_some() && !w.caught_up {
                    w.caught_up = true;
                    w.log_caught_up = true;
                }
                if before.is_some() {
                    wanted.push(w);
                }
                continue;
            }
            let selected = want.selected;
            let mut candidates = vec![want];
            if want.pointer == PointerState::Pressed {
                candidates.push(LookState { pointer: PointerState::Hover, selected });
            }
            candidates.push(LookState { pointer: PointerState::Normal, selected });
            let Some(state) = candidates.into_iter().find(|&s| s == looks.shown || looks.views.iter().any(|v| v.state(s).is_some())) else {
                continue;
            };
            wanted.push(w);
            if state == looks.shown {
                continue;
            }
            for v in &looks.views {
                if let Some(sprites) = v.state(state) {
                    out.push(Replacement { view: v.view, range: v.sprites, sprites });
                }
            }
            drawn.push((looks.id, state));
        }
        // (The log: the first frame drawing a wanted state.)
        for w in &mut wanted {
            if !self.off && !w.drawn && drawn.iter().any(|d| d.0 == w.id) {
                w.drawn = true;
                w.log_drawn = true;
            }
        }
        self.wanted = wanted;
        if self.off {
            self.drawn.clear();
            return Vec::new();
        }
        self.drawn = drawn;
        out
    }

    /// The frame made with the last [`Feedback::replacements`] is on its
    /// way to the screen at `now` (seconds): the log says how long after
    /// the pointer asked for them its looks were drawn, here or by the
    /// driver.
    pub fn presented(&mut self, now: f64) {
        for w in &mut self.wanted {
            let ms = (now - w.since) * 1000.0;
            if std::mem::take(&mut w.log_drawn) {
                log::debug!("looks: {:?} of {:#x} drawn here {ms:.1} ms after the pointer asked for it", w.state, w.id);
            }
            if std::mem::take(&mut w.log_caught_up) {
                log::debug!("looks: {:?} of {:#x} drawn by the driver {ms:.1} ms after the pointer asked for it", w.state, w.id);
            }
        }
    }

    /// The elements and states the last [`Feedback::replacements`] drew
    /// (to tell whether anything changed).
    pub fn drawn(&self) -> &[(u64, LookState)] {
        &self.drawn
    }
}

/// Where a view's overlay lies among the renderer's sprites: from `start`,
/// `len` of them, each drawn by its entry in `cameras`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub view: u32,
    pub start: usize,
    pub len: usize,
    pub cameras: Vec<u32>,
}

/// `base` (the renderer's sprites, the views' overlays at `spans`) with
/// `replacements` made, their sprites placed by `place` (with the camera of
/// each original sprite). Ranges that do not fit their overlay, or overlap one made
/// before, are left alone.
pub fn splice<T: Clone>(base: &[T], spans: &[Span], replacements: &[Replacement], place: impl Fn(&FreeSprite, u32) -> T) -> Vec<T> {
    let mut cuts: Vec<(usize, usize, &Replacement, &[u32])> = replacements
        .iter()
        .filter_map(|r| {
            let span = spans.iter().find(|s| s.view == r.view)?;
            let (a, b) = (r.range.0 as usize, r.range.1 as usize);
            if !(a < b && b <= span.len && span.start + span.len <= base.len()) {
                return None;
            }
            let cameras = span.cameras.get(a..b)?;
            // A variable-length state can replace a control in one panel.
            // Across panels each sprite must keep its original camera.
            (r.sprites.len() == cameras.len() || cameras.iter().all(|c| *c == cameras[0]))
                .then_some((span.start + a, span.start + b, r, cameras))
        })
        .collect();
    cuts.sort_by_key(|c| (c.0, c.1));
    let mut out = Vec::with_capacity(base.len());
    let mut at = 0;
    for (a, b, r, cameras) in cuts {
        if a < at {
            continue;
        }
        out.extend_from_slice(&base[at..a]);
        out.extend(r.sprites.iter().enumerate().map(|(i, s)| place(s, cameras.get(i).copied().unwrap_or(cameras[0]))));
        at = b;
    }
    out.extend_from_slice(&base[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::ViewLooks;

    const NORMAL: LookState = LookState { pointer: PointerState::Normal, selected: false };
    const HOVER: LookState = LookState { pointer: PointerState::Hover, selected: false };
    const PRESSED: LookState = LookState { pointer: PointerState::Pressed, selected: false };

    fn sprite(set: u32) -> FreeSprite {
        FreeSprite { set, screen: true, ..Default::default() }
    }

    /// A button at (10, 10, 20, 20) drawn by sprites 1..3 of view 5's
    /// overlay, with art for normal (set 10), hover (11) and pressed (12).
    fn scene() -> LookScene {
        let mut s = LookScene::default();
        s.interactables = vec![Interactable { anchor: None, rect: [10, 10, 20, 20], kind: InteractableKind::Button, id: 7 }];
        let states = vec![(NORMAL, vec![sprite(10)]), (HOVER, vec![sprite(11)]), (PRESSED, vec![sprite(12)])];
        s.looks = vec![Looks { id: 7, select: Select::None, shown: NORMAL, views: vec![ViewLooks { view: 5, sprites: (1, 3), states }] }];
        s
    }

    fn sets(r: &[Replacement]) -> Vec<(u32, (u32, u32), Vec<u32>)> {
        r.iter().map(|r| (r.view, r.range, r.sprites.iter().map(|s| s.set).collect())).collect()
    }

    /// A nonempty world and overlay, so metadata-only updates exercise
    /// the path that has no cell, world-sprite, or overlay changes.
    fn connected_scene() -> (scene::Driver, scene::Display, LookScene) {
        let metadata = scene();
        let (driver, mut display) = scene::connect();
        assert!(driver.send(vec![
            scene::Update::World {
                shape: scene::WorldShape { cols: 1, rows: 1, ..Default::default() },
                layers: vec![scene::LayerCells { values: vec![3], ..Default::default() }],
            },
            scene::Update::Sprites(vec![sprite(100)]),
            scene::Update::View { id: 5, view: Some(scene::View::default()) },
            scene::Update::Overlay { view: 5, sprites: (0..5).map(sprite).collect() },
            scene::Update::Interactables(metadata.interactables),
            scene::Update::Looks(metadata.looks),
        ]));
        assert!(display.receive(std::time::Duration::ZERO));
        assert!(display.take_changes().0.world);
        let cached = LookScene::of(display.scene());
        (driver, display, cached)
    }

    #[test]
    fn looks_only_update_refreshes_driver_state_without_rebuilding_world_or_overlay() {
        let (driver, mut display, mut cached) = connected_scene();
        let before = display.scene().clone();
        let mut feedback = Feedback::default();
        feedback.sent(&Input::MouseMove { x: 15, y: 15 }, &cached, 0.0);
        assert_eq!(sets(&feedback.replacements(&cached, 0.01)), [(5, (1, 3), vec![11])]);

        let mut updated = before.looks.clone();
        updated[0].shown = HOVER;
        assert!(driver.send(vec![scene::Update::Looks(updated)]));
        assert!(display.receive(std::time::Duration::ZERO));
        assert!(display.changes().is_empty());
        assert_eq!(display.scene().layers, before.layers);
        assert_eq!(display.scene().sprites, before.sprites);
        assert_eq!(display.scene().views, before.views);
        assert!(cached.refresh(display.scene()));
        assert!(feedback.replacements(&cached, 0.02).is_empty());
    }

    #[test]
    fn replacement_art_refreshes_and_splices_without_a_look_state_change() {
        let (driver, mut display, mut cached) = connected_scene();
        let before = display.scene().clone();
        let base: Vec<_> = before.sprites.iter().map(|s| (*s, 9))
            .chain(before.views[0].overlay.iter().map(|s| (*s, 3))).collect();
        let spans = [Span { view: 5, start: 1, len: 5, cameras: vec![3; 5] }];
        let mut feedback = Feedback::default();
        feedback.sent(&Input::MouseMove { x: 15, y: 15 }, &cached, 0.0);
        let previous = splice(&base, &spans, &feedback.replacements(&cached, 0.01), |s, camera| (*s, camera));
        let drawn = feedback.drawn().to_vec();
        assert_eq!(drawn, [(7, HOVER)]);
        assert_eq!(previous.iter().map(|(s, _)| s.set).collect::<Vec<_>>(), [100, 0, 11, 3, 4]);

        let replacement = FreeSprite { x: 17, y: 19, ..sprite(21) };
        let mut updated = before.looks.clone();
        updated[0].views[0].states.iter_mut().find(|(state, _)| *state == HOVER).unwrap().1 = vec![replacement];
        assert!(driver.send(vec![scene::Update::Looks(updated)]));
        assert!(display.receive(std::time::Duration::ZERO));
        assert!(display.changes().is_empty());
        assert_eq!(display.scene().layers, before.layers);
        assert_eq!(display.scene().sprites, before.sprites);
        assert_eq!(display.scene().views, before.views);
        assert!(cached.refresh(display.scene()));
        let refreshed = splice(&base, &spans, &feedback.replacements(&cached, 0.02), |s, camera| (*s, camera));
        // Comparing only Feedback::drawn would miss this changed art.
        assert_eq!(feedback.drawn(), drawn);
        assert_ne!(refreshed, previous);
        assert_eq!(refreshed[2], (replacement, 3));
        assert_eq!(refreshed.iter().map(|(s, _)| s.set).collect::<Vec<_>>(), [100, 0, 21, 3, 4]);
    }

    #[test]
    fn identical_metadata_in_a_new_scene_does_not_invalidate_the_cached_looks() {
        let (driver, mut display, mut cached) = connected_scene();
        let before = cached.clone();
        let old_seq = display.scene().seq;
        let mut feedback = Feedback::default();
        feedback.sent(&Input::MouseMove { x: 15, y: 15 }, &cached, 0.0);
        let previous = sets(&feedback.replacements(&cached, 0.01));
        let drawn = feedback.drawn().to_vec();

        assert!(driver.send(vec![scene::Update::Looks(before.looks.clone())]));
        assert!(display.receive(std::time::Duration::ZERO));
        assert!(display.scene().seq > old_seq);
        assert!(display.changes().is_empty());
        assert!(!cached.refresh(display.scene()));
        assert_eq!(cached, before);
        assert_eq!(sets(&feedback.replacements(&cached, 0.02)), previous);
        assert_eq!(feedback.drawn(), drawn);
    }

    #[test]
    fn interactable_only_update_rechecks_a_stationary_pointer() {
        let (driver, mut display, mut cached) = connected_scene();
        let before = display.scene().clone();
        let mut feedback = Feedback::default();
        feedback.sent(&Input::MouseMove { x: 15, y: 15 }, &cached, 0.0);
        assert_eq!(sets(&feedback.replacements(&cached, 0.01)), [(5, (1, 3), vec![11])]);

        let mut moved = before.interactables.clone();
        moved[0].rect = [50, 50, 20, 20];
        assert!(driver.send(vec![scene::Update::Interactables(moved)]));
        assert!(display.receive(std::time::Duration::ZERO));
        assert!(display.changes().is_empty());
        assert_eq!(display.scene().looks, before.looks);
        assert_eq!(display.scene().views, before.views);
        assert!(cached.refresh(display.scene()));
        assert!(feedback.replacements(&cached, 0.02).is_empty());
    }

    #[test]
    fn hover_pressed_and_back_on_leave_until_the_driver_shows_them() {
        let mut s = scene();
        let mut f = Feedback::default();
        assert!(f.replacements(&s, 0.0).is_empty());
        f.sent(&Input::MouseMove { x: 15, y: 15 }, &s, 0.0);
        assert_eq!(sets(&f.replacements(&s, 0.01)), [(5, (1, 3), vec![11])]);
        // The driver shows the hover now: its own sprites are drawn.
        s.looks[0].shown = HOVER;
        assert!(f.replacements(&s, 0.05).is_empty());
        f.sent(&Input::MouseButton { button: Button::Left, down: true, x: 15, y: 15 }, &s, 0.1);
        assert_eq!(sets(&f.replacements(&s, 0.1)), [(5, (1, 3), vec![12])]);
        // Off it while pressed: hovered (it holds the pointer); back on it:
        // pressed again.
        f.sent(&Input::MouseMove { x: 50, y: 15 }, &s, 0.2);
        assert!(f.replacements(&s, 0.2).is_empty());
        f.sent(&Input::MouseMove { x: 12, y: 15 }, &s, 0.3);
        assert_eq!(sets(&f.replacements(&s, 0.3)), [(5, (1, 3), vec![12])]);
        f.sent(&Input::MouseButton { button: Button::Left, down: false, x: 12, y: 15 }, &s, 0.4);
        assert!(f.replacements(&s, 0.4).is_empty());
        // Gone (the driver still shows the hover): normal.
        f.pointer_gone(0.0);
        assert_eq!(sets(&f.replacements(&s, 0.5)), [(5, (1, 3), vec![10])]);
    }

    #[test]
    fn a_state_the_driver_shows_without_looks_stays_the_drivers() {
        let mut s = scene();
        let mut f = Feedback::default();
        f.sent(&Input::MouseMove { x: 15, y: 15 }, &s, 0.0);
        // e.g. disabled: the driver sends no looks for it.
        s.looks.clear();
        assert!(f.replacements(&s, 0.0).is_empty());
        // Art for normal only: hover shows as normal, which it is.
        let mut s = scene();
        s.looks[0].views[0].states.retain(|(st, _)| *st == NORMAL);
        assert!(f.replacements(&s, 0.0).is_empty());
    }

    #[test]
    fn a_click_selects_at_once_until_the_driver_shows_it_or_not() {
        let mut s = scene();
        let sel = LookState { pointer: PointerState::Normal, selected: true };
        let entry = |id: u64, at: i32, shown: LookState| Looks {
            id,
            select: Select::Pick { group: 1 },
            shown,
            views: vec![ViewLooks {
                view: 5,
                sprites: (at as u32, at as u32 + 1),
                states: vec![(NORMAL, vec![sprite(20 + id as u32)]), (sel, vec![sprite(30 + id as u32)])],
            }],
        };
        s.interactables = vec![
            Interactable { anchor: None, rect: [0, 0, 100, 10], kind: InteractableKind::ListItem, id: 1 },
            Interactable { anchor: None, rect: [0, 10, 100, 10], kind: InteractableKind::ListItem, id: 2 },
            Interactable { anchor: None, rect: [0, 0, 100, 20], kind: InteractableKind::List { scroll: [0, 0, 100, 20], step: 10 }, id: 3 },
        ];
        s.looks = vec![entry(1, 0, sel), entry(2, 1, NORMAL)];
        let mut f = Feedback::default();
        f.sent(&Input::MouseButton { button: Button::Left, down: true, x: 5, y: 15 }, &s, 0.0);
        f.sent(&Input::MouseButton { button: Button::Left, down: false, x: 5, y: 15 }, &s, 0.05);
        // Entry 2 selected (hovered: no art for that, shown selected),
        // entry 1 no longer.
        assert_eq!(sets(&f.replacements(&s, 0.06)), [(5, (0, 1), vec![21]), (5, (1, 2), vec![32])]);
        // The driver took it.
        s.looks = vec![entry(1, 0, NORMAL), entry(2, 1, sel)];
        assert!(f.replacements(&s, 0.1).is_empty());
        // A click the driver does not take: shown for a while, then the
        // driver's state again.
        f.sent(&Input::MouseButton { button: Button::Left, down: true, x: 5, y: 5 }, &s, 1.0);
        f.sent(&Input::MouseButton { button: Button::Left, down: false, x: 5, y: 5 }, &s, 1.0);
        f.pointer_gone(0.0);
        assert_eq!(sets(&f.replacements(&s, 1.1)), [(5, (0, 1), vec![31]), (5, (1, 2), vec![22])]);
        assert!(f.replacements(&s, 1.0 + PREDICTION + 0.01).is_empty());
    }

    #[test]
    fn check_boxes_flip() {
        let mut s = scene();
        let on = LookState { pointer: PointerState::Normal, selected: true };
        s.looks[0].select = Select::Toggle;
        s.looks[0].views[0].states = vec![(NORMAL, vec![sprite(10)]), (on, vec![sprite(13)])];
        let mut f = Feedback::default();
        f.sent(&Input::MouseButton { button: Button::Left, down: true, x: 15, y: 15 }, &s, 0.0);
        f.sent(&Input::MouseButton { button: Button::Left, down: false, x: 15, y: 15 }, &s, 0.0);
        f.pointer_gone(0.0);
        assert_eq!(sets(&f.replacements(&s, 0.0)), [(5, (1, 3), vec![13])]);
        s.looks[0].shown = on;
        assert!(f.replacements(&s, 0.1).is_empty());
    }

    #[test]
    fn replacements_splice_the_overlay() {
        let overlay: Vec<FreeSprite> = (0..5).map(sprite).collect();
        let a = [sprite(10), sprite(11)];
        let b = [sprite(12)];
        let r = [
            Replacement { view: 1, range: (3, 4), sprites: &b },
            Replacement { view: 1, range: (1, 2), sprites: &a },
            Replacement { view: 2, range: (0, 5), sprites: &b },
            Replacement { view: 1, range: (4, 9), sprites: &b },
        ];
        // The world's 2 sprites, view 1's overlay (5), view 2's (5).
        let base: Vec<FreeSprite> = (100..102).map(sprite).chain(overlay.iter().copied()).chain((200..205).map(sprite)).collect();
        let spans = [Span { view: 1, start: 2, len: 5, cameras: vec![0; 5] }, Span { view: 2, start: 7, len: 5, cameras: vec![1; 5] }];
        let want: Vec<(u32, u32)> = [100, 101, 0].map(|s| (s, 9)).into_iter().chain([(10, 0), (11, 0)]).chain([(2, 9)]).chain([(12, 0)]).chain([(4, 9)]).chain([(12, 1)]).collect();
        let base_marked: Vec<(u32, u32)> = base.iter().map(|s| (s.set, 9)).collect();
        assert_eq!(splice(&base_marked, &spans, &r, |s, camera| (s.set, camera)), want);
        assert_eq!(splice(&base_marked, &spans, &[], |s, c| (s.set, c)), base_marked);
    }

    #[test]
    fn replacements_keep_each_scaled_panels_camera() {
        let spans = [Span { view: 1, start: 1, len: 4, cameras: vec![3, 3, 7, 7] }];
        let base = vec![(100, 0), (0, 3), (1, 3), (2, 7), (3, 7)];
        let sprites = [sprite(10), sprite(11)];
        // A control spanning panel cameras keeps their separate transforms.
        let r = [Replacement { view: 1, range: (1, 3), sprites: &sprites }];
        assert_eq!(splice(&base, &spans, &r, |s, c| (s.set, c)), [(100, 0), (0, 3), (10, 3), (11, 7), (3, 7)]);
        // A state with more sprites can stay inside one panel.
        let r = [Replacement { view: 1, range: (2, 3), sprites: &sprites }];
        assert_eq!(splice(&base, &spans, &r, |s, c| (s.set, c)), [(100, 0), (0, 3), (1, 3), (10, 7), (11, 7), (3, 7)]);
        // Different-length states across panels cannot be assigned safely.
        let r = [Replacement { view: 1, range: (1, 4), sprites: &sprites }];
        assert_eq!(splice(&base, &spans, &r, |s, c| (s.set, c)), base);
    }

}
