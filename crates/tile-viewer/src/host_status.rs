//! Whether the driver's host answers (`scene::HostStatus`, e.g. a game's
//! thread): after [`BUSY_AFTER`] without an answer the viewer shows a
//! small panel next to the driver's top-left buttons (or the hint that
//! takes their place while the mouse is captured; else at the top-left of
//! the view showing the world, else of the window): the busy cursor (the
//! turning hourglass) at its left, then "Game appears busy" in orange,
//! both on its parchment; after
//! [`STUCK_AFTER`] "Game appears stuck" in red. It goes the moment the host
//! answers again.
//!
//! How long the host has not answered is what the driver last said
//! (`stale_ms`) plus the time since it said it, by the viewer's clock: a
//! driver or a connection gone silent counts the same. A driver that never
//! sends a status shows nothing.

use std::time::{Duration, Instant};

use render_desc::Compiled;
use tile_render::{Camera, GlyphShape, OutlineGlyph, PlacedSprite, SolidRect};

/// Unanswered for longer than this: busy.
pub const BUSY_AFTER: Duration = Duration::from_millis(500);
/// Unanswered for longer than this: stuck.
pub const STUCK_AFTER: Duration = Duration::from_secs(5);

/// The panel's art: the parchment, its edges (a row, a column), the busy
/// cursor (the loading cursor's turning hourglass, as the busy pointer),
/// the accent line along its bottom in each state's colour.
pub const PANEL_SPRITE: &str = "Viewer@busy_panel/0";
pub const EDGE_ROW_SPRITE: &str = "Viewer@busy_edge_h/0";
pub const EDGE_COLUMN_SPRITE: &str = "Viewer@busy_edge_v/0";
pub const ICON_SPRITE: &str = scene::loading_cursor::SET;
pub const BUSY_ACCENT_SPRITE: &str = "Viewer@busy_accent/0";
pub const STUCK_ACCENT_SPRITE: &str = "Viewer@stuck_accent/0";
/// The game's font, as its "Press CTRL" hint uses it.
pub const FONT: &str = "Lucida Sans";
pub const FONT_PX: u32 = 14;

/// The text colours: orange and red, dark enough to read on the parchment.
pub const BUSY_COLOR: [u8; 3] = [160, 64, 0];
pub const STUCK_COLOR: [u8; 3] = [176, 0, 0];

/// The panel's height, its icon's box, the gap to what it sits beside.
pub const PANEL_HEIGHT: i32 = 36;
const PAD: i32 = 4;
pub const GAP: i32 = 6;
pub const MARGIN: i32 = 8;

/// How the host is doing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Responsiveness {
    Answering,
    Busy,
    Stuck,
}

impl Responsiveness {
    /// From how long the host has not answered.
    pub fn of(stale: Duration) -> Self {
        if stale > STUCK_AFTER {
            Self::Stuck
        } else if stale > BUSY_AFTER {
            Self::Busy
        } else {
            Self::Answering
        }
    }

    pub fn text(self) -> Option<&'static str> {
        match self {
            Self::Answering => None,
            Self::Busy => Some("Game appears busy"),
            Self::Stuck => Some("Game appears stuck"),
        }
    }

    pub fn color(self) -> Option<[u8; 3]> {
        match self {
            Self::Answering => None,
            Self::Busy => Some(BUSY_COLOR),
            Self::Stuck => Some(STUCK_COLOR),
        }
    }


}

/// How long the host has not answered at `now`, from its last status and
/// when that came (`scene::Display::host`); `None`: it sends none.
pub fn stale(host: Option<(scene::HostStatus, Instant)>, now: Instant) -> Option<Duration> {
    let (status, at) = host?;
    Some(Duration::from_millis(status.stale_ms.into()) + now.saturating_duration_since(at))
}

/// How the host is doing at `now` (answering if it sends no status).
pub fn judge(host: Option<(scene::HostStatus, Instant)>, now: Instant) -> Responsiveness {
    stale(host, now).map_or(Responsiveness::Answering, Responsiveness::of)
}

/// The state last shown, for the log.
#[derive(Debug)]
pub struct Indicator {
    shown: Responsiveness,
}

impl Default for Indicator {
    fn default() -> Self {
        Self { shown: Responsiveness::Answering }
    }
}

impl Indicator {
    /// Notes the state now (logs a change).
    pub fn note(&mut self, now: Responsiveness, stale: Option<Duration>) {
        if now != self.shown {
            log::info!("viewer: host {} ({} ms without an answer)", match now {
                Responsiveness::Answering => "answers again",
                Responsiveness::Busy => "appears busy",
                Responsiveness::Stuck => "appears stuck",
            }, stale.unwrap_or_default().as_millis());
            self.shown = now;
        }
    }
}

/// Where the panel goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Right of this window rectangle (x, y, w, h): the driver's top-left
    /// buttons, or the hint in their place; at its scale.
    Beside([i32; 4], (u32, u32)),
    /// At the top-left of this window rectangle: the view showing the world.
    Inside([i32; 4], (u32, u32)),
    /// At the window's top-left.
    Window,
}

/// Window rectangle (x, y, w, h) camera `i` of `cameras` draws into: a
/// stacked one inside the picture of the last unstacked one before it, at
/// its zoom; with the exact rational UI scale.
pub fn camera_rect(cameras: &[Camera], i: usize) -> Option<([i32; 4], (u32, u32))> {
    let c = cameras.get(i)?;
    let head = if c.stacked { cameras[..i].iter().rev().find(|h| !h.stacked)? } else { c };
    let (zn, zd) = (head.zoom.0.max(1) as i64, head.zoom.1.max(1) as i64);
    let rect = if c.stacked {
        let s = |v: i64| (v * zn / zd) as i32;
        [head.origin.0 + s(c.origin.0 as i64), head.origin.1 + s(c.origin.1 as i64), s(c.viewport.0 as i64), s(c.viewport.1 as i64)]
    } else {
        [c.origin.0, c.origin.1, c.viewport.0 as i32, c.viewport.1 as i32]
    };
    let rect = match c.target_clip.or(head.target_clip) {
        Some([x0, y0, x1, y1]) => {
            let (l, t) = (rect[0].max(x0), rect[1].max(y0));
            let (r, b) = ((rect[0] + rect[2]).min(x1), (rect[1] + rect[3]).min(y1));
            [l, t, r - l, b - t]
        }
        None => rect,
    };
    (rect[2] > 0 && rect[3] > 0).then_some((rect, (zn as u32, zd as u32)))
}

/// Where the panel goes in the frame the layout arranged (`cameras`, and
/// the camera of each of `scene`'s views): beside the hint for a captured
/// mouse if that shows, else beside the top-left buttons, else in the
/// first view showing the world, else at the window's corner.
pub fn anchor(compiled: &Compiled, scene: &scene::Scene, cameras: &[Camera], view_cameras: &[Option<u32>]) -> Anchor {
    let shown = |id: Option<u32>| {
        let i = scene.views.iter().position(|v| Some(v.id) == id)?;
        let view = &scene.views[i];
        let camera = (*view_cameras.get(i)?)? as usize;
        let (viewport, zoom) = camera_rect(cameras, camera)?;
        // Prefer the actual clipped drawing bounds, not the camera viewport.
        // Empty parts retain their semantic viewport until their art arrives.
        let bounds = view.overlay.iter().filter_map(|s| super::sprite_rect(compiled, s)).reduce(|a, b| {
            let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
            [x, y, (a[0] + a[2]).max(b[0] + b[2]) - x, (a[1] + a[3]).max(b[1] + b[3]) - y]
        });
        let rect = bounds.map(|[x,y,w,h]| {
            let scaled = |v: i32| (i64::from(v) * i64::from(zoom.0) / i64::from(zoom.1)) as i32;
            let c = &cameras[camera];
            let raw = if c.stacked {
                let head = cameras[..camera].iter().rev().find(|h| !h.stacked).unwrap();
                (head.origin.0 + scaled(c.origin.0), head.origin.1 + scaled(c.origin.1))
            } else { c.origin };
            let (l,t) = (raw.0 + scaled(x - view.view.origin.0), raw.1 + scaled(y - view.view.origin.1));
            let (r,b) = (l + scaled(w), t + scaled(h));
            let (l,t,r,b) = (l.max(viewport[0]),t.max(viewport[1]),r.min(viewport[0]+viewport[2]),b.min(viewport[1]+viewport[3]));
            [l,t,(r-l).max(0),(b-t).max(0)]
        }).filter(|r| r[2] > 0 && r[3] > 0).unwrap_or(viewport);
        Some((rect, zoom))
    };
    if let Some((r, n)) = shown(scene.interface.capture_hint).or_else(|| shown(scene.interface.roaming)) {
        return Anchor::Beside(r, n);
    }
    match cameras.iter().position(|c| c.cells && !c.stacked).and_then(|i| camera_rect(cameras, i)) {
        Some((r, n)) => Anchor::Inside(r, n),
        None => Anchor::Window,
    }
}

/// The panel's top-left corner for a panel of `size` (window pixels) in a
/// window of `window`, kept inside it.
pub fn place(anchor: Anchor, size: (i32, i32), window: (u32, u32)) -> (i32, i32) {
    let (x, y) = match anchor {
        Anchor::Beside([x, y, w, h], _) => (x + w + GAP, y + ((h - size.1) / 2).max(0)),
        Anchor::Inside([x, y, _, _], n) => (x + MARGIN * n.0 as i32 / n.1.max(1) as i32, y + MARGIN * n.0 as i32 / n.1.max(1) as i32),
        Anchor::Window => (MARGIN, MARGIN),
    };
    let (ww, wh) = (window.0 as i32, window.1 as i32);
    if let Anchor::Beside([bx, by, bw, bh], _) = anchor {
        if x + size.0 > ww {
            // Clamping left would cover the buttons. Use the free row below.
            return (bx.min((ww-size.0).max(0)).max(0), (by+bh+GAP).max(0));
        }
        debug_assert!(x >= bx + bw);
    }
    (x.min(ww - size.0).max(0), y.min(wh - size.1).max(0))
}

/// The scale the panel is drawn at.
pub fn scale(anchor: Anchor) -> (u32, u32) {
    match anchor {
        Anchor::Beside(_, n) | Anchor::Inside(_, n) => n,
        Anchor::Window => (1, 1),
    }
}

/// Require a complete GPU glyph command group before showing any panel.
/// Atlas images are optional decorations and never gate readable text.
pub(crate) fn face_for<'a>(compiled: &'a Compiled, text: &str) -> Option<(u32, &'a tile_render::gpu_text::Face)> {
    let usable = |face: &tile_render::gpu_text::Face| text.chars().all(|ch| face.glyph(ch).is_some() && face.advance(ch).is_some());
    compiled.font(FONT).filter(|(_, f)| usable(f)).or_else(|| compiled.fonts.iter().enumerate()
        .find(|(_, (_, f))| usable(f)).map(|(i, (_, face))| (i as u32, &**face)))
}

fn regions(anchor: Anchor, window: (u32, u32), gap: i32) -> Vec<[i32; 4]> {
    let (ww, wh) = (window.0.min(i32::MAX as u32) as i32, window.1.min(i32::MAX as u32) as i32);
    let clip = |[x,y,w,h]: [i32;4]| {
        let (l,t,r,b)=(x.max(0),y.max(0),(x+w).min(ww),(y+h).min(wh));
        [l,t,(r-l).max(0),(b-t).max(0)]
    };
    match anchor {
        Anchor::Beside([x,y,w,h], _) => vec![clip([x+w+gap,y,ww-(x+w+gap),wh-y]),clip([0,y+h+gap,ww,wh-(y+h+gap)])],
        Anchor::Inside([x,y,w,h], _) => vec![clip([x+MARGIN,y+MARGIN,w-2*MARGIN,h-2*MARGIN])],
        Anchor::Window => vec![clip([MARGIN,MARGIN,ww-2*MARGIN,wh-2*MARGIN])],
    }
}

/// The panel is a small native GPU command group with physical coordinates.
/// Its own 1:1 camera avoids mixing scaled camera and physical clip origins.
/// Right of the controls is preferred; a narrow window gets a row below.
/// If no readable group fits, it remains absent rather than covering controls.
pub fn compose(compiled: &Compiled, state: Responsiveness, anchor: Anchor, window: (u32, u32), _seconds: f64, camera: u32) -> Option<(Camera, Vec<PlacedSprite>)> {
    let (text, color) = (state.text()?, state.color()?);
    let (font, face) = face_for(compiled, text)?;
    let (zn, zd) = scale(anchor);
    let requested = (f64::from(zn) / f64::from(zd.max(1))).max(1.0);
    // Keep current UI scale whenever the group fits. Only exceptionally
    // small windows reduce the group to its readable 14px minimum.
    let scales = [requested, 1.0];
    // The turning hourglass, if its frames are uploaded: (set, frame size,
    // anchor). Decoration only: without room for it the text stays alone.
    let icon = compiled.set(ICON_SPRITE).filter(|_| compiled.set_available(ICON_SPRITE)).and_then(|(set, _)| {
        let art = compiled.atlas.sprites.get(compiled.sets.frame(set, 0)?.0 as usize).filter(|a| a.page != u32::MAX)?;
        Some((set, (art.width as i32, art.height as i32), (art.anchor_x, art.anchor_y)))
    });
    let slot = scene::loading_cursor::ENVELOPE as i32;
    for factor in scales {
    for icon in [icon, None].into_iter().take(if icon.is_some() { 2 } else { 1 }) {
        let px = (f64::from(FONT_PX) * factor).round().clamp(14.0, 1024.0) as u32;
        let pad = (f64::from(PAD) * factor).round().max(2.0) as i32;
        let gap = (f64::from(GAP) * factor).round().max(1.0) as i32;
        let em = i64::from(face.units_per_em().max(1));
        let letters: Vec<_> = text.chars().map(|ch| (ch,face.glyph(ch).unwrap(),
            ((i64::from(face.advance(ch).unwrap()) * i64::from(px) + em/2)/em) as i32 + 1)).collect();
        let min_word_width = letters.split(|(ch,_,_)| *ch == ' ').map(|word| word.iter().map(|(_,_,advance)| *advance).sum::<i32>()).max().unwrap_or(1);
        for region in regions(anchor, window, gap) {
            let [x,y,available_w,available_h] = region;
            // The hourglass's slot and a pad before the text.
            let lead = if icon.is_some() { slot + pad } else { 0 };
            let text_room = available_w - 2*pad - lead;
            if text_room < min_word_width { continue; }
            let mut lines: Vec<Vec<(u32,i32)>> = vec![Vec::new()];
            let mut widths = vec![0];
            for (at, &(ch, glyph, advance)) in letters.iter().enumerate() {
                if ch != ' ' && at > 0 && letters[at-1].0 == ' ' {
                    let word_width: i32 = letters[at..].iter().take_while(|(ch,_,_)| *ch != ' ').map(|(_,_,advance)| *advance).sum();
                    let i=lines.len()-1;
                    if word_width <= text_room && widths[i]+word_width > text_room && !lines[i].is_empty() {
                        lines.push(Vec::new());widths.push(0);
                    }
                }
                let i = lines.len()-1;
                if widths[i]+advance > text_room && !lines[i].is_empty() { lines.push(Vec::new()); widths.push(0); }
                let i=lines.len()-1;
                lines[i].push((glyph,advance)); widths[i]+=advance;
            }
            let line_h=px as i32+pad;
            // (Room for the hourglass inside the frame and above the accent.)
            let h=(lines.len() as i32*line_h+2*pad).max(PANEL_HEIGHT).max(if icon.is_some() { slot+4 } else { 0 });
            let text_w=widths.iter().copied().max().unwrap_or(0);
            let w=text_w+2*pad+lead;
            if h > available_h { continue; }
            let clip=Some([x,y,x+w,y+h]);
            let solid=|[sx,sy,sw,sh]:[i32;4],rgba| PlacedSprite { screen:true,view:Some(camera),clip,
                solid:Some(SolidRect {width:sw as u32,height:sh as u32,rgba}),
                ..PlacedSprite::new(tile_render::SpriteSetId::EMPTY,sx,sy) };
            let mut out=vec![solid([x,y,w,h],[245,233,198,255]),solid([x,y,w,1],[72,52,24,255]),
                solid([x,y+h-1,w,1],[72,52,24,255]),solid([x,y,1,h],[72,52,24,255]),solid([x+w-1,y,1,h],[72,52,24,255]),
                solid([x+1,y+h-3,w-2,2],[color[0],color[1],color[2],255])];
            // First the hourglass, at the left inside the panel, turning
            // about its slot's middle by the viewer's clock.
            if let Some((set,(fw,fh),(ax,ay)))=icon {
                // (Centred between the top edge and the accent line.)
                let (ix,iy)=(x+pad,y+1+(h-4-slot)/2);
                out.push(PlacedSprite {screen:true,view:Some(camera),phase:0,ticks_per_frame:scene::loading_cursor::TICKS_PER_FRAME,
                    clip:Some([ix,iy.max(y+1),ix+slot,(iy+slot).min(y+h-3)]),
                    ..PlacedSprite::new(set,ix-(fw-slot)/2-ax,iy-(fh-slot)/2-ay)});
            }
            // Then the text.
            for (row,line) in lines.iter().enumerate() {
                let mut pen=x+pad+lead;
                let baseline=y+pad+px as i32+row as i32*line_h;
                for &(glyph,advance) in line {
                    let g=OutlineGlyph {font,shape:GlyphShape::Glyph(glyph),em64:px*64,advance,bold:true,italic:false,color,key:None};
                    out.push(PlacedSprite {screen:true,view:Some(camera),clip,text:Some(g),
                        ..PlacedSprite::new(tile_render::SpriteSetId::EMPTY,pen,baseline)});
                    pen+=advance;
                }
            }
            return Some((Camera {cells:false,..Camera::new(window)},out));
        }
    }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Compiled {
        struct Empty;
        impl render_desc::ImageLoader for Empty {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no art".into()) }
        }
        render_desc::Description::parse("[grid]\nlayout=\"orthogonal\"\nstep=[1,1]\n").unwrap().compile_layout(&Empty,16).unwrap()
    }

    fn status(stale_ms: u32) -> scene::HostStatus {
        scene::HostStatus { stale_ms, serial: 1 }
    }

    #[test]
    fn answering_then_busy_after_half_a_second_then_stuck_after_five_then_hidden_again() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // The game's thread stops answering at t0; statuses keep coming.
        let host = |sent: u64| Some((status(sent as u32), at(sent)));
        assert_eq!(judge(host(0), at(0)), Responsiveness::Answering);
        assert_eq!(judge(host(400), at(450)), Responsiveness::Answering);
        assert_eq!(judge(host(500), at(500)), Responsiveness::Answering, "500 ms is not yet more than 500 ms");
        assert_eq!(judge(host(500), at(501)), Responsiveness::Busy);
        assert_eq!(judge(host(2_900), at(3_000)), Responsiveness::Busy);
        assert_eq!(judge(host(3_000), at(3_001)), Responsiveness::Busy);
        assert_eq!(STUCK_AFTER, Duration::from_secs(5));
        assert_eq!(judge(host(4_900), at(5_000)), Responsiveness::Busy);
        assert_eq!(judge(host(5_000), at(5_001)), Responsiveness::Stuck);
        assert_eq!(judge(host(9_000), at(9_050)), Responsiveness::Stuck);
        // It answers again: hidden at once.
        assert_eq!(judge(Some((status(0), at(9_100))), at(9_100)), Responsiveness::Answering);
        // The texts and colours.
        assert_eq!((Responsiveness::Answering.text(), Responsiveness::Answering.color()), (None, None));
        assert_eq!((Responsiveness::Busy.text(), Responsiveness::Busy.color()), (Some("Game appears busy"), Some(BUSY_COLOR)));
        assert_eq!((Responsiveness::Stuck.text(), Responsiveness::Stuck.color()), (Some("Game appears stuck"), Some(STUCK_COLOR)));
    }

    #[test]
    fn waiting_in_the_message_loop_counts_as_answering() {
        // An idle game asleep in its message wait reports 0, however long
        // ago its last message was (`civ3_bridge::heartbeat`).
        let t0 = Instant::now();
        for since_ms in [0u64, 50, 99] {
            assert_eq!(judge(Some((status(0), t0)), t0 + Duration::from_millis(since_ms)), Responsiveness::Answering);
        }
    }

    #[test]
    fn a_silent_connection_counts_as_not_answering() {
        let t0 = Instant::now();
        let last = Some((status(0), t0));
        assert_eq!(judge(last, t0 + Duration::from_millis(400)), Responsiveness::Answering);
        assert_eq!(judge(last, t0 + Duration::from_millis(600)), Responsiveness::Busy);
        assert_eq!(judge(last, t0 + Duration::from_millis(3_100)), Responsiveness::Busy);
        assert_eq!(judge(last, t0 + Duration::from_millis(5_100)), Responsiveness::Stuck);
        // Silence on top of what was said.
        assert_eq!(judge(Some((status(4_800), t0)), t0 + Duration::from_millis(300)), Responsiveness::Stuck);
        // A driver that sends no status: nothing shows.
        assert_eq!(judge(None, t0 + Duration::from_secs(60)), Responsiveness::Answering);
    }

    /// WCAG contrast of two sRGB colours.
    fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
        let lum = |c: [u8; 3]| {
            let l = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * l(c[0]) + 0.7152 * l(c[1]) + 0.0722 * l(c[2])
        };
        let (x, y) = (lum(a) + 0.05, lum(b) + 0.05);
        x.max(y) / x.min(y)
    }

    #[test]
    fn busy_is_orange_and_stuck_red_both_readable_on_the_parchment() {
        // Art/PopUpandMenuBackground.pcx's top-left 240x40: its mean colour
        // and its darkest pixel.
        let (mean, darkest) = ([245, 233, 198], [189, 189, 156]);
        for c in [BUSY_COLOR, STUCK_COLOR] {
            assert!(contrast(c, mean) >= 4.5, "{c:?} on {mean:?}: {}", contrast(c, mean));
            assert!(contrast(c, darkest) >= 3.0, "{c:?} on {darkest:?}: {}", contrast(c, darkest));
        }
        // Orange: red over green over blue, green well present; red: red alone.
        let [r, g, b] = BUSY_COLOR;
        assert!(r > g && g > b && g as u32 * 4 >= r as u32);
        let [r, g, b] = STUCK_COLOR;
        assert!(r >= 128 && g == 0 && b == 0);
        assert_ne!(BUSY_COLOR, STUCK_COLOR);
    }

    fn view(id: u32) -> scene::ViewState {
        scene::ViewState { id, view: scene::View { interface_only: true, ..Default::default() }, ..Default::default() }
    }

    fn cam(origin: (i32, i32), viewport: (u32, u32)) -> Camera {
        Camera { origin, cells: false, ..Camera::new(viewport) }
    }

    fn with_font() -> Compiled {
        let mut c=empty();
        c.fonts.clear();
        let chars: std::collections::BTreeSet<_>="Game appears busyGame appears stuck".chars().collect();
        let glyphs:Vec<_>=chars.into_iter().map(|ch|(ch,Vec::new(),600)).collect();
        c.fonts.push((FONT.into(),std::sync::Arc::new(tile_render::gpu_text::Face::from_curves(1000,&glyphs))));
        c
    }
    fn group_bounds(sprites:&[PlacedSprite])->[i32;4] {
        let panel=sprites[0].solid.unwrap();
        [sprites[0].x,sprites[0].y,panel.width as i32,panel.height as i32]
    }
    fn overlaps(a:[i32;4],b:[i32;4])->bool { a[0]<b[0]+b[2] && b[0]<a[0]+a[2] && a[1]<b[1]+b[3] && b[1]<a[1]+a[3] }
    #[test]
    fn fonts_and_complete_glyph_commands_are_atomic_but_missing_atlas_art_is_optional() {
        let anchor=Anchor::Beside([0,0,149,54],(1,1));
        let mut c=empty();
        let (_,fallback)=compose(&c,Responsiveness::Busy,anchor,(800,600),0.0,3).unwrap();
        assert_eq!(fallback.iter().filter(|s|s.text.is_some()).count(),"Game appears busy".len());
        assert!(c.font(render_desc::FALLBACK_FONT).is_some());
        c.fonts=with_font().fonts;
        let (_,commands)=compose(&c,Responsiveness::Busy,anchor,(800,600),0.0,3).unwrap();
        assert_eq!(commands.iter().filter(|s|s.text.is_some()).count(),"Game appears busy".len());
        assert!(commands.iter().all(|s|s.view==Some(3) && s.screen));
        assert!(commands.iter().all(|s|s.text.is_some() || s.solid.is_some()),"no atlas art needed");
        c.fonts[0].1=std::sync::Arc::new(tile_render::gpu_text::Face::from_curves(1000,&[('G',Vec::new(),600)]));
        c.fonts.extend(empty().fonts);
        let (_,fallback)=compose(&c,Responsiveness::Busy,anchor,(800,600),0.0,3).unwrap();
        assert_eq!(fallback.iter().filter(|s|s.text.is_some()).count(),"Game appears busy".len(),"missing game glyphs use the built-in outline face");
    }
    #[test]
    fn actual_bounds_fractional_scale_narrow_window_and_maximum_scale_never_overlap_controls() {
        let c=with_font();
        for (window,controls,zoom) in [((1600,900),[0,0,149,54],(1,1)),((1600,900),[800,0,298,108],(2,1)),
            ((900,600),[0,0,224,81],(3,2)),((300,200),[0,0,149,54],(1,1)),((240,200),[0,0,224,81],(3,2)),
            ((640,480),[0,0,596,216],(4,1)),((640,480),[0,0,596,216],(32,1))] {
            let (_,commands)=compose(&c,Responsiveness::Stuck,Anchor::Beside(controls,zoom),window,0.0,0).unwrap();
            let bounds=group_bounds(&commands);
            assert!(!overlaps(bounds,controls),"{window:?} {zoom:?} {bounds:?}");
            assert!(bounds[0]>=0 && bounds[1]>=0 && bounds[0]+bounds[2]<=window.0 as i32 && bounds[1]+bounds[3]<=window.1 as i32);
            assert_eq!(commands.iter().filter(|s|s.text.is_some()).count(),"Game appears stuck".len());
        }
        assert!(compose(&c,Responsiveness::Busy,Anchor::Beside([0,0,100,100],(1,1)),(100,100),0.0,0).is_none());
    }
    /// The test font and an uploaded (solid) hourglass frame in the loading
    /// cursor's set.
    fn with_font_and_hourglass() -> Compiled {
        struct Empty;
        impl render_desc::ImageLoader for Empty {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no art".into()) }
        }
        let mut c=render_desc::Description::parse(r#"
            grid = { layout = "orthogonal", step = [1, 1] }
            [images.hourglass]
            source = "solid:20,20,20,255:46x46"
            [sheets.hourglass]
            image = "hourglass"
            cell = [46, 46]
            anchor = [-23, -23]
            group = "Viewer@loading_cursor_frame"
            [sets."Viewer@loading_cursor/0"]
            frames = ["Viewer@loading_cursor_frame/0"]
            "#).unwrap().compile(&Empty,256).unwrap();
        c.fonts=with_font().fonts;
        assert!(c.set_available(ICON_SPRITE));
        c
    }
    #[test]
    fn the_hourglass_comes_first_then_the_text_both_inside_the_panel() {
        let c=with_font_and_hourglass();
        let hourglass=c.set(ICON_SPRITE).unwrap().0;
        for (state,text) in [(Responsiveness::Busy,"Game appears busy"),(Responsiveness::Stuck,"Game appears stuck")] {
            for (window,controls,zoom) in [((1600,900),[0,0,149,54],(1,1)),((1600,900),[800,0,298,108],(2,1)),((900,600),[0,0,224,81],(3,2))] {
                let (_,commands)=compose(&c,state,Anchor::Beside(controls,zoom),window,0.0,0).unwrap();
                let [px,py,pw,ph]=group_bounds(&commands);
                let icons:Vec<_>=commands.iter().enumerate().filter(|(_,s)|s.text.is_none() && s.solid.is_none()).collect();
                assert_eq!(icons.len(),1,"one hourglass");
                let (at,icon)=icons[0];
                assert_eq!(icon.set,hourglass);
                assert_eq!(icon.ticks_per_frame,scene::loading_cursor::TICKS_PER_FRAME,"turning by the viewer's clock");
                let [l,t,r,b]=icon.clip.unwrap();
                // Inside the parchment: off its frame, above the accent.
                assert!(l>px && t>py && r<px+pw && b<=py+ph-3,"{state:?} {zoom:?}: icon {:?} in panel {:?}",[l,t,r,b],[px,py,pw,ph]);
                assert_eq!((r-l,b-t),(32,32),"the whole turning hourglass shows");
                assert_eq!((icon.x,icon.y),(l+16,t+16),"it turns about its slot's middle");
                // Then the text: every glyph right of the icon, inside too.
                let glyphs:Vec<_>=commands.iter().enumerate().filter(|(_,s)|s.text.is_some()).collect();
                assert_eq!(glyphs.len(),text.len());
                for (i,g) in glyphs {
                    assert!(i>at,"drawn after the icon");
                    let advance=g.text.as_ref().unwrap().advance;
                    assert!(g.x>=r,"{state:?} {zoom:?}: glyph at {} left of the icon's right {r}",g.x);
                    assert!(g.x+advance<=px+pw && g.y>py && g.y<py+ph,"{state:?} {zoom:?}: glyph ({},{}) outside {:?}",g.x,g.y,[px,py,pw,ph]);
                }
                let first=commands.iter().filter(|s|s.text.is_some()).map(|s|s.x).min().unwrap();
                assert!(l<first,"icon first, then text");
            }
        }
        // Too narrow for both: the text alone, never the icon instead.
        let (_,commands)=compose(&c,Responsiveness::Stuck,Anchor::Beside([0,0,149,54],(1,1)),(300,200),0.0,0).unwrap();
        assert_eq!(commands.iter().filter(|s|s.text.is_some()).count(),"Game appears stuck".len());
    }
    #[test]
    fn anchor_uses_real_drawn_command_bounds_at_current_ui_scale() {
        let c=empty();
        let mut s=scene::Scene::default();
        s.views=vec![view(256)];s.interface.roaming=Some(256);
        s.views[0].view.viewport=(500,300);
        s.views[0].overlay=vec![scene::FreeSprite {screen:true,x:4,y:6,solid:Some(scene::SolidRect {width:149,height:54,rgba:[0;4]}),..Default::default()}];
        let camera=Camera {zoom:(3,2),..cam((0,0),(500,300))};
        assert_eq!(anchor(&c,&s,&[camera],&[Some(0)]),Anchor::Beside([6,9,223,81],(3,2)));
    }

    #[test]
    fn the_panel_sits_beside_the_corner_buttons() {
        let mut s = scene::Scene::default();
        s.views = vec![view(255), view(256), view(259)];
        s.interface.roaming = Some(256);
        s.interface.capture_hint = Some(259);
        let world = Camera { cells: true, ..Camera::new((1600, 900)) };
        // The buttons drawn (the hint not, the mouse free).
        let cameras = [world, cam((0, 0), (1600, 900)), cam((0, 0), (149, 54))];
        let a = anchor(&empty(), &s, &cameras, &[Some(1), Some(2), None]);
        assert_eq!(a, Anchor::Beside([0, 0, 149, 54], (1, 1)));
        assert_eq!(place(a, (200, PANEL_HEIGHT), (1600, 900)), (149 + GAP, (54 - PANEL_HEIGHT) / 2));
        // In the right pane of a split, scaled twice: beside them there.
        let pane = Camera { zoom: (2, 1), ..cam((800, 0), (800, 900)) };
        let buttons = Camera { stacked: true, ..cam((0, 0), (149, 54)) };
        let a = anchor(&empty(), &s, &[world, pane, buttons], &[None, Some(2), None]);
        assert_eq!(a, Anchor::Beside([800, 0, 298, 108], (2, 1)));
        assert_eq!(scale(a), (2, 1));
        assert_eq!(place(a, (400, 2 * PANEL_HEIGHT), (1600, 900)).0, 800 + 298 + GAP);
    }

    #[test]
    fn the_panel_sits_beside_the_capture_hint_in_their_place() {
        let mut s = scene::Scene::default();
        s.views = vec![view(256), view(259)];
        s.interface.roaming = Some(256);
        s.interface.capture_hint = Some(259);
        let world = Camera { cells: true, ..Camera::new((1600, 900)) };
        let hint = cam((808, 8), (133, 38));
        let a = anchor(&empty(), &s, &[world, hint], &[None, Some(1)]);
        assert_eq!(a, Anchor::Beside([808, 8, 133, 38], (1, 1)));
        let (x, y) = place(a, (200, PANEL_HEIGHT), (1600, 900));
        assert_eq!((x, y), (808 + 133 + GAP, 9));
    }

    #[test]
    fn with_a_screen_hiding_the_buttons_the_panel_is_at_the_worlds_top_left_else_the_windows() {
        let mut s = scene::Scene::default();
        s.views = vec![view(258)];
        s.interface.screen_view = Some(258);
        // The city screen at the left, its world beside it at the right.
        let screen = cam((0, 0), (1024, 768));
        let world = Camera { cells: true, origin: (1024, 0), ..Camera::new((576, 768)) };
        let a = anchor(&empty(), &s, &[screen, world], &[Some(0)]);
        assert_eq!(a, Anchor::Inside([1024, 0, 576, 768], (1, 1)));
        assert_eq!(place(a, (200, PANEL_HEIGHT), (1600, 768)), (1024 + MARGIN, MARGIN));
        // A screen over everything: the window's corner.
        let a = anchor(&empty(), &s, &[screen], &[Some(0)]);
        assert_eq!(a, Anchor::Window);
        assert_eq!(place(a, (200, PANEL_HEIGHT), (1024, 768)), (MARGIN, MARGIN));
        // Kept inside a narrow window.
        assert_eq!(place(Anchor::Beside([0, 0, 149, 54], (1, 1)), (200, PANEL_HEIGHT), (300, 200)), (0, 54 + GAP));
    }
}
