//! The on-screen count of render errors (`render_errors`): "During the last
//! second, N rendering errors happened." in the game's message area, the
//! column at the top left below the corner buttons where the game lists its
//! chat and log messages, below every message there: the messages are
//! never covered nor moved. Drawn by the viewer itself (no game message), in
//! the game's message font and style (white, with a dark shadow), at the
//! corner buttons' scale (the HUD scale).
//!
//! N counts every render error of the last second on both sides, repeats of
//! types already logged included ([`render_errors::Rolling`]). The notice
//! shows while there are any and for [`render_errors::HOLD`] after (so that
//! a single error can be read), its number changing at most every
//! [`render_errors::REFRESH`] ([`render_errors::Notice`]).
//!
//! What lies in the message column is what the driver's interface draws
//! there: the screen sprites of its interface views, other than the
//! corner's own and the menus, popups and screens, starting right below the
//! corner and following on line by line ([`message_lines`], [`place`]).

use render_desc::Compiled;
use tile_render::{Camera, GlyphShape, OutlineGlyph, PlacedSprite};

use crate::host_status::{self, Anchor};

/// The game's message font, as the driver's mirrored text and the
/// "Press CTRL" hint use it.
pub const FONT_PX: u32 = 14;
/// White on the map, with a dark shadow, as the game's messages.
pub const COLOR: [u8; 3] = [255, 255, 255];
pub const SHADOW: [u8; 3] = [0, 0, 0];
/// Pixels at 1× (scaled with the HUD): the left margin with no message to
/// align to, the gap above the notice, how far apart lines of one column
/// of messages may be, how tall one line of it is at most.
pub const MARGIN: i32 = 8;
pub const GAP: i32 = 4;
pub const LINE_REACH: i32 = 24;
pub const LINE_HEIGHT: i32 = 40;

fn scaled(v: i32, (n, d): (u32, u32)) -> i32 {
    (i64::from(v) * i64::from(n.max(1)) / i64::from(d.max(1))) as i32
}

/// The corner buttons' window rectangle (x, y, w, h; empty below the
/// world's or window's corner when they are not shown) and their scale.
pub fn corner(anchor: Anchor) -> ([i32; 4], (u32, u32)) {
    match anchor {
        Anchor::Beside(r, n) => (r, n),
        Anchor::Inside([x, y, _, _], n) => ([x + scaled(MARGIN, n), y + scaled(MARGIN, n), 0, 0], n),
        Anchor::Window => ([MARGIN, MARGIN, 0, 0], (1, 1)),
    }
}

/// The window rectangles of what the driver's interface draws in the
/// message column below `corner` (window pixels): the screen sprites of
/// its interface views other than the corner's, the menus', popups' and
/// screens', that start left of the corner's right edge, below it, and are
/// no taller than a line.
pub fn message_lines(compiled: &Compiled, scene: &scene::Scene, cameras: &[Camera], view_cameras: &[Option<u32>], overlay_cameras: &[(usize, Vec<u32>)], corner: [i32; 4], scale: (u32, u32)) -> Vec<[i32; 4]> {
    let i = &scene.interface;
    let apart = |id: u32| {
        [i.roaming, i.capture_hint, i.screen_view, i.loading_bar].contains(&Some(id))
            || i.is_popup_view(id)
            || i.detached.iter().any(|d| d.view == id)
            || i.queued.iter().any(|q| q.view == id)
    };
    let (right, bottom, tallest) = (corner[0] + corner[2].max(scaled(MARGIN * 2, scale)), corner[1] + corner[3], scaled(LINE_HEIGHT, scale));
    // Where camera `c` draws the screen rectangle `r` of view `v` (window
    // pixels, within its viewport), as the renderer places screen sprites:
    // from its origin, less its screen shift, at its zoom.
    let window = |v: &scene::ViewState, c: usize, [x, y, w, h]: [i32; 4]| -> Option<[i32; 4]> {
        let camera = cameras.get(c)?;
        let (viewport, zoom) = host_status::camera_rect(cameras, c)?;
        let (l, t) = if camera.stacked {
            let head = cameras[..c].iter().rev().find(|h| !h.stacked).unwrap_or(camera);
            (head.origin.0 + scaled(camera.origin.0 + x - v.view.origin.0, zoom), head.origin.1 + scaled(camera.origin.1 + y - v.view.origin.1, zoom))
        } else {
            let base = (camera.origin.0 - camera.screen_shift.0, camera.origin.1 - camera.screen_shift.1);
            (camera.origin.0 + scaled(x - base.0, zoom), camera.origin.1 + scaled(y - base.1, zoom))
        };
        let (r, b) = (l + scaled(w, zoom), t + scaled(h, zoom));
        let (l, t, r, b) = (l.max(viewport[0]), t.max(viewport[1]), r.min(viewport[0] + viewport[2]), b.min(viewport[1] + viewport[3]));
        (r > l && b > t).then_some([l, t, r - l, b - t])
    };
    let mut lines = Vec::new();
    for (k, v) in scene.views.iter().enumerate() {
        if !v.view.interface_only || v.view.menu || apart(v.id) {
            continue;
        }
        // (A view in panels: a camera per sprite, `scaling::Arranged`.)
        let per_sprite = overlay_cameras.iter().find(|(view, _)| *view == k).map(|(_, c)| c);
        let Some(view_camera) = view_cameras.get(k).copied().flatten().or(per_sprite.map(|_| u32::MAX)) else { continue };
        for (n, s) in v.overlay.iter().enumerate().filter(|(_, s)| s.screen) {
            let c = per_sprite.map_or(Some(view_camera), |c| c.get(n).copied());
            let Some(r) = crate::sprite_rect(compiled, s).zip(c).and_then(|(r, c)| window(v, c as usize, r)) else { continue };
            if r[0] < right && r[1] >= bottom && r[3] <= tallest {
                lines.push(r);
            }
        }
    }
    lines
}

/// The notice's top-left for its `size` in a window of `window`: below the
/// messages that follow on from the corner (`corner`, at `scale`) line by
/// line, aligned with them; else beside them, right of the widest; never
/// over a message nor outside the window (`None` then: no room).
pub fn place(corner: [i32; 4], scale: (u32, u32), lines: &[[i32; 4]], size: (i32, i32), window: (u32, u32)) -> Option<(i32, i32)> {
    let mut sorted = lines.to_vec();
    sorted.sort_by_key(|l| l[1]);
    let reach = scaled(LINE_REACH, scale);
    let mut bottom = corner[1] + corner[3];
    let mut column = Vec::new();
    for l in sorted {
        if l[1] > bottom + reach {
            break;
        }
        bottom = bottom.max(l[1] + l[3]);
        column.push(l);
    }
    let left = column.iter().map(|l| l[0]).min().unwrap_or(corner[0] + scaled(MARGIN, scale)).max(0);
    let (ww, wh) = (window.0.min(i32::MAX as u32) as i32, window.1.min(i32::MAX as u32) as i32);
    let fits = |[x, y]: [i32; 2]| {
        x >= 0 && y >= 0 && x + size.0 <= ww && y + size.1 <= wh
            && !lines.iter().any(|l| x < l[0] + l[2] && l[0] < x + size.0 && y < l[1] + l[3] && l[1] < y + size.1)
    };
    let below = [left, bottom + scaled(GAP, scale)];
    // Beside the column: right of its widest line, from the corner down.
    let widest = column.iter().map(|l| l[0] + l[2]).max().unwrap_or(left);
    let beside = [widest + scaled(MARGIN, scale), corner[1] + corner[3] + scaled(GAP, scale)];
    [below, beside].into_iter().find(|&p| fits(p)).map(|[x, y]| (x, y))
}

/// The notice for `n` errors: its camera (the window 1:1) and sprites, the
/// shadow under each glyph; `None` without a font that has its characters
/// or room for it.
pub fn compose(compiled: &Compiled, n: u64, corner: [i32; 4], scale: (u32, u32), lines: &[[i32; 4]], window: (u32, u32), camera: u32) -> Option<(Camera, Vec<PlacedSprite>)> {
    let text = render_errors::notice_text(n);
    let (font, face) = host_status::face_for(compiled, &text)?;
    let px = (scaled(FONT_PX as i32, scale) as u32).clamp(FONT_PX, 1024);
    let em = i64::from(face.units_per_em().max(1));
    let letters: Vec<_> = text.chars().map(|ch| (face.glyph(ch).unwrap(), ((i64::from(face.advance(ch).unwrap()) * i64::from(px) + em / 2) / em) as i32)).collect();
    let width: i32 = letters.iter().map(|l| l.1).sum();
    let shadow = scaled(1, scale).max(1);
    // The line's height: the em, room for descenders and the shadow.
    let size = (width + shadow, px as i32 * 4 / 3 + shadow);
    let (x, y) = place(corner, scale, lines, size, window)?;
    let baseline = y + px as i32;
    let clip = Some([x, y, x + size.0, y + size.1]);
    let mut out = Vec::with_capacity(letters.len() * 2);
    for (color, (dx, dy)) in [(SHADOW, (shadow, shadow)), (COLOR, (0, 0))] {
        let mut pen = x + dx;
        for &(glyph, advance) in &letters {
            let g = OutlineGlyph { font, shape: GlyphShape::Glyph(glyph), em64: px * 64, advance, bold: false, italic: false, color, key: None };
            out.push(PlacedSprite { screen: true, view: Some(camera), clip, text: Some(g), ..PlacedSprite::new(tile_render::SpriteSetId::EMPTY, pen, baseline + dy) });
            pen += advance;
        }
    }
    Some((Camera { cells: false, ..Camera::new(window) }, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: [i32; 4], b: [i32; 4]) -> bool {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    }

    /// `count` chat lines at `scale` below corner buttons at the window's
    /// top left, as the game lists them (16 px apart at 1×, some long).
    fn chat(corner: [i32; 4], scale: (u32, u32), count: usize) -> Vec<[i32; 4]> {
        (0..count as i32).map(|i| {
            let top = corner[1] + corner[3] + scaled(6 + 16 * i, scale);
            [corner[0] + scaled(8, scale), top, scaled(if i % 3 == 0 { 520 } else { 240 }, scale), scaled(14, scale)]
        }).collect()
    }

    #[test]
    fn the_notice_goes_below_every_chat_line_and_never_over_one_at_every_hud_scale() {
        for scale in [(1, 1), (3, 2), (2, 1), (3, 1)] {
            let corner = [0, 0, scaled(149, scale), scaled(54, scale)];
            let window = (2560, 1440);
            let size = (scaled(330, scale), scaled(20, scale));
            for count in [0, 1, 3, 12] {
                let lines = chat(corner, scale, count);
                let (x, y) = place(corner, scale, &lines, size, window).unwrap();
                let notice = [x, y, size.0, size.1];
                assert!(lines.iter().all(|l| !overlaps(*l, notice)), "{scale:?} {count}: {notice:?} over a line");
                assert!(lines.iter().all(|l| y >= l[1] + l[3]), "{scale:?} {count}: below every line");
                assert!(y >= corner[1] + corner[3], "below the buttons");
                let left = lines.first().map_or(corner[0] + scaled(MARGIN, scale), |l| l[0]);
                assert_eq!(x, left, "aligned with the messages");
                // Right below the last line: no more than the gap.
                let last = lines.iter().map(|l| l[1] + l[3]).max().unwrap_or(corner[1] + corner[3]);
                assert_eq!(y, last + scaled(GAP, scale));
            }
        }
    }

    #[test]
    fn with_no_room_below_it_goes_beside_the_messages_or_nowhere_never_over_them() {
        let (corner, scale) = ([0, 0, 149, 54], (1, 1));
        // Chat down to the window's bottom.
        let lines = chat(corner, scale, 40);
        let window = (1600, 700);
        let size = (330, 20);
        let (x, y) = place(corner, scale, &lines, size, window).unwrap();
        assert!(lines.iter().all(|l| !overlaps(*l, [x, y, size.0, size.1])));
        assert!(x >= 8 + 520, "right of the widest message");
        // A window too narrow for that too: not shown at all.
        assert_eq!(place(corner, scale, &lines, size, (800, 700)), None);
    }

    #[test]
    fn what_lies_further_down_is_no_message() {
        let (corner, scale) = ([0, 0, 149, 54], (1, 1));
        let mut lines = chat(corner, scale, 2);
        // A panel line far below (the minimap's caption).
        lines.push([10, 600, 200, 14]);
        let (_, y) = place(corner, scale, &lines, (330, 20), (1600, 900)).unwrap();
        assert_eq!(y, 54 + 6 + 16 + 14 + GAP);
    }

    fn view(id: u32) -> scene::ViewState {
        scene::ViewState { id, view: scene::View { interface_only: true, viewport: (1600, 900), ..Default::default() }, ..Default::default() }
    }

    fn line(x: i32, y: i32, w: u32, h: u32) -> scene::FreeSprite {
        scene::FreeSprite { screen: true, x, y, solid: Some(scene::SolidRect { width: w, height: h, rgba: [255; 4] }), ..Default::default() }
    }

    #[test]
    fn message_lines_are_the_interfaces_and_scaled_with_its_view() {
        struct Empty;
        impl render_desc::ImageLoader for Empty {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no art".into()) }
        }
        let c = render_desc::Description::parse("[grid]\nlayout=\"orthogonal\"\nstep=[1,1]\n").unwrap().compile_layout(&Empty, 16).unwrap();
        let mut s = scene::Scene::default();
        s.views = vec![view(254), view(256), view(257)];
        s.interface.roaming = Some(256);
        s.interface.popup = Some(257);
        // The interface: a chat line, a tall panel, and one right of the
        // column; the corner's and a popup's own sprites are no messages.
        s.views[0].overlay = vec![line(8, 60, 400, 14), line(0, 300, 200, 300), line(600, 70, 100, 14)];
        s.views[1].overlay = vec![line(4, 60, 100, 14)];
        s.views[2].overlay = vec![line(8, 80, 100, 14)];
        let cameras = [Camera { zoom: (2, 1), cells: false, ..Camera::new((1600, 900)) }, Camera { cells: false, ..Camera::new((298, 108)) }, Camera { cells: false, ..Camera::new((1600, 900)) }];
        let lines = message_lines(&c, &s, &cameras, &[Some(0), Some(1), Some(2)], &[], [0, 0, 298, 108], (2, 1));
        assert_eq!(lines, vec![[16, 120, 800, 28]]);
        // The HUD in panels (a camera per sprite, moved by its screen shift):
        // the same line where that panel's camera puts it.
        let mut panels = cameras.to_vec();
        panels.push(Camera { origin: (16, 120), screen_shift: (8, 60), zoom: (2, 1), cells: false, ..Camera::new((1600, 900)) });
        let lines = message_lines(&c, &s, &panels, &[None, Some(1), Some(2)], &[(0, vec![3, 3, 3])], [0, 0, 298, 108], (2, 1));
        assert_eq!(lines, vec![[16, 120, 800, 28]]);
    }

    #[test]
    fn the_text_is_drawn_with_its_shadow_at_the_hud_scale() {
        struct Empty;
        impl render_desc::ImageLoader for Empty {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no art".into()) }
        }
        let c = render_desc::Description::parse("[grid]\nlayout=\"orthogonal\"\nstep=[1,1]\n").unwrap().compile_layout(&Empty, 16).unwrap();
        let text = render_errors::notice_text(3);
        for scale in [(1, 1), (2, 1)] {
            let (camera, sprites) = compose(&c, 3, [0, 0, scaled(149, scale), scaled(54, scale)], scale, &[], (1600, 900), 5).unwrap();
            assert!(!camera.cells);
            assert_eq!(sprites.len(), 2 * text.chars().count());
            let g = sprites[text.len()].text.unwrap();
            assert_eq!(g.em64, FONT_PX * 64 * scale.0, "{scale:?}");
            assert_eq!((g.color, sprites[0].text.unwrap().color), (COLOR, SHADOW), "the shadow first");
            assert!(sprites.iter().all(|s| s.view == Some(5) && s.screen));
        }
    }
}
