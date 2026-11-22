//! Tile information boxes the driver closed at once and left to the viewer
//! (`scene::Interface::detached`, docs/world-input.md "Popups the viewer
//! keeps"). Each goes where a popup anchored at its click would
//! (`Layout::anchored_place`), in the view the click was made in, and stays
//! there while that view and the others go on playing. A press in that view
//! (or on the box) closes it and does nothing else, as a click beside the
//! game's box does; so does Esc with the pointer in that view. Nothing in it
//! takes input: the game has no box open any more.
//!
//! A box that would reach past its view's edge moves the view as little as
//! it takes to show it whole (it stays on its tile), and closing it moves
//! the view back by as much: for the view following the world cursor, only
//! while the box is there.

use super::{button_index, fit_pan, inside, scaled_up, world_steps, Button, Follow, Hit, Input, Layout, WorldPress};

/// The views' kept popups, and what the driver is to hear of them.
#[derive(Clone, Debug, Default)]
pub(super) struct Detached {
    kept: Vec<Kept>,
    /// The latest world presses sent (a popup is the newest one's at its
    /// click), oldest first.
    presses: Vec<WorldPress>,
    /// Closed here, until the driver stops sending them (not drawn).
    closing: Vec<u64>,
    /// `Input::CloseDetached` not sent yet.
    outbox: Vec<Input>,
    /// Buttons whose press closed one: their release does nothing either.
    swallowed: [bool; 3],
    /// Views moved for boxes now gone, to be moved back by as much.
    unpan: Vec<(Hit, (i32, i32))>,
}

#[derive(Copy, Clone, Debug)]
struct Kept {
    serial: u64,
    view: u32,
    anchor: scene::PopupAnchor,
    size: (i32, i32),
    press: WorldPress,
    /// How far its view was moved for it (world pixels).
    pan: (i32, i32),
    /// That view follows the world cursor: moved only while it is kept.
    pointer: bool,
}

/// How many world presses are remembered for finding a popup's click.
const PRESSES: usize = 8;

impl Detached {
    fn close(&mut self, serial: u64) {
        self.forget(serial);
        if !self.closing.contains(&serial) {
            self.closing.push(serial);
            self.outbox.push(Input::CloseDetached { serial });
        }
    }

    /// Drops kept box `serial`; its view's move is to be undone.
    fn forget(&mut self, serial: u64) {
        for k in self.kept.iter().filter(|k| k.serial == serial && k.pan != (0, 0) && !k.pointer) {
            self.unpan.push((k.press.hit, k.pan));
        }
        self.kept.retain(|k| k.serial != serial);
    }

    /// The driver's inputs this asks for (`Layout::route`).
    pub(super) fn take_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.outbox)
    }
}

impl Layout {
    /// A press on a world pixel went to the driver (`Input::WorldButton`):
    /// a popup it opens is kept in its view.
    pub(super) fn detached_press_sent(&mut self, press: WorldPress) {
        let d = &mut self.detached;
        d.presses.push(press);
        if d.presses.len() > PRESSES {
            d.presses.remove(0);
        }
    }

    /// The driver's kept popups now (`scene`): a new one goes to the view of
    /// the newest press at its click (that view's older one closes); those
    /// it no longer sends are gone; those whose view is no longer shown
    /// close.
    pub(super) fn detached_update(&mut self, scene: &scene::Scene) {
        let popups = &scene.interface.detached;
        let size = |view: u32| scene.view(view).map_or((0, 0), |v| (v.view.viewport.0 as i32, v.view.viewport.1 as i32));
        let shown: Vec<Hit> = self.shown.iter().map(|(hit, _)| *hit).collect();
        let d = &mut self.detached;
        let gone: Vec<u64> = d.kept.iter().filter(|k| !popups.iter().any(|p| p.serial == k.serial)).map(|k| k.serial).collect();
        for serial in gone {
            d.forget(serial);
        }
        let mut new = Vec::new();
        d.closing.retain(|s| popups.iter().any(|p| p.serial == *s));
        for p in popups {
            if let Some(k) = d.kept.iter_mut().find(|k| k.serial == p.serial) {
                // (Its view id moves down as older ones go.)
                (k.view, k.anchor, k.size) = (p.view, p.anchor, size(p.view));
                continue;
            }
            if d.closing.contains(&p.serial) {
                continue;
            }
            let press = d.presses.iter().rev().find(|q| q.world == p.click).or(d.presses.last()).copied();
            let Some(press) = press else {
                log::info!("viewer: kept popup {} has no press here: closed", p.serial);
                d.close(p.serial);
                continue;
            };
            let older: Vec<u64> = d.kept.iter().filter(|k| k.press.hit == press.hit).map(|k| k.serial).collect();
            for serial in older {
                d.close(serial);
            }
            log::info!("viewer: the tile box {} kept at the world click {:?} ({:?}); the views play on", p.serial, press.world, press.hit);
            d.kept.push(Kept { serial: p.serial, view: p.view, anchor: p.anchor, size: size(p.view), press, pan: (0, 0), pointer: false });
            new.push(p.serial);
        }
        if !shown.is_empty() {
            let gone: Vec<u64> = d.kept.iter().filter(|k| !shown.contains(&k.press.hit)).map(|k| k.serial).collect();
            for serial in gone {
                d.close(serial);
            }
        }
        for serial in new {
            self.detached_fit(serial);
        }
        self.detached_unpan();
    }

    /// Moves the view of new kept box `serial` as little as it takes for
    /// the box to show whole in it.
    fn detached_fit(&mut self, serial: u64) {
        let Some(k) = self.detached.kept.iter().find(|k| k.serial == serial).copied() else { return };
        let Some(((x, y), zoom, camera)) = self.anchored_place(k.anchor, k.press, k.size) else { return };
        let (w, h) = (scaled_up(k.size.0, zoom), scaled_up(k.size.1, zoom));
        let (vw, vh) = (camera.viewport.0 as i32, camera.viewport.1 as i32);
        let pan = (fit_pan(x - camera.origin.0, w, vw), fit_pan(y - camera.origin.1, h, vh));
        if pan == (0, 0) {
            return;
        }
        let by = (world_steps(pan.0, camera.zoom), world_steps(pan.1, camera.zoom));
        let pointer = self.follow_of(k.press.hit) == Some(Follow::Pointer);
        if !pointer {
            let Some(c) = self.free_view(k.press.hit).map(|v| v.center) else { return };
            self.place_view(k.press.hit, (c.0 + by.0, c.1 + by.1));
        }
        if let Some(kept) = self.detached.kept.iter_mut().find(|k| k.serial == serial) {
            (kept.pan, kept.pointer) = (by, pointer);
        }
        log::info!("viewer: the tile box {serial} needs {pan:?} more room in its view: moved by {by:?} until it closes");
    }

    /// Moves the views moved for boxes now closed back by as much.
    fn detached_unpan(&mut self) {
        for (hit, by) in std::mem::take(&mut self.detached.unpan) {
            if let Some(c) = self.free_view(hit).map(|v| v.center) {
                self.place_view(hit, (c.0 - by.0, c.1 - by.1));
            }
        }
    }

    /// How far the kept boxes move the view following the world cursor at
    /// `hit` while they are there (world pixels).
    pub(super) fn detached_pointer_pan(&self, hit: Hit) -> (i32, i32) {
        self.detached.kept.iter().filter(|k| k.pointer && k.press.hit == hit).fold((0, 0), |a, k| (a.0 + k.pan.0, a.1 + k.pan.1))
    }

    /// Where the kept popups go on the canvas now (view, (top-left, zoom)).
    pub(super) fn detached_places(&self) -> Vec<(u32, ((i32, i32), (u32, u32)))> {
        self.detached.kept.iter()
            .filter_map(|k| self.anchored_place(k.anchor, k.press, k.size).map(|(at, zoom, _)| (k.view, (at, zoom))))
            .collect()
    }

    /// The view a press or key at `p` belongs to: the world cursor's while
    /// the mouse is captured.
    fn detached_hit(&self, p: (i32, i32)) -> Option<Hit> {
        if self.locked {
            self.shown.iter().find(|(h, _)| self.follow_of(*h) == Some(Follow::Pointer)).map(|(h, _)| *h)
        } else {
            Some(self.hit(p))
        }
    }

    /// A mouse button at `p`: `Some` (what goes to the driver: the
    /// popups it closes) if it was for a kept popup, a press in its view
    /// or on it; its release then does nothing.
    pub(super) fn detached_button(&mut self, button: Button, down: bool, p: (i32, i32)) -> Option<Vec<Input>> {
        let b = button_index(button);
        if !down {
            return std::mem::take(&mut self.detached.swallowed[b]).then(Vec::new);
        }
        if self.detached.kept.is_empty() {
            return None;
        }
        let hit = self.detached_hit(p);
        let on_box = |k: &Kept, at: ((i32, i32), (u32, u32))| {
            let ((x, y), zoom) = at;
            inside([x, y, scaled_up(k.size.0, zoom), scaled_up(k.size.1, zoom)], p)
        };
        let places = self.detached_places();
        let closing: Vec<u64> = self.detached.kept.iter().filter(|k| {
            Some(k.press.hit) == hit || (!self.locked && places.iter().any(|&(view, at)| view == k.view && on_box(k, at)))
        }).map(|k| k.serial).collect();
        if closing.is_empty() {
            return None;
        }
        for serial in closing {
            self.detached.close(serial);
        }
        self.detached_unpan();
        self.detached.swallowed[b] = true;
        log::info!("viewer: a press in its view closes the kept tile box");
        Some(self.detached.take_inputs())
    }

    /// Esc: closes the kept popup of the view the pointer is in, if any
    /// (it then does not reach the driver, where it would open its quit
    /// dialog).
    pub(super) fn detached_escape(&mut self) -> bool {
        let hit = self.detached_hit(self.pointer);
        let closing: Vec<u64> = self.detached.kept.iter().filter(|k| Some(k.press.hit) == hit).map(|k| k.serial).collect();
        for &serial in &closing {
            self.detached.close(serial);
        }
        self.detached_unpan();
        !closing.is_empty()
    }
}
