//! The lists the driver shows as views of their own (`scene::ListView`),
//! scrolled by the viewer.
//!
//! A list has one offset (content pixels, continuous). The trackpad moves
//! it at once by the pixels the fingers move (through the list's zoom),
//! from the first input on; a wheel notch moves its target a step, which
//! the offset glides to. The entries are drawn at that offset every frame
//! (`scaling::scrolled`: by screen pixels, the entries it puts partly in
//! the list cut at its edges); nothing waits for the driver, which follows
//! (`Input::ListScrolled`): its own list shows the entry there first and
//! it publishes the entries round it. A move of the driver's own (keys, a
//! selection it brings into view) is followed.
//!
//! The pointer on a list goes to the driver where its list, at its own
//! offset, has what the viewer shows under the pointer.

use scene::{Input, Scene};

/// The time constant (seconds) of a list's glide to where wheel notches
/// put it.
pub const GLIDE: f64 = crate::snap::SCROLL_SMOOTHING;
/// A wheel notch, as `Input::Wheel` counts them.
const NOTCH: f64 = 120.0;

#[derive(Clone, Debug, PartialEq)]
struct List {
    id: u64,
    /// Its rectangle in the driver's pixels.
    rect: [i32; 4],
    step: f64,
    /// The largest offset (the content less the viewport).
    max: f64,
    /// The offset shown and where wheel notches glide it.
    pos: f64,
    target: f64,
    /// The driver's offset when last seen.
    driver: i32,
    /// The offset last told the driver.
    sent: Option<i32>,
    /// Canvas pixels per driver pixel where it shows.
    zoom: f64,
}

impl List {
    fn clamp(&mut self) {
        self.pos = self.pos.clamp(0.0, self.max);
        self.target = self.target.clamp(0.0, self.max);
    }
}

/// The viewer's lists (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct ListScrolls {
    lists: Vec<List>,
    /// The last window scroll moved a list: the `Wheel` notches the window
    /// made of the same motion are not the driver's.
    scrolled: bool,
}

fn inside([x, y, w, h]: [i32; 4], (px, py): (i32, i32)) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

impl ListScrolls {
    /// Follows the scene's lists: a new one starts where the driver shows
    /// it; a move the driver made of its own is taken over; one gone is
    /// forgotten.
    pub fn follow(&mut self, scene: &Scene) {
        let mut kept = Vec::new();
        for v in &scene.views {
            let Some(l) = v.view.list else { continue };
            let rect = [v.view.origin.0, v.view.origin.1, v.view.viewport.0 as i32, v.view.viewport.1 as i32];
            let max = (l.count as f64 * l.step as f64 - rect[3] as f64).max(0.0);
            let driver = v.view.camera_y;
            let mut list = match self.lists.iter().find(|x| x.id == l.id) {
                Some(old) => {
                    let mut old = old.clone();
                    // (Moved by the driver, not to where it was told.)
                    if driver != old.driver && Some(driver) != old.sent {
                        (old.pos, old.target, old.sent) = (driver as f64, driver as f64, Some(driver));
                    }
                    old
                }
                None => List { id: l.id, rect, step: 0.0, max, pos: driver as f64, target: driver as f64, driver, sent: Some(driver), zoom: 1.0 },
            };
            (list.rect, list.step, list.max, list.driver) = (rect, l.step.max(1) as f64, max, driver);
            list.clamp();
            kept.push(list);
        }
        self.lists = kept;
    }

    /// Where each list shows (`scaling::ListShown`): its zoom there.
    pub fn shown(&mut self, shown: &[crate::scaling::ListShown]) {
        for l in &mut self.lists {
            if let Some(s) = shown.iter().find(|s| s.id == l.id) {
                l.zoom = s.zoom.0.max(1) as f64 / s.zoom.1.max(1) as f64;
            }
        }
    }

    /// `dt` seconds on: the glides to the wheel's targets. Whether a list
    /// still moves.
    pub fn advance(&mut self, dt: f64) -> bool {
        let k = 1.0 - (-dt.max(0.0) / GLIDE).exp();
        let mut moving = false;
        for l in &mut self.lists {
            let d = l.target - l.pos;
            if d.abs() < 0.25 {
                l.pos = l.target;
            } else {
                l.pos += d * k;
                moving = true;
            }
        }
        moving
    }

    /// Where the viewer shows each list (`layout::Frame::list_offsets`).
    pub fn offsets(&self) -> Vec<(u64, f64)> {
        self.lists.iter().map(|l| (l.id, l.pos)).collect()
    }

    /// The driver's input at its pixels: a wheel or a window scroll over a
    /// list moves the list and is the viewer's (`None`); a pointer input on
    /// a list goes where the driver's list has what shows under it; any
    /// other input as it is.
    pub fn route(&mut self, input: Input) -> Option<Input> {
        let over = |lists: &[List], x: i32, y: i32| lists.iter().position(|l| inside(l.rect, (x, y)));
        match input {
            Input::Scroll { dx: _, dy, pixels, x, y } => {
                let Some(i) = over(&self.lists, x, y) else { self.scrolled = false; return Some(input) };
                let l = &mut self.lists[i];
                // (Positive moves the content down: towards the start.)
                let by = if pixels { dy as f64 / 1000.0 / l.zoom.max(1e-6) } else { dy as f64 / 1000.0 * l.step };
                l.pos -= by;
                l.target = l.pos;
                l.clamp();
                self.scrolled = true;
                None
            }
            Input::Wheel { delta, x, y } => {
                let Some(i) = over(&self.lists, x, y) else { return Some(input) };
                if self.scrolled {
                    return None;
                }
                let l = &mut self.lists[i];
                l.target -= delta as f64 / NOTCH * l.step;
                l.clamp();
                None
            }
            Input::MouseMove { x, y } => {
                self.scrolled = false;
                let (x, y) = self.to_driver(x, y);
                Some(Input::MouseMove { x, y })
            }
            Input::MouseButton { button, down, x, y } => {
                self.scrolled = false;
                let (x, y) = self.to_driver(x, y);
                Some(Input::MouseButton { button, down, x, y })
            }
            other => {
                if matches!(other, Input::Key { .. }) {
                    self.scrolled = false;
                }
                Some(other)
            }
        }
    }

    /// Where the driver's list (at its own offset) has the content the
    /// viewer shows at its pixel (x, y).
    fn to_driver(&self, x: i32, y: i32) -> (i32, i32) {
        match self.lists.iter().find(|l| inside(l.rect, (x, y))) {
            Some(l) => (x, y + (l.pos.round() as i32 - l.driver)),
            None => (x, y),
        }
    }

    /// What the driver gets: where each list shows, when that changed.
    pub fn take_inputs(&mut self) -> Vec<Input> {
        let mut out = Vec::new();
        for l in &mut self.lists {
            let at = l.pos.round() as i32;
            if l.sent != Some(at) {
                l.sent = Some(at);
                out.push(Input::ListScrolled { id: l.id, offset: at });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::{ListView, View, ViewState};

    fn scene(camera_y: i32) -> Scene {
        let mut s = Scene::default();
        s.views.push(ViewState { id: 7, view: View { interface_only: true, origin: (100, 200), viewport: (300, 120), camera_y,
            list: Some(ListView { id: 42, over: 1, count: 30, step: 24, published: (0, 30) }), ..Default::default() }, overlay: vec![] });
        s
    }

    fn pos(l: &ListScrolls) -> f64 {
        l.offsets()[0].1
    }

    #[test]
    fn the_trackpad_moves_a_list_from_its_first_pixel() {
        let mut l = ListScrolls::default();
        l.follow(&scene(0));
        l.shown(&[crate::scaling::ListShown { id: 42, rect: [0, 0, 1, 1], zoom: (2, 1) }]);
        // One window pixel up at 2:1: half a content pixel, at once.
        assert!(l.route(Input::Scroll { dx: 0, dy: -1000, pixels: true, x: 150, y: 250 }).is_none());
        assert_eq!(pos(&l), 0.5);
        // The window's notches of the same motion are not the driver's.
        assert!(l.route(Input::Wheel { delta: -120, x: 150, y: 250 }).is_none());
        assert_eq!(pos(&l), 0.5);
        assert_eq!(l.take_inputs(), [Input::ListScrolled { id: 42, offset: 1 }]);
        // Not past its ends: 30 entries of 24 in 120 pixels.
        l.route(Input::Scroll { dx: 0, dy: -10_000_000, pixels: true, x: 150, y: 250 });
        assert_eq!(pos(&l), 30.0 * 24.0 - 120.0);
        l.route(Input::Scroll { dx: 0, dy: 10_000_000, pixels: true, x: 150, y: 250 });
        assert_eq!(pos(&l), 0.0);
        // Beside it: not its.
        assert!(l.route(Input::Scroll { dx: 0, dy: -1000, pixels: true, x: 50, y: 250 }).is_some());
    }

    #[test]
    fn a_wheel_notch_glides_a_step_and_the_driver_follows() {
        let mut l = ListScrolls::default();
        l.follow(&scene(0));
        assert!(l.route(Input::Key { vk: 0x41, down: true, text: None }).is_some());
        assert!(l.route(Input::Wheel { delta: -120, x: 150, y: 250 }).is_none());
        assert!(l.advance(0.016));
        let first = pos(&l);
        assert!(first > 0.0 && first < 24.0, "{first}");
        for _ in 0..200 { l.advance(0.016); }
        assert_eq!(pos(&l), 24.0);
        assert_eq!(l.take_inputs(), [Input::ListScrolled { id: 42, offset: 24 }]);
        assert!(l.take_inputs().is_empty());
        // The driver shows it there: nothing changes.
        l.follow(&scene(24));
        assert_eq!(pos(&l), 24.0);
        // The driver moved it itself (a key): followed.
        l.follow(&scene(96));
        assert_eq!(pos(&l), 96.0);
        assert!(l.take_inputs().is_empty());
    }

    #[test]
    fn the_pointer_on_a_list_goes_where_the_drivers_list_has_it() {
        let mut l = ListScrolls::default();
        l.follow(&scene(48));
        l.route(Input::Scroll { dx: 0, dy: -10_000, pixels: false, x: 150, y: 250 });
        // Ten lines (entries) on: the driver still at 48.
        assert_eq!(pos(&l), 48.0 + 240.0);
        assert_eq!(l.route(Input::MouseButton { button: scene::Button::Left, down: true, x: 150, y: 210 }),
            Some(Input::MouseButton { button: scene::Button::Left, down: true, x: 150, y: 210 + 240 }));
        // Beside the list: as it is.
        assert_eq!(l.route(Input::MouseMove { x: 50, y: 210 }), Some(Input::MouseMove { x: 50, y: 210 }));
    }
}
