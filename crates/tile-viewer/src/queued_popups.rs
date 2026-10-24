//! Reports the driver accepted at once and left to the viewer to show
//! (`scene::Interface::queued`, docs/modal-input.md "Reports the game does
//! not wait for"). They are no modal: the driver's game went on. The viewer
//! shows them as a stack, the first to read on top (`scaling::queued_places`:
//! over the world at its top-right, over the menus while a game loads where
//! a modal popup would be), and the player reads them one by one: a press
//! on the stack, or Enter or Esc with the pointer on it, closes the first
//! and does nothing else. Every other input goes on to the driver as if
//! they were not there. The first one's button is a target of the Alt
//! mode (`Layout::queued_targets`).

use super::{button_index, inside, Button, Input, Layout};
use crate::scaling::QueuedShown;

const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;

/// The queued reports as shown, and what the driver is to hear of them.
#[derive(Clone, Debug, Default)]
pub(super) struct Queued {
    /// As arranged in the last frame (`scaling::Arranged::queued`), the one
    /// to read first.
    shown: Vec<QueuedShown>,
    /// Read here, until the driver stops sending them (not drawn).
    closing: Vec<u64>,
    /// `Input::CloseQueued` not sent yet.
    outbox: Vec<Input>,
    /// Buttons whose press closed one: their release does nothing either.
    swallowed: [bool; 3],
    /// Keys whose press closed one: their release does nothing either.
    keys: Vec<u32>,
}

impl Queued {
    /// The driver's inputs this asks for (`Layout::route`).
    pub(super) fn take_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.outbox)
    }

    /// The first one shown is read: it goes, the driver hears of it.
    fn read_first(&mut self) -> bool {
        let Some(first) = self.shown.iter().find(|q| !self.closing.contains(&q.serial)).copied() else { return false };
        self.closing.push(first.serial);
        self.outbox.push(Input::CloseQueued { serial: first.serial });
        self.shown.retain(|q| q.serial != first.serial);
        log::info!("viewer: queued report {} read", first.serial);
        true
    }
}

impl Layout {
    /// The queued reports as `scaling::arrange` showed them.
    pub fn set_queued_shown(&mut self, shown: Vec<QueuedShown>) {
        self.queued.shown = shown.into_iter().filter(|q| !self.queued.closing.contains(&q.serial)).collect();
    }

    /// The queued reports as shown now (canvas pixels), the first on top.
    pub fn queued_shown(&self) -> &[QueuedShown] {
        &self.queued.shown
    }

    /// Those read here and still sent by the driver get no camera: they
    /// are gone at once. Those it no longer sends are forgotten.
    pub(super) fn queued_frame(&mut self, scene: &scene::Scene, frame: &mut super::Frame) {
        let queued = &scene.interface.queued;
        self.queued.closing.retain(|s| queued.iter().any(|q| q.serial == *s));
        self.queued.shown.retain(|s| queued.iter().any(|q| q.serial == s.serial));
        for (i, v) in scene.views.iter().enumerate() {
            let closing = queued.iter().any(|q| q.view == v.id && self.queued.closing.contains(&q.serial));
            if closing {
                if let Some(camera) = frame.view_cameras.get_mut(i) {
                    *camera = None;
                }
            }
        }
    }

    /// Whether canvas pixel `p` is on the queued reports.
    fn on_queued(&self, p: (i32, i32)) -> bool {
        self.queued.shown.iter().any(|q| inside(q.rect, p))
    }

    /// A mouse button at `p`: `Some` (what goes to the driver: the report
    /// it read) if it was on the queued reports (not with the mouse
    /// captured: it plays the world then); its release then does nothing.
    pub(super) fn queued_button(&mut self, button: Button, down: bool, p: (i32, i32)) -> Option<Vec<Input>> {
        if down && self.locked {
            return None;
        }
        self.queued_click(button, down, p)
    }

    /// A click of the Alt mode's pointer at `p` (the mouse may be
    /// captured meanwhile): as `queued_button`.
    pub fn queued_click(&mut self, button: Button, down: bool, p: (i32, i32)) -> Option<Vec<Input>> {
        let b = button_index(button);
        if !down {
            return std::mem::take(&mut self.queued.swallowed[b]).then(Vec::new);
        }
        if !self.on_queued(p) {
            return None;
        }
        self.queued.read_first();
        self.queued.swallowed[b] = true;
        Some(self.queued.take_inputs())
    }

    /// Enter or Esc with the pointer on the queued reports reads the first
    /// (true: the key, and its release, go no further).
    pub(super) fn queued_key(&mut self, vk: u32, down: bool) -> bool {
        if !down {
            let held = self.queued.keys.contains(&vk);
            self.queued.keys.retain(|&k| k != vk);
            return held;
        }
        if self.queued.keys.contains(&vk) {
            // (A key held repeats its press.)
            return true;
        }
        if !matches!(vk, VK_RETURN | VK_ESCAPE) || self.locked || !self.on_queued(self.pointer) {
            return false;
        }
        self.queued.keys.push(vk);
        self.queued.read_first();
        true
    }

    /// The Alt mode's targets on the queued reports (canvas pixels): the
    /// first one's button, else all of it.
    pub fn queued_targets(&self) -> Vec<[i32; 4]> {
        self.queued.shown.first().map(|q| q.button.unwrap_or(q.rect)).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scaling::QueuedShown;

    fn layout(shown: &[(u64, [i32; 4])]) -> Layout {
        let mut l = Layout::default();
        l.set_queued_shown(shown.iter().map(|&(serial, rect)| QueuedShown { serial, view: 0x1100 + serial as u32, rect, button: Some([rect[0] + 10, rect[1] + rect[3] - 30, 24, 24]) }).collect());
        l
    }

    #[test]
    fn a_press_on_the_queue_reads_the_first_and_nothing_else() {
        let mut l = layout(&[(1, [600, 16, 300, 200]), (2, [610, 26, 300, 200])]);
        // Beside it: the driver's, as without it.
        assert_eq!(l.queued_button(Button::Left, true, (100, 100)), None);
        assert_eq!(l.queued_button(Button::Left, false, (100, 100)), None);
        // On the one behind (its visible edge): the first is read.
        assert_eq!(l.queued_button(Button::Left, true, (905, 220)), Some(vec![Input::CloseQueued { serial: 1 }]));
        assert_eq!(l.queued_button(Button::Left, false, (905, 220)), Some(vec![]), "its release swallowed");
        assert_eq!(l.queued_shown().iter().map(|q| q.serial).collect::<Vec<_>>(), [2]);
        assert_eq!(l.queued_targets(), vec![[620, 196, 24, 24]], "the next one's button");
        // The driver still sends the first for a frame: it stays read.
        l.set_queued_shown(vec![QueuedShown { serial: 1, view: 0x1101, rect: [600, 16, 300, 200], button: None }, QueuedShown { serial: 2, view: 0x1100, rect: [600, 16, 300, 200], button: None }]);
        assert_eq!(l.queued_shown().iter().map(|q| q.serial).collect::<Vec<_>>(), [2]);
        // With the mouse captured the press plays the world.
        l.locked = true;
        assert_eq!(l.queued_button(Button::Left, true, (700, 100)), None);
    }

    #[test]
    fn enter_and_escape_read_the_first_only_with_the_pointer_on_the_queue() {
        let mut l = layout(&[(5, [600, 16, 300, 200])]);
        l.pointer = (10, 10);
        assert!(!l.queued_key(VK_RETURN, true), "elsewhere Enter is the game's");
        assert!(!l.queued_key(VK_RETURN, false));
        l.pointer = (700, 100);
        assert!(!l.queued_key(0x41, true), "other keys are the game's");
        assert!(l.queued_key(VK_ESCAPE, true));
        assert!(l.queued_key(VK_ESCAPE, true), "repeats swallowed");
        assert_eq!(l.queued.take_inputs(), vec![Input::CloseQueued { serial: 5 }]);
        assert!(l.queued_key(VK_ESCAPE, false), "its release swallowed");
        assert!(!l.queued_key(VK_RETURN, true), "nothing left to read");
        assert!(l.queued_targets().is_empty());
    }

    #[test]
    fn routed_input_reaches_the_driver_beside_the_queue_and_reads_it_on_it() {
        let mut l = layout(&[(7, [600, 16, 300, 200])]);
        l.pointer = (700, 100);
        assert_eq!(l.route(Input::Key { vk: VK_RETURN, down: true, text: None }), vec![Input::CloseQueued { serial: 7 }]);
        assert_eq!(l.route(Input::Key { vk: VK_RETURN, down: false, text: None }), vec![]);
        let mut l = layout(&[(8, [600, 16, 300, 200])]);
        let out = l.route(Input::MouseButton { button: Button::Left, down: true, x: 700, y: 100 });
        assert_eq!(out, vec![Input::CloseQueued { serial: 8 }]);
        assert_eq!(l.route(Input::MouseButton { button: Button::Left, down: false, x: 700, y: 100 }), vec![]);
        let mut l = layout(&[(9, [600, 16, 300, 200])]);
        l.pointer = (5, 5);
        assert!(!l.route(Input::Key { vk: VK_RETURN, down: true, text: None }).contains(&Input::CloseQueued { serial: 9 }));
    }
}
