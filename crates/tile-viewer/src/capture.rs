//! When the mouse may be captured, and capturing it again after menus. A
//! child of `layout`: these are [`Layout`]'s methods.
//!
//! # Menus
//!
//! A menu is anything of the driver's that takes the mouse over the world:
//! a named screen (a report screen, a site screen, the game menu,
//! Preferences, an encyclopedia, a negotiation) or a popup (a dialog, a context menu, the
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
//! beside the menu (playing there while it is open, `docs/modal-input.md`):
//! that is the player's capture, not a menu's. A view is beside the menu
//! when no open menu lies over it ([`Cover`], as of the last frame): a
//! screen fitted into the other pane (or the game menu following the
//! pointer into it), a popup placed in the other pane by the screen place
//! setting or the view it was opened from. A floating screen, and a popup
//! placed nowhere in particular (a single view, at a click), lie over every
//! view; a popup opened at a world click is dismissed by such a click
//! instead (and the mouse captured once it is gone). Over a view beside
//! the menu the pointer shows that a click captures
//! ([`Layout::take_pointer_icon`]); the click captures and goes nowhere
//! else. That capture stays while the menu is open (`settle_capture` lets
//! go only of a capture in a view a menu lies over); the divider and the
//! window's edges hold the mouse as for any capture. Ctrl on its own lets
//! it go, and so does the Alt mode's click on the menu outside the captured
//! view ([`Layout::menu_clicked`], the click going to the menu). Capturing
//! beside a menu turns off the Alt mode the menu turned on, as Alt or Ctrl
//! would (`crate::snap`: off until the menus are all closed).
//!
//! Losing the window's focus forgets a pending capture: coming back, a
//! click captures the mouse, as before. Activation never captures it.
//!
//! # The dialogues that let go of the pointer
//!
//! A player who wants to leave the window and does not know how may open
//! the game menu (Save, Load, Quit Game) or press Escape on the map (the
//! "really quit?" popup). While either is open the driver says so in every
//! scene (`scene::Interface::releases_pointer`, from the game's state) and
//! nothing holds the pointer: the mouse is let go wherever it was captured
//! (beside the menu too) and is not captured by a click, Ctrl+Shift+Space
//! or the Alt mode ([`Layout::capture_refused`]); the divider, the zoom
//! boxes and the window's edges hold the free pointer nowhere
//! ([`Layout::pointer_unbounded`]), so the window asks the system for no
//! confinement either (ClipCursor, Wayland constraints, X11 barriers, the
//! macOS region) and warps it nowhere. The one exception is the Alt mode
//! (`crate::snap`): it works in these dialogues as in any menu, its own
//! grab holding the pointer while it is on; with it off the pointer is free
//! to leave the window on every system.
//!
//! As the dialogue closes, everything that is decided from the frame (the
//! walls, the confinement) is as before at once. A capture the dialogue
//! interrupted is taken up again as after any menu, at the same world
//! cursor once no menu has been open for [`RESUME_AFTER`], only if the
//! pointer is in the window then: a player who used the dialogue to leave
//! the window is not pulled back into it, and a click into the view
//! captures the mouse again, as always. A mouse that was free stays free.
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

/// Where the open menus lie over the window (window rectangles), as of
/// the last frame: the pane a screen is fitted into or a menu follows the
/// pointer into, the view a popup is placed in, else the whole window.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Cover {
    /// The named screen's.
    pub screen: Vec<[i32; 4]>,
    /// The popup's (over a screen in a pane: that pane).
    pub popup: Vec<[i32; 4]>,
}

/// A capture a menu interrupted, waiting for the menus to be gone.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(super) struct Resume {
    /// The world cursor to capture the mouse at again.
    pub at: Option<(i32, i32)>,
    /// Since when (the layout's clock) no menu has been open.
    pub clear_since: Option<f64>,
    /// A dialogue that lets go of the pointer was open since the capture
    /// was interrupted: taken up again only with the pointer in the window.
    pub released: bool,
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
        if self.releases_pointer {
            return Some("the game menu or the quit dialogue lets the pointer go");
        }
        if hit == Hit::Beside {
            return None;
        }
        if self.map_movement() == MapMovement::Drag {
            return Some("map movement is Drag: no capture");
        }
        // (One opened at a world click: a press beside it dismisses it,
        // `Layout::dismissing_press`.)
        if popup && self.click_popup_open() {
            return Some("a popup is open");
        }
        let view = self.shown.iter().find(|(h, _)| *h == hit).map(|(_, c)| [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]);
        let covers = |rects: &[[i32; 4]]| view.is_none_or(|v| rects.iter().any(|&r| super::overlap(r, v)));
        if popup && self.popup_shown && covers(&self.menu_cover.popup) {
            return Some("a popup is open over it");
        }
        if self.screen.is_some() && covers(&self.menu_cover.screen) {
            return Some("a screen is open over it");
        }
        None
    }

    /// Where the open menus lie this frame ([`Cover`]), once the screen and
    /// the popup are known: `following_menu`, the game menu placed in the
    /// pane it was opened from; `over_menu`, the popup is over a menu.
    pub(super) fn cover_menus(&mut self, following_menu: bool, over_menu: bool) {
        let whole = [0, 0, self.window.0 as i32, self.window.1 as i32];
        let (a, b) = self.panes();
        let pane = |h: Hit| if h == Hit::Pane { b.unwrap_or(a) } else { a };
        let fitted = self.fit_target(self.screen.as_deref()).or(following_menu.then(|| self.menu_side.unwrap_or(super::Side::A).hit()));
        let screen = match (&self.screen, fitted) {
            (None, _) => vec![],
            (Some(_), Some(h)) => vec![pane(h)],
            (Some(_), None) => vec![whole],
        };
        let popup = if !self.popup_shown {
            vec![]
        } else if over_menu {
            // (Over a screen in a pane it goes over that pane.)
            if screen.is_empty() || screen.contains(&whole) { vec![whole] } else { screen.clone() }
        } else {
            vec![self.popup_region().unwrap_or(whole)]
        };
        self.menu_cover = Cover { screen, popup };
    }

    /// The view following the world cursor the mouse is captured in.
    fn captured_view(&self) -> Option<Hit> {
        if !self.locked {
            return None;
        }
        self.shown.iter().map(|(h, _)| *h).find(|&h| matches!(h, Hit::Main | Hit::Pane | Hit::Inset(_)) && self.follow_of(h) == Some(super::Follow::Pointer))
    }

    /// A click of the Alt mode's pointer at window pixel `p` on the
    /// driver's interface while a menu is open: outside the view the mouse
    /// is captured in, it lets the mouse go (the click goes to the menu).
    pub fn menu_clicked(&mut self, p: (i32, i32)) {
        let Some(hit) = self.captured_view().filter(|_| self.menu_open()) else { return };
        let inside = self.shown.iter().find(|(h, _)| *h == hit).is_some_and(|(_, c)| super::inside([c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32], p));
        if !inside {
            self.release("a click on the menu outside the captured view");
        }
    }

    /// Whether the mouse may be captured in a view following the world
    /// cursor at all (the Alt mode's attractors capture or just put the
    /// pointer there).
    pub fn may_capture(&self) -> bool {
        self.map_movement() != MapMovement::Drag && !self.releases_pointer
    }

    /// Whether a dialogue that lets go of the pointer is open (the game
    /// menu, the quit dialogue: see the module's documentation).
    pub fn releases_pointer(&self) -> bool {
        self.releases_pointer
    }

    /// The window says whether the system pointer is in it.
    pub fn set_pointer_inside(&mut self, inside: bool) {
        self.pointer_inside = inside;
    }

    /// Nothing holds the free pointer back: no divider, no window edge, no
    /// system confinement ([`Layout::pointer_free_everywhere`], or a
    /// dialogue that lets go of the pointer is open).
    pub(super) fn pointer_unbounded(&self) -> bool {
        self.releases_pointer || self.pointer_free_everywhere()
    }

    /// With Drag map movement the free pointer goes anywhere: the divider
    /// holds nothing back and the system confines nothing, so it may leave
    /// the window (but while a right-drag previews a piece's order, which
    /// still scrolls at the window's edge).
    pub(super) fn pointer_free_everywhere(&self) -> bool {
        !self.locked && self.map_movement() == MapMovement::Drag && !self.world_order_dragging()
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
            self.world_order_events_push(scene::Input::DismissPopup);
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
            self.resume = Resume { at: Some(at), ..Resume::default() };
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
        if self.releases_pointer {
            // (Wherever it was captured: beside the menu too.)
            if self.locked {
                self.suspend_capture();
                self.release_here("the game menu or the quit dialogue is open: nothing holds the pointer");
            }
            if self.resume.at.is_some() {
                self.resume.released = true;
            }
            self.resume.clear_since = None;
            return;
        }
        if self.locked && self.explore.is_none() {
            if self.map_movement() == MapMovement::Drag {
                self.forget_resume();
                self.release_here("map movement is Drag");
            } else if self.menu_open() && self.captured_view().is_none_or(|h| self.capture_refused(h).is_some()) {
                // (The menu's own opening let go already; any other way it
                // got captured since is undone here. The player's capture
                // in a view no menu lies over stays.)
                self.suspend_capture();
                self.release_here("a menu is open over the captured view");
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
        if self.resume.released && !self.pointer_inside {
            self.forget_resume();
            log::info!("viewer: the mouse stays free (the pointer is outside the window as the dialogue that let it go closed); a click into the view following the world cursor captures it");
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
