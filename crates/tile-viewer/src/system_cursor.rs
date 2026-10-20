//! CPU cursor metadata follows received scenes independently of presented frames.

use std::sync::Arc;
use crate::cursor::Art;

#[derive(Default)]
pub(crate) struct SystemCursor {
    base: u64,
    art: Arc<Art>,
    cursor: scene::Cursor,
}

impl SystemCursor {
    /// A replacement's sprite ids become usable with its CPU cursor pixels,
    /// without waiting for an atlas upload or presentation. Older scenes keep
    /// their previous art until they refer to the replacement.
    pub fn refresh(&mut self, scene: &scene::Scene, base: u64, through: u64, art: Arc<Art>) -> bool {
        let compatible = scene.assets >= base && scene.assets <= through;
        let initial = self.base == 0 && scene.seq == 0;
        let changed = (compatible || initial) && self.base != base;
        if changed {
            self.base = base;
            self.art = art;
        }
        let old_scene = self.base != 0 && scene.assets >= self.base && scene.assets < base;
        if compatible || old_scene || matches!(scene.cursor, scene::Cursor::Default | scene::Cursor::System(_)) {
            self.cursor = scene.cursor;
        }
        changed
    }

    /// A target command's valid/invalid icon takes precedence over the
    /// ordinary map hint that a click would capture the mouse.
    pub fn animating(&self) -> bool {
        matches!(self.cursor, scene::Cursor::System(scene::SystemCursor::Wait | scene::SystemCursor::Progress))
    }

    /// Only a visible, loaded animation needs a timer. Use its next host-time
    /// boundary rather than delaying another full frame after every input.
    pub fn next_animation_wake(&self, hidden: bool, elapsed: std::time::Duration) -> Option<std::time::Duration> {
        animation_wake(self.cursor, self.art.busy_frames(), hidden, elapsed)
    }

    pub fn shown_at(&self, hidden: bool, capture: bool, targeting: bool, elapsed: std::time::Duration) -> crate::cursor::Shown {
        let cursor = if self.animating() {
            self.art.busy_cursor(elapsed).map_or(scene::Cursor::Default, scene::Cursor::Sprite)
        } else { self.cursor() };
        crate::cursor::system_pointer(cursor, self.art(), hidden, capture && !targeting)
    }

    pub fn shown(&self, hidden: bool, capture: bool, targeting: bool) -> crate::cursor::Shown {
        self.shown_at(hidden, capture, targeting, std::time::Duration::ZERO)
    }

    pub fn art(&self) -> &Art { &self.art }

    pub fn cursor(&self) -> scene::Cursor { self.art.system_shown(self.cursor) }
}

fn animation_wake(cursor: scene::Cursor, frames: usize, hidden: bool, elapsed: std::time::Duration) -> Option<std::time::Duration> {
    if hidden || frames < 2 || !matches!(cursor, scene::Cursor::System(scene::SystemCursor::Wait | scene::SystemCursor::Progress)) {
        return None;
    }
    let period = scene::loading_cursor::FRAME_MS;
    let remaining = period - (elapsed.as_millis() % u128::from(period)) as u64;
    Some(std::time::Duration::from_millis(remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::{Cursor, Scene, SystemCursor as Icon};

    fn scene(assets: u64, cursor: Cursor) -> Scene { Scene { seq: 1, assets, cursor, ..Default::default() } }

    #[test]
    fn cpu_cursor_changes_without_a_presented_scene_or_uploaded_art() {
        let mut cursor = SystemCursor::default();
        assert!(cursor.refresh(&scene(10, Cursor::Sprite(7)), 10, 12, Arc::default()));
        assert_eq!(cursor.cursor(), Cursor::Sprite(7));
        assert!(!cursor.refresh(&scene(12, Cursor::Sprite(8)), 10, 12, Arc::default()));
        assert_eq!(cursor.cursor(), Cursor::Sprite(8));
    }

    #[test]
    fn replacement_art_never_pairs_new_sprite_ids_with_the_old_generation() {
        let mut cursor = SystemCursor::default();
        cursor.refresh(&scene(10, Cursor::Sprite(7)), 10, 10, Arc::default());
        assert!(!cursor.refresh(&scene(10, Cursor::Sprite(8)), 20, 20, Arc::default()));
        assert_eq!(cursor.cursor(), Cursor::Sprite(8), "the old scene still uses old art");
        assert!(!cursor.refresh(&scene(20, Cursor::Sprite(99)), 10, 10, Arc::default()));
        assert_eq!(cursor.cursor(), Cursor::Sprite(8), "future sprite ids wait for matching CPU art");
        assert!(cursor.refresh(&scene(20, Cursor::Sprite(99)), 20, 20, Arc::default()));
        assert_eq!(cursor.cursor(), Cursor::Sprite(99));
    }

    #[test]
    fn targeting_icon_takes_precedence_over_the_free_map_capture_hint() {
        let mut cursor = SystemCursor::default();
        cursor.refresh(&scene(10, Cursor::System(Icon::NotAllowed)), 10, 10, Arc::default());
        assert_eq!(cursor.shown(false, true, true), crate::cursor::Shown::Icon(winit::window::CursorIcon::NotAllowed));
        assert_eq!(cursor.shown(false, true, false), crate::cursor::Shown::Icon(winit::window::CursorIcon::Crosshair));
        assert_eq!(cursor.shown(true, true, true), crate::cursor::Shown::Hidden, "explicit virtual-pointer modes still hide the OS cursor");
    }

    #[test]
    fn animation_wakes_only_for_visible_multiframe_busy_cursors() {
        use std::time::Duration as D;
        for mode in [Cursor::Default, Cursor::Sprite(7), Cursor::System(Icon::Arrow)] {
            assert_eq!(animation_wake(mode, 30, false, D::ZERO), None);
        }
        for mode in [Icon::Wait, Icon::Progress] {
            for frames in [0,1] { assert_eq!(animation_wake(Cursor::System(mode), frames, false, D::ZERO), None); }
            assert_eq!(animation_wake(Cursor::System(mode),30,true,D::ZERO),None);
            for (elapsed, wait) in [(0,83),(1,82),(82,1),(83,83),(166,83)] {
                let at = D::from_millis(elapsed);
                assert_eq!(animation_wake(Cursor::System(mode),30,false,at),Some(D::from_millis(wait)));
                let current = scene::loading_cursor::frame_at(at);
                assert_eq!(scene::loading_cursor::frame_at(at + D::from_millis(wait - 1)), current, "no wake before the picture advances");
                assert_ne!(scene::loading_cursor::frame_at(at + D::from_millis(wait)), current, "wake at the next picture");
            }
        }
    }

    #[test]
    fn busy_modes_never_request_the_host_loading_animation() {
        for mode in [Icon::Wait, Icon::Progress] {
            let mut cursor = SystemCursor::default();
            cursor.refresh(&scene(10, Cursor::System(mode)), 10, 10, Arc::default());
            assert_eq!(cursor.shown_at(false, false, false, std::time::Duration::from_millis(83)), crate::cursor::Shown::Icon(winit::window::CursorIcon::Default));
            assert_eq!(cursor.shown_at(true, false, false, std::time::Duration::ZERO), crate::cursor::Shown::Hidden);
        }
    }

    #[test]
    fn system_busy_cursor_is_independent_of_atlas_generation() {
        let mut cursor = SystemCursor::default();
        cursor.refresh(&scene(99, Cursor::System(Icon::Wait)), 10, 10, Arc::default());
        assert_eq!(cursor.cursor(), Cursor::System(Icon::Wait));
    }
}
