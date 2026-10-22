//! When the mouse may be captured, and capturing it again after menus. A
//! child of `layout`: these are [`Layout`]'s methods.
//!
//! # Menus
//!
//! A menu is anything of the driver's that takes the mouse over the world:
//! a named screen (an advisor, the city screen, the game menu, Preferences,
//! the Civilopedia, diplomacy) or a popup (a dialog, a context menu, the
//! tile information box), as [`Layout::menu_open`] says. While one is open
//! the mouse is never captured by the layout on its own: the menu that
//! opens over a captured mouse lets it go and remembers the world cursor
//! ([`Resume`]), and nothing captures it again while any menu is still
//! open, however menus follow each other (a popup giving way to a screen,
//! a screen to the next, a popup over a screen closing first). Only once
//! no menu has been open for [`RESUME_AFTER`] is the mouse captured again
//! at that world cursor, so a menu following another a moment later never
//! sees a capture in between.
//!
//! Before, a popup and a screen each kept their own "capture again" and
//! used it as soon as *they* closed: a popup closing as a screen opened
//! (in the same frame, or with the screen already up) captured the mouse
//! under the screen.
//!
//! The player may still click into the view following the world cursor
//! beside a screen placed in the other pane (playing there while the
//! screen is open, `docs/modal-input.md`): that is the player's capture,
//! not a menu's. Over a floating screen, and while any popup is open, such
//! a click does not capture.
//!
//! Losing the window's focus forgets a pending capture: coming back, a
//! click captures the mouse, as before. Activation never captures it.
//!
//! # Drag map movement
//!
//! With `MapMovement::Drag` the window's edges and the divider do nothing,
//! so nothing captures the mouse: a click into the view following the world
//! cursor is a click on the world there, the pointer is free to leave the
//! window, and the divider holds nothing back. Switching to Drag while
//! captured lets the mouse go. Exploring a scenario's world beside its
//! screen (`Hit::Beside`) is its own gesture and still captures.

use viewer_settings::MapMovement;

use super::{Hit, Layout};

/// How long no menu must have been open before a mouse a menu let go is
/// captured again (seconds of the layout's clock).
pub const RESUME_AFTER: f64 = 0.2;

/// Where the Alt mode's pointer is drawn to, and a click goes back to
/// playing in a view (`crate::snap`, [`Layout::attractors`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Attractor {
    /// The view's window rectangle (x, y, w, h).
    pub rect: [i32; 4],
    /// The point to click: the view's middle (else the world cursor in it,
    /// should the driver's interface cover the middle).
    pub at: (i32, i32),
    /// A click there captures the mouse (the view follows the world cursor
    /// and capturing is allowed); else it puts the free pointer there.
    pub captures: bool,
}

/// A capture a menu interrupted, waiting for the menus to be gone.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(super) struct Resume {
    /// The world cursor to capture the mouse at again.
    pub at: Option<(i32, i32)>,
    /// Since when (the layout's clock) no menu has been open.
    pub clear_since: Option<f64>,
}

impl Layout {
    /// Whether a menu of the driver's is open over the world: a named
    /// screen or a popup (see the module's documentation).
    pub fn menu_open(&self) -> bool {
        self.screen.is_some() || self.popup_shown
    }

    /// Why the mouse may not be captured in view `hit` now, by the player
    /// (a click, Ctrl+Shift+Space, the Alt mode's attractor); `None`: it
    /// may be.
    pub(super) fn capture_refused(&self, hit: Hit) -> Option<&'static str> {
        self.capture_refused_by(hit, true)
    }

    /// [`Layout::capture_refused`], the open popup counting or not.
    fn capture_refused_by(&self, hit: Hit, popup: bool) -> Option<&'static str> {
        if hit == Hit::Beside {
            return None;
        }
        if self.map_movement() == MapMovement::Drag {
            return Some("map movement is Drag: no capture");
        }
        if popup && self.popup_shown {
            return Some("a popup is open");
        }
        if self.screen.is_some() && !self.screen_beside_world() {
            return Some("a screen is open over the world");
        }
        None
    }

    /// Whether the open screen is placed into one pane, the other showing
    /// the world to play in.
    fn screen_beside_world(&self) -> bool {
        self.fit_target(self.screen.as_deref()).is_some()
    }

    /// Whether the mouse may be captured in a view following the world
    /// cursor at all (the Alt mode's attractors capture or just put the
    /// pointer there).
    pub fn may_capture(&self) -> bool {
        self.map_movement() != MapMovement::Drag
    }

    /// With Drag map movement the free pointer goes anywhere: the divider
    /// holds nothing back and the system confines nothing, so it may leave
    /// the window (but while a right-drag previews a unit's order, which
    /// still scrolls at the window's edge).
    pub(super) fn pointer_free_everywhere(&self) -> bool {
        !self.locked && self.map_movement() == MapMovement::Drag && !self.unit_order_dragging()
    }

    /// A menu lets go of the captured mouse: captured again at the world
    /// cursor once the menus are gone (unless the mouse was already let go
    /// for an earlier one, whose world cursor stays).
    pub(super) fn suspend_capture(&mut self) {
        if self.locked && self.map_movement() != MapMovement::Drag {
            self.resume.at.get_or_insert(self.cursor);
        }
        self.resume.clear_since = None;
    }

    /// The player asks to capture the mouse in view `into` at window pixel
    /// `p` (Ctrl+Shift+Space, the Alt mode's attractor): captured now, or
    /// a popup opened at a world click dismissed and the mouse captured
    /// once it is gone, or refused (Drag, another menu). Whether it was
    /// captured now.
    pub(super) fn player_capture(&mut self, into: Hit, p: (i32, i32)) -> bool {
        if self.click_popup_open() {
            self.capture_after_dismissal(into);
            self.unit_order_events_push(scene::Input::DismissPopup);
            return false;
        }
        if let Some(why) = self.capture_refused(into) {
            log::info!("viewer: mouse stays free ({why})");
            return false;
        }
        self.capture(into, p);
        true
    }

    /// A press dismissing a popup, made where a click captures the mouse in
    /// `hit`: captured once the popup (and every other menu) is gone, if
    /// nothing but the popup is in the way.
    pub(super) fn capture_after_dismissal(&mut self, hit: Hit) {
        if self.capture_refused_by(hit, false).is_some() {
            return;
        }
        if let Some(at) = self.view_middle(hit) {
            self.resume = Resume { at: Some(at), clear_since: None };
            log::info!("viewer: the mouse is captured in {hit:?} once the popup is gone");
        }
    }

    /// The world pixel in the middle of view `hit` as last shown (where a
    /// capture puts the world cursor).
    pub(super) fn view_middle(&self, hit: Hit) -> Option<(i32, i32)> {
        let (_, camera) = self.shown.iter().find(|(h, _)| *h == hit)?;
        let (w, h) = camera.world_size();
        Some((camera.x + w as i32 / 2, camera.y + h as i32 / 2))
    }

    /// The Alt mode's attractors as of the last frame: one in each view of
    /// a split (A and B), else in the single view if it follows the world
    /// cursor; the views following the world cursor first. Not where the
    /// driver's interface (a screen fitted into the view, a menu over it)
    /// covers the point.
    pub fn attractors(&self) -> Vec<Attractor> {
        let candidates: &[Hit] = if self.split.is_some() { &[Hit::Main, Hit::Pane] } else { &[Hit::Main] };
        let mut out: Vec<Attractor> = Vec::new();
        for &hit in candidates {
            let Some((_, c)) = self.shown.iter().find(|(h, _)| *h == hit) else { continue };
            let follows = self.follow_of(hit) == Some(super::Follow::Pointer);
            if self.split.is_none() && !follows {
                continue;
            }
            let rect = [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32];
            let middle = (rect[0] + rect[2] / 2, rect[1] + rect[3] / 2);
            let cursor = self.cursor_on_window().filter(|&p| follows && super::inside(rect, p));
            let reaches = |at: (i32, i32)| if follows { self.capture_hit(at) == Some(hit) } else { self.hit(at) == hit };
            let Some(at) = [Some(middle), cursor].into_iter().flatten().find(|&at| reaches(at)) else { continue };
            out.push(Attractor { rect, at, captures: follows && self.may_capture() });
        }
        out.sort_by_key(|a| !a.captures);
        out
    }

    /// The Alt mode's attractor at window pixel `p` was clicked (`crate::snap`):
    /// back to playing in its view. Over a view following the world cursor
    /// the mouse is captured as a click there would (where it may); else the
    /// free pointer goes there (a captured mouse let go there).
    pub fn attractor_clicked(&mut self, p: (i32, i32)) {
        if self.locked {
            if self.capture_hit(p).is_none() {
                self.let_go_at(p);
            }
            return;
        }
        if !self.capture_at(p) {
            self.place_pointer(p);
        }
    }

    /// A pending capture is forgotten (the window lost the focus, the world
    /// closed, the player let go of the mouse otherwise).
    pub(super) fn forget_resume(&mut self) {
        self.resume = Resume::default();
    }

    /// The one place, once a frame after the screen and popups are known,
    /// that decides the capture: a menu or Drag that should not have the
    /// mouse captured lets it go; with no menu open for [`RESUME_AFTER`],
    /// a capture a menu interrupted is taken up again.
    pub(super) fn settle_capture(&mut self) {
        if self.locked && self.explore.is_none() {
            if self.map_movement() == MapMovement::Drag {
                self.forget_resume();
                self.release_here("map movement is Drag");
            } else if self.popup_shown || (self.screen.is_some() && !self.screen_beside_world()) {
                // (The menu's own opening let go already; any other way it
                // got captured since is undone here.)
                self.suspend_capture();
                self.release_here("a menu is open");
            }
        }
        if self.menu_open() {
            self.resume.clear_since = None;
            return;
        }
        let Some(at) = self.resume.at else { return };
        if self.locked || self.map_movement() == MapMovement::Drag || !self.follows_pointer() {
            self.forget_resume();
            return;
        }
        let since = *self.resume.clear_since.get_or_insert(self.clock);
        if self.clock - since < RESUME_AFTER {
            return;
        }
        self.forget_resume();
        self.cursor = at;
        self.locked = true;
        self.grab = Some(true);
        self.against = None;
        log::info!("viewer: mouse captured again (the menus closed)");
    }
}
