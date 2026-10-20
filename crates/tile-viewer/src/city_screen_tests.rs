//! Named screens composed as the viewer draws them, from scenes shaped as
//! the bridge publishes them: laid out (`Layout::frame`), arranged
//! (`scaling::arrange_panned`) and rendered on the CPU
//! (`tile_render::reference`).
//!
//! The city screen: the game's view interface-only (filling the window,
//! nothing of its own), the city's surroundings a world view of their own
//! in the screen's transparent band (`civ3_bridge::scene_map::city_view`,
//! its camera centring the city as the screen intends:
//! `civ3_bridge::city_screen::camera`), the screen's interface a menu view
//! with what the game drew over its map moved onto that camera
//! (`civ3_bridge::city_screen::Anchor`). An advisor: a screen part.

use super::*;
use crate::{interface_rects, named_screen_content, scaling, scene_world_arranged, sprite_rect, world_bounds};
use render_desc::Compiled;
use scene::{FreeSprite, View, ViewState};
use tile_render::RgbaImage;
use viewer_settings::{Layout as Settings, Split};

const GROUND: [u8; 4] = [20, 160, 40, 255];
const PANEL: [u8; 4] = [200, 30, 40, 255];
const MARK: [u8; 4] = [250, 250, 20, 255];
const OUTLINE: [u8; 4] = [255, 255, 255, 255];
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];
const SHADE: [u8; 4] = [90, 10, 90, 255];

struct NoFiles;
impl render_desc::ImageLoader for NoFiles {
    fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> { panic!("generated art only") }
}

fn compiled() -> Compiled {
    render_desc::Description::parse(r#"
        [grid]
        layout = "staggered"
        step = [64, 32]
        [images.ground]
        source = "solid:20,160,40,255:128x64"
        [images.panel]
        source = "solid:200,30,40,255:64x64"
        [images.hud]
        source = "solid:30,50,220,255:64x64"
        [images.mark]
        source = "solid:250,250,20,255:16x16"
        [images.outline]
        source = "solid:255,255,255,255:6x6"
        [sheets.ground]
        image = "ground"
        cell = [128, 64]
        group = "Ground"
        [sheets.panel]
        image = "panel"
        cell = [64, 64]
        group = "Panel"
        [sheets.hud]
        image = "hud"
        cell = [64, 64]
        group = "Hud"
        [sheets.mark]
        image = "mark"
        cell = [16, 16]
        group = "Mark"
        [sheets.outline]
        image = "outline"
        cell = [6, 6]
        group = "Outline"
        [images.shade]
        source = "solid:90,10,90,255:128x64"
        [sheets.shade]
        image = "shade"
        cell = [128, 64]
        group = "Shade"
        [[layers]]
        name = "ground"
        sets = ["Ground/*"]
        [[layers]]
        name = "shade"
        sets = ["Shade/*"]
    "#).unwrap().compile(&NoFiles, 1024).unwrap()
}

/// A screen sprite of set `name` at (x, y).
fn screen_sprite(compiled: &Compiled, name: &str, x: i32, y: i32) -> FreeSprite {
    FreeSprite { set: compiled.set(name).unwrap().0.raw(), x, y, screen: true, after_pass: u32::MAX, ticks_per_frame: 1, ..Default::default() }
}

/// Panel sprites covering `r` (x, y, w, h; clipped to it).
fn panel(compiled: &Compiled, [x, y, w, h]: [i32; 4]) -> Vec<FreeSprite> {
    let mut out = Vec::new();
    for py in (y..y + h).step_by(64) {
        for px in (x..x + w).step_by(64) {
            out.push(FreeSprite { clip: Some([x, y, x + w, y + h]), ..screen_sprite(compiled, "Panel/0", px, py) });
        }
    }
    out
}

/// The game's window.
const SCREEN: (u32, u32) = (1920, 1080);
/// The city screen's background at 1920×1080 and its transparent band.
const CITY_PANEL: [i32; 4] = [448, 156, 1024, 768];
const CITY_MAP: [i32; 4] = [448, 306, 1024, 320];
/// Where the city screen puts its city tile's centre on that window.
const CITY_CENTRE: (i32, i32) = (960, 476);
/// The world: 80 × 60 tiles (40 columns), wrapping east-west.
const COLS: u32 = 40;
const ROWS: u32 = 60;

fn world(compiled: &Compiled) -> Scene {
    let shape = scene::WorldShape { cols: COLS, rows: ROWS, wrap_x: true, wrap_y: false };
    let mut scene = Scene::empty_with(shape, compiled.layers.iter().map(|l| (l.per_cell_phase, l.per_cell_offset)));
    let ground = compiled.value("ground", "Ground/0").unwrap();
    for row in 0..ROWS {
        for col in 0..COLS {
            scene.set(0, col, row, ground);
        }
    }
    scene
}

/// `civ3_bridge::city_screen::camera`: city tile `c`'s centre at
/// [`CITY_CENTRE`].
fn city_camera(c: (i32, i32)) -> (i32, i32) {
    (64 * c.0 + 64 - CITY_CENTRE.0, 32 * c.1 + 32 - CITY_CENTRE.1)
}

/// The bridge's scene with the city screen of the city at tile `city`
/// open. The game's view (hidden) at `game_downscale` (the game may have
/// been zoomed out) with the game's camera `game_camera` (in its pixels;
/// e.g. clamped at the map's edge). The city's map through our camera; the
/// interface in a menu view, the game's mark on the city tile (its work
/// area's centre) moved onto our camera. The city's world mark: a world
/// sprite on the tile's centre.
fn city_scene(compiled: &Compiled, city: (i32, i32), game_downscale: u32, game_camera: (i32, i32)) -> Scene {
    let mut s = world(compiled);
    let ours = city_camera(city);
    let [x, y, w, h] = CITY_PANEL;
    let mut ui = panel(compiled, [x, y, w, CITY_MAP[1] - y]);
    ui.extend(panel(compiled, [x, CITY_MAP[1] + CITY_MAP[3], w, y + h - CITY_MAP[1] - CITY_MAP[3]]));
    ui.push(screen_sprite(compiled, "Outline/0", 64 * city.0 + 64 - ours.0 - 3, 32 * city.1 + 32 - ours.1 - 3));
    s.views = vec![
        ViewState { id: 0, view: View { camera_x: game_camera.0, camera_y: game_camera.1, zoom: (1, 1), interface_only: true, downscale: game_downscale, ..Default::default() }, overlay: vec![] },
        ViewState { id: 1, view: View { camera_x: ours.0 + CITY_MAP[0], camera_y: ours.1 + CITY_MAP[1], zoom: (1, 1), viewport: (CITY_MAP[2] as u32, CITY_MAP[3] as u32), origin: (CITY_MAP[0], CITY_MAP[1]), downscale: 1, ..Default::default() }, overlay: vec![] },
        ViewState { id: 254, view: View { interface_only: true, ..Default::default() }, overlay: vec![] },
        ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay: ui },
    ];
    s.sprites.push(FreeSprite { screen: false, ..screen_sprite(compiled, "Mark/0", 64 * city.0 + 56, 32 * city.1 + 24) });
    s.interface.screen = Some("City".into());
    s
}

/// An advisor: a 1024×768 screen part over the map's HUD.
fn advisor_scene(compiled: &Compiled, name: &str) -> Scene {
    let mut s = world(compiled);
    s.views = vec![
        ViewState { id: 0, view: View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() }, overlay: vec![] },
        ViewState { id: 254, view: View { interface_only: true, ..Default::default() }, overlay: vec![screen_sprite(compiled, "Hud/0", 0, 0), screen_sprite(compiled, "Hud/0", 64, 0)] },
        ViewState { id: 258, view: View { interface_only: true, origin: (CITY_PANEL[0], CITY_PANEL[1]), viewport: (CITY_PANEL[2] as u32, CITY_PANEL[3] as u32), ..Default::default() }, overlay: panel(compiled, CITY_PANEL) },
    ];
    s.interface.screen = Some(name.into());
    s.interface.screen_view = Some(258);
    s
}

/// The plain map before the screen opens.
fn map_scene(compiled: &Compiled) -> Scene {
    let mut s = world(compiled);
    s.views = vec![ViewState { id: 0, view: View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() }, overlay: vec![] }];
    s
}

struct Composed {
    layout: Layout,
    frame: Frame,
    image: RgbaImage,
}

/// `scene` as a viewer arranged by `settings` shows it on a window as large
/// as the game's (the mouse captured first if `capture`), after the map.
fn compose(compiled: &Compiled, settings: &Settings, scene: &Scene, capture: bool) -> Composed {
    let plan = scaling::plan(SCREEN, 1.0, SCREEN, &viewer_settings::Resolution::default());
    let mut l = Layout::default();
    // (Capturing: B follows the captured mouse.)
    l.apply_settings(&Settings { roles: if capture { viewer_settings::Roles::UnitMouse } else { settings.roles }, ..settings.clone() });
    l.grid_step = compiled.step;
    l.driver_screen = Some(plan.screen);
    l.menu_scale = plan.menu.zoom;
    let bounds = Some(world_bounds(compiled, scene));
    let map = map_scene(compiled);
    l.frame(&map, plan.canvas(), (0, 0, 0, 0), vec![], bounds);
    if let Some([x, y, w, h]) = l.panes().1.filter(|_| capture) {
        assert!(l.capture_at((x + w / 2, y + h / 2)));
        assert!(l.captured());
        l.frame(&map, plan.canvas(), (0, 0, 0, 0), vec![], bounds);
    }
    let draw = |l: &mut Layout| {
        l.screen_content = named_screen_content(compiled, scene);
        let mut frame = l.frame(scene, plan.canvas(), (0, 0, 0, 0), interface_rects(compiled, scene), bounds);
        let arranged = scaling::arrange_panned(&plan, scene, &mut frame, &mut l.menu_pan, |s| sprite_rect(compiled, s));
        l.set_interface_shown(arranged.shown.clone());
        let world = scene_world_arranged(compiled, scene, &frame.view_cameras, &arranged.overlay_cameras);
        let image = tile_render::reference::render_views(&world, &compiled.atlas, &frame.cameras, plan.canvas(), 0, BACKGROUND);
        (frame, image)
    };
    draw(&mut l);
    let (frame, image) = draw(&mut l);
    Composed { layout: l, frame, image }
}

/// The pixels of `color` in `r` (x, y, w, h): how many, and their centroid.
fn pixels(image: &RgbaImage, r: [i32; 4], color: [u8; 4]) -> (usize, (f64, f64)) {
    let (mut n, mut sx, mut sy) = (0, 0.0, 0.0);
    for y in r[1].max(0)..(r[1] + r[3]).min(image.height as i32) {
        for x in r[0].max(0)..(r[0] + r[2]).min(image.width as i32) {
            if image.get(x as u32, y as u32) == color {
                (n, sx, sy) = (n + 1, sx + x as f64, sy + y as f64);
            }
        }
    }
    (n, if n > 0 { (sx / n as f64, sy / n as f64) } else { (0.0, 0.0) })
}

const WHOLE: [i32; 4] = [0, 0, SCREEN.0 as i32, SCREEN.1 as i32];

/// Where the settings put a named screen: A (`Hit::Main`), B or floating.
fn expected(split: Split, place: Place) -> Option<Hit> {
    match (split, place) {
        (Split::Standard, _) | (_, Place::Floating) => None,
        (Split::Horizontal1To1, Place::LeftOrBottom) | (Split::Vertical1To1 | Split::Vertical2To1, Place::RightOrTop) => Some(Hit::Pane),
        _ => Some(Hit::Main),
    }
}

/// The city's work area as the game marks it lies on the city's tile in the
/// world shown: the overlay's centre is the world mark's.
fn assert_overlay_on_city(c: &Composed, what: &str) -> (f64, f64) {
    let (n, o) = pixels(&c.image, WHOLE, OUTLINE);
    assert!(n > 0, "{what}: the city screen's overlay is drawn");
    let around = [o.0 as i32 - 24, o.1 as i32 - 24, 48, 48];
    let (m, w) = pixels(&c.image, around, MARK);
    assert!(m > 0, "{what}: the city's tile is drawn under its overlay");
    assert!((w.0 - o.0).abs() <= 1.0 && (w.1 - o.1).abs() <= 1.0, "{what}: overlay centre {o:?}, city tile centre {w:?}");
    o
}

#[test]
fn named_screens_are_drawn_whole_in_their_pane_and_the_other_pane_keeps_its_world() {
    let compiled = compiled();
    let city = city_scene(&compiled, (21, 31), 1, city_camera((21, 31)));
    for (name, scene) in [("City", city), ("F1", advisor_scene(&compiled, "F1")), ("Help", advisor_scene(&compiled, "Help"))] {
        for split in [Split::Standard, Split::Vertical1To1, Split::Vertical2To1, Split::Horizontal1To1] {
            for place in [Place::LeftOrBottom, Place::RightOrTop, Place::Floating] {
                for capture in [false, true] {
                    let what = format!("{name}, {split:?}, {place:?}, capture {capture}");
                    let settings = Settings { split, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() };
                    let c = compose(&compiled, &settings, &scene, capture);
                    let hit = expected(split, place);
                    assert_eq!(c.layout.placed_screen.map(|f| f.hit), hit, "{what}");
                    let (total, _) = pixels(&c.image, WHOLE, PANEL);
                    assert!(total > 0, "{what}: the screen is drawn");
                    let (a, b) = c.layout.panes();
                    if let Some(hit) = hit {
                        let (target, other) = if hit == Hit::Main { (a, b.unwrap()) } else { (b.unwrap(), a) };
                        assert_eq!(pixels(&c.image, target, PANEL).0, total, "{what}: the whole screen in its pane");
                        assert_eq!(pixels(&c.image, other, PANEL).0, 0, "{what}: nothing of it in the other pane");
                        assert!(c.frame.cameras.iter().any(|k| k.cells && !k.stacked && [k.origin.0, k.origin.1, k.viewport.0 as i32, k.viewport.1 as i32] == other),
                            "{what}: the other pane keeps its own world camera");
                        assert!(pixels(&c.image, other, GROUND).0 > (other[2] * other[3]) as usize / 2, "{what}: the other pane shows the world");
                        // The screen's every corner takes clicks where the game has it.
                        let [x, y, w, h] = CITY_PANEL;
                        for corner in [(x + 8, y + 8), (x + w - 8, y + 8), (x + 8, y + h - 8), (x + w - 8, y + h - 8)] {
                            let shown = c.layout.driver_rect_to_window([corner.0, corner.1, 1, 1]);
                            let p = (shown[0].floor() as i32, shown[1].floor() as i32);
                            assert!(inside(target, p), "{what}: {corner:?} shows at {p:?} in {target:?}");
                            let q = c.layout.route_window(p).unwrap_or_else(|| panic!("{what}: {p:?} goes nowhere"));
                            assert!((q.0 - corner.0).abs() <= 2 && (q.1 - corner.1).abs() <= 2, "{what}: {p:?} reaches {q:?}, not {corner:?}");
                        }
                    }
                    if name == "City" {
                        let o = assert_overlay_on_city(&c, &what);
                        // A click on the city's tile reaches the game where its
                        // screen has that tile.
                        let q = c.layout.route_window((o.0.round() as i32, o.1.round() as i32)).unwrap();
                        assert!((q.0 - CITY_CENTRE.0).abs() <= 2 && (q.1 - CITY_CENTRE.1).abs() <= 2, "{what}: the city's tile clicked at {q:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn the_city_screens_overlay_stays_on_the_city_for_every_row_parity_seam_zoom_and_game_camera() {
    let compiled = compiled();
    // Odd and even rows, both sides of the wrap seam, the map's top edge.
    for city in [(21, 31), (20, 30), (0, 30), (79, 31), (1, 1), (78, 58)] {
        let ours = city_camera(city);
        // The game's own camera: ours, wrapped by the world's width, or
        // clamped onto the map (the overlay moved onto ours by the bridge),
        // the game's view zoomed out before the screen opened.
        let games = [(ours, 1), ((ours.0 + 64 * 2 * COLS as i32, ours.1), 1), ((ours.0.max(0), ours.1.max(0)), 1), (((ours.0 / 2), ours.1 / 2), 2)];
        for (game, downscale) in games {
            let scene = city_scene(&compiled, city, downscale, game);
            for (split, place) in [(Split::Vertical1To1, Place::LeftOrBottom), (Split::Vertical2To1, Place::RightOrTop), (Split::Horizontal1To1, Place::RightOrTop), (Split::Vertical1To1, Place::Floating), (Split::Standard, Place::Floating)] {
                for capture in [false, true] {
                    let settings = Settings { split, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() };
                    let c = compose(&compiled, &settings, &scene, capture);
                    assert_overlay_on_city(&c, &format!("city {city:?}, game camera {game:?} at 1/{downscale}, {split:?}, {place:?}, capture {capture}"));
                }
            }
        }
    }
}

/// The city screen with layer data of its own (as its fog of war is, see
/// docs/per-view-layers.md): its shade layer in a slot, shaded everywhere,
/// which only its view's binding draws (the player's own shade is empty).
fn city_scene_with_own_shade(compiled: &Compiled) -> Scene {
    let mut s = city_scene(compiled, (21, 31), 1, city_camera((21, 31)));
    let n = (COLS * ROWS) as usize;
    let shade = compiled.value("shade", "Shade/0").unwrap();
    s.slots.push(scene::LayerSlot { id: 5, layer: 1, cells: scene::LayerCells { values: vec![shade; n], phases: None, offsets: None } });
    s.bindings.push(scene::Binding { id: 1, slots: vec![(1, 5)] });
    s.views[1].view.binding = 1;
    s
}

#[test]
fn the_city_screens_own_layers_show_in_its_view_and_the_other_pane_keeps_the_players() {
    let compiled = compiled();
    let scene = city_scene_with_own_shade(&compiled);
    let plain = city_scene(&compiled, (21, 31), 1, city_camera((21, 31)));
    for (split, place) in [(Split::Vertical1To1, Place::LeftOrBottom), (Split::Vertical1To1, Place::RightOrTop), (Split::Standard, Place::Floating)] {
        let what = format!("{split:?}, {place:?}");
        let settings = Settings { split, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() };
        let c = compose(&compiled, &settings, &scene, false);
        let p = compose(&compiled, &settings, &plain, false);
        // Only the city's view binds its slots; every other camera, the
        // other pane's world among them, the shared layers.
        let city = c.frame.view_cameras[1].expect("the city's view is drawn") as usize;
        for (i, k) in c.frame.cameras.iter().enumerate() {
            assert_eq!(k.binding, if i == city { 1 } else { 0 }, "{what}: camera {i}");
        }
        let (a, b) = c.layout.panes();
        if let Some(hit) = expected(split, place) {
            let other = if hit == Hit::Main { b.unwrap() } else { a };
            assert!(pixels(&c.image, other, GROUND).0 > (other[2] * other[3]) as usize / 2, "{what}: the other pane shows the player's world");
            assert_eq!(pixels(&c.image, other, SHADE).0, 0, "{what}: unshaded, as the player's");
            assert_eq!(pixels(&c.image, other, GROUND), pixels(&p.image, other, GROUND), "{what}: as without the city's own layers");
        }
        // The city's band shows its own shade, which the player's has not.
        let band = c.layout.driver_rect_to_window(CITY_MAP);
        let band = [band[0].ceil() as i32 + 2, band[1].ceil() as i32 + 2, (band[2] as i32 - 4).max(1), (band[3] as i32 - 4).max(1)];
        assert_eq!(pixels(&p.image, band, SHADE).0, 0, "{what}: no shade without its own layers");
        // (All of it but the city's marks over it.)
        assert!(pixels(&c.image, band, SHADE).0 > (band[2] * band[3]) as usize * 99 / 100, "{what}: the city's view draws its own slot");
    }
}

#[test]
fn slots_follow_the_scene_incrementally_as_a_new_world_has_them() {
    let compiled = compiled();
    let plain = city_scene(&compiled, (21, 31), 1, city_camera((21, 31)));
    let mut world = crate::scene_cells(&compiled, &plain);
    let _ = world.take_dirty();
    // The screen opens: its slot and binding arrive.
    let mut open = city_scene_with_own_shade(&compiled);
    let ground = 0;
    open.slots[0].cells.values[7] = ground;
    crate::scene_slots(&compiled, &open, &mut world);
    let fresh = crate::scene_cells(&compiled, &open);
    let held = |w: &tile_render::TileWorld| (w.slots().collect::<Vec<_>>(), w.bindings().collect::<Vec<_>>(), w.bound(1, tile_render::LayerId(1)));
    assert_eq!(held(&world), held(&fresh));
    assert!(matches!(world.take_dirty(), tile_render::Dirty::Records(r) if r.contains(&(world.slot_region(tile_render::SlotId(0)), tile_render::ALL_CELLS))));
    // Its cells change: only those.
    open.slots[0].cells.values[9] = ground;
    open.slots[0].cells.values[7] = compiled.value("shade", "Shade/0").unwrap();
    crate::scene_slot_cells_changed(&compiled, &open, &mut world, &[(5, 9), (5, 7)]);
    let fresh = crate::scene_cells(&compiled, &open);
    let slot = tile_render::SlotId(0);
    for cell in 0..COLS * ROWS {
        let (c, r) = (cell % COLS, cell / COLS);
        assert_eq!(world.get_in(slot, c, r), fresh.get_in(slot, c, r), "cell {cell}");
    }
    assert_eq!(world.take_dirty(), tile_render::Dirty::Records(vec![(world.slot_region(slot), 9), (world.slot_region(slot), 7)]));
    assert_eq!((world.get_in(slot, 9, 0), world.get_in(slot, 7, 0)), (0, compiled.value("shade", "Shade/0").unwrap()));
    // It closes: no slot, no binding.
    crate::scene_slots(&compiled, &plain, &mut world);
    assert_eq!((world.slots().count(), world.bindings().count(), world.slot_bytes()), (0, 0, 0));
}

#[test]
fn popup_layers_never_change_the_underlying_named_screen_or_world_layout() {
    use viewer_settings::{Place, ScreenPlace};
    let compiled = compiled();
    for kind in ["F6", "Menu", "City", "full-window"] {
        let mut base = if kind == "City" { city_scene(&compiled, (10, 10), 1, (0, 0)) }
            else { advisor_scene(&compiled, if kind == "full-window" { "F1" } else { kind }) };
        base.screen_size = SCREEN;
        if kind == "full-window" {
            base.interface.screen_view = None;
            let underlay = base.views.iter_mut().find(|v| v.id == 258).unwrap();
            underlay.view.menu = true;
            underlay.view.origin = (0, 0);
            underlay.view.viewport = SCREEN;
            underlay.overlay = panel(&compiled, [0, 0, 1920, 1080]);
        }
        for (window, pixel_scale) in [((1920, 1080), 1), ((1280, 800), 1), ((1080, 1920), 3), ((3840, 2160), 3)] {
            for scale_menus in [false, true] {
                for split in [Split::Standard, Split::Vertical1To1, Split::Horizontal1To1] {
                    for place in [Place::Floating, Place::LeftOrBottom, Place::RightOrTop] {
                        let what = format!("{kind} {window:?} pixel{pixel_scale} scale{scale_menus} {split:?} {place:?}");
                        let resolution = viewer_settings::Resolution { pixel_scale, scale_menus, menu_scale_percent: 75.0, world_scale: 1.5, ..Default::default() };
                        let mut layout = Layout::default();
                        layout.apply_settings(&Settings { split, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() });
                        let draw = |layout: &mut Layout, scene: &Scene| {
                            let plan = scaling::plan(window, 1.0, SCREEN, &resolution).for_scene(scene, |s| sprite_rect(&compiled, s));
                            layout.driver_screen = Some(SCREEN);
                            layout.menu_scale = plan.menu.zoom;
                            layout.world_scale = plan.world;
                            layout.screen_content = named_screen_content(&compiled, scene);
                            let mut frame = layout.frame(scene, plan.canvas(), (0, 0, 0, 0), interface_rects(&compiled, scene), Some(world_bounds(&compiled, scene)));
                            let arranged = scaling::arrange_panned(&plan, scene, &mut frame, &mut layout.menu_pan, |s| sprite_rect(&compiled, s));
                            layout.set_interface_shown(arranged.shown);
                            frame
                        };
                        for _ in 0..3 { draw(&mut layout, &base); }
                        let before = draw(&mut layout, &base);
                        let content = named_screen_content(&compiled, &base);
                        let panel_rect = layout.driver_rect_to_window(CITY_PANEL);
                        // A prompt above/left of the panel, a larger nested/replacement
                        // prompt, returning to the first and finally dismissing both.
                        for popup in [Some([8, 10, 400, 160]), Some([30, 2, 1100, 350]), Some([8, 10, 400, 160]), None] {
                            let mut scene = base.clone();
                            if let Some([x, y, w, h]) = popup {
                                scene.interface.popup = Some(257);
                                scene.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (x, y), viewport: (w as u32, h as u32), ..Default::default() }, overlay: panel(&compiled, [x, y, w, h]) });
                            }
                            assert_eq!(named_screen_content(&compiled, &scene), content, "{what}: popup never owns the screen bounds");
                            let frame = draw(&mut layout, &scene);
                            assert_eq!(layout.driver_rect_to_window(CITY_PANEL), panel_rect, "{what}: screen mapping after {popup:?}");
                            for (i, view) in base.views.iter().enumerate() {
                                let camera = |f: &Frame| f.view_cameras[i].map(|c| f.cameras[c as usize]);
                                assert_eq!(camera(&frame), camera(&before), "{what}: underlying view{} after {popup:?}", view.id);
                            }
                            if let Some([x, y, w, h]) = popup {
                                let shown = layout.interface_shown.as_ref().unwrap();
                                let modal = shown.iter().find(|p| p.is_modal()).unwrap_or_else(|| panic!("{what}: independent modal {popup:?}"));
                                let target = modal.to_canvas([x + w / 2, y + h / 2, 1, 1]);
                                let at = (target[0].floor() as i32, target[1].floor() as i32);
                                let driver = layout.route_window(at).unwrap_or_else(|| panic!("{what}: modal takes input at {at:?}"));
                                assert!(driver.0 >= x && driver.0 < x + w && driver.1 >= y && driver.1 < y + h, "{what}: {at:?} -> {driver:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn an_opaque_popup_changes_pixels_only_inside_its_overlay_not_the_screen_beneath_it() {
    check_opaque_modal_stack(None);
}

#[test]
#[ignore = "requires a GPU; validates opaque popup order against the CPU oracle"]
fn opaque_modal_stack_gpu_matches_the_independent_cpu_pixels() {
    let gpu = tile_render::Headless::new().unwrap();
    eprintln!("opaque modal adapter: {:?}", gpu.adapter.get_info());
    check_opaque_modal_stack(Some(&gpu));
}

fn check_opaque_modal_stack(gpu: Option<&tile_render::Headless>) {
    use viewer_settings::{Place, ScreenPlace};
    let compiled = compiled();
    for kind in ["F6", "City", "Menu", "title-Menu", "preview", "legacy-Menu"] {
        let mut base = if kind == "City" { city_scene(&compiled, (10, 10), 1, (0, 0)) } else { advisor_scene(&compiled, if matches!(kind, "title-Menu" | "preview" | "legacy-Menu") { "Menu" } else { kind }) };
        base.screen_size = SCREEN;
        if kind == "title-Menu" {
            base.shape = Default::default();
            base.views.retain(|v| v.view.interface_only);
            base.screen_size = (1280, 800);
            let native = base.views.iter_mut().find(|view| view.id == 258).unwrap();
            native.view.origin = (128, 16);
            native.overlay = panel(&compiled, [128, 16, 1024, 768]);
        }
        if kind == "preview" {
            base.interface.beside = Some(scene::Beside { screen: CITY_PANEL, edge: scene::Edge::Left, world: None });
        }
        let mut open = base.clone();
        open.interface.popup = Some(257);
        open.interface.modal_stack = scene::ModalStack([Some(257), None, None, None]);
        let rect = if kind == "title-Menu" { [312, 100, 656, 174] } else { [8, 2, 400, 160] };
        let art = panel(&compiled, rect).into_iter().map(|s| FreeSprite { set: compiled.set("Shade/0").unwrap().0.raw(), ..s }).collect();
        open.views.push(ViewState { id: 257, view: View { interface_only: true, origin: (rect[0], rect[1]), viewport: (rect[2] as u32, rect[3] as u32), ..Default::default() }, overlay: art });
        open.views.sort_by_key(|view| view.id);
        for place in [Place::Floating, Place::RightOrTop] {
            let plan = scaling::plan(if kind == "title-Menu" { (1012, 632) } else { (1280, 800) }, 1.0, base.screen_size, &viewer_settings::Resolution {
                pixel_scale: if kind == "title-Menu" { 1 } else { 2 }, scale_menus: true,
                menu_scale_percent: if kind == "title-Menu" { 100.0 } else { 75.0 }, ..Default::default()
            }).for_scene(&base, |s| sprite_rect(&compiled, s));
            let mut layout = Layout::default();
            layout.apply_settings(&Settings { split: Split::Vertical1To1, screen_place: ScreenPlace { vertical: place, horizontal: place }, ..Default::default() });
            layout.driver_screen = Some(base.screen_size);
            layout.ui_in_pane = kind == "legacy-Menu";
            let draw = |layout: &mut Layout, scene: &Scene| {
                layout.screen_content = named_screen_content(&compiled, scene);
                let mut frame = layout.frame(scene, plan.canvas(), (0, 0, 0, 0), interface_rects(&compiled, scene), Some(world_bounds(&compiled, scene)));
                let arranged = scaling::arrange_panned(&plan, scene, &mut frame, &mut layout.menu_pan, |s| sprite_rect(&compiled, s));
                let modal = arranged.shown.as_ref().and_then(|shown| shown.iter().find(|p| p.is_modal())).map(|p| p.rect)
                    .or_else(|| scene.interface.popup.and_then(|id| scene.view(id)).filter(|_| layout.fit.is_some()).map(|view| {
                        let [x,y,w,h] = layout.driver_rect_to_window([view.view.origin.0, view.view.origin.1, view.view.viewport.0 as i32, view.view.viewport.1 as i32]);
                        [x.floor() as i32,y.floor() as i32,(x+w).ceil() as i32-x.floor() as i32,(y+h).ceil() as i32-y.floor() as i32]
                    }));
                layout.set_interface_shown(arranged.shown);
                let world = scene_world_arranged(&compiled, scene, &frame.view_cameras, &arranged.overlay_cameras);
                let reference = tile_render::reference::render_views(&world, &compiled.atlas, &frame.cameras, plan.canvas(), 0, BACKGROUND);
                let image = if let Some(gpu) = gpu {
                    let mut renderer = tile_render::TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
                    renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
                    renderer.set_world(&gpu.device, &world);
                    let actual = gpu.render_views(&mut renderer, &frame.cameras, plan.canvas(), 0, BACKGROUND);
                    assert_eq!(actual.pixels, reference.pixels, "{kind} {place:?}: GPU/reference modal composition");
                    actual
                } else { reference };
                (image, modal)
            };
            draw(&mut layout, &base);
            let (before, _) = draw(&mut layout, &base);
            let (after, modal) = draw(&mut layout, &open);
            let [mx, my, mw, mh] = modal.expect("independently composed popup");
            assert_eq!(after.get((mx + mw / 2) as u32, (my + mh / 2) as u32), SHADE, "{kind} {place:?}: opaque popup must win over opaque screen");
            let mut changed_inside = 0;
            for y in 0..before.height {
                for x in 0..before.width {
                    if before.get(x, y) != after.get(x, y) {
                        assert!((x as i32) >= mx && (x as i32) < mx + mw && (y as i32) >= my && (y as i32) < my + mh,
                            "{kind} {place:?}: popup changed underlying pixel {x},{y} outside {modal:?}");
                        changed_inside += 1;
                    }
                }
            }
            assert!(changed_inside > 0, "{kind} {place:?}: popup must actually cover its underlay");
            let lower_mapping = layout.interface_shown.as_ref().and_then(|shown| shown.iter().find(|piece| piece.driver == rect)).map_or_else(|| layout.driver_rect_to_window(rect), |piece| piece.to_canvas(rect));
            let mut nested = open.clone();
            nested.interface.popup = Some(259);
            nested.interface.modal_stack = scene::ModalStack([Some(257), Some(259), None, None]);
            let top = [30, 5, 240, 96];
            let mut art = Vec::new();
            for y in (top[1]..top[1] + top[3]).step_by(16) {
                for x in (top[0]..top[0] + top[2]).step_by(16) {
                    art.push(screen_sprite(&compiled, "Mark/0", x, y));
                }
            }
            nested.views.push(ViewState { id: 259, view: View { interface_only: true, origin: (top[0], top[1]), viewport: (top[2] as u32, top[3] as u32), ..Default::default() }, overlay: art });
            nested.views.sort_by_key(|view| view.id);
            let (nested_image, top_shown) = draw(&mut layout, &nested);
            assert_eq!(layout.interface_shown.as_ref().and_then(|shown| shown.iter().find(|piece| piece.driver == rect)).map_or_else(|| layout.driver_rect_to_window(rect), |piece| piece.to_canvas(rect)), lower_mapping, "{kind} {place:?}: pushing top keeps lower popup fixed");
            let [tx, ty, tw, th] = top_shown.unwrap();
            assert_eq!(nested_image.get((tx + tw / 2) as u32, (ty + th / 2) as u32), MARK, "{kind} {place:?}: nested opaque popup wins");
            let at = (tx + tw / 2, ty + th / 2);
            let driver = layout.route_window(at).expect("top popup consumes pointer input");
            assert!(driver.0 >= top[0] && driver.0 < top[0] + top[2] && driver.1 >= top[1] && driver.1 < top[1] + top[3], "{kind} {place:?}: top popup owns {at:?}, got {driver:?}");
            for y in 0..after.height {
                for x in 0..after.width {
                    if (x as i32) < tx || (x as i32) >= tx + tw || (y as i32) < ty || (y as i32) >= ty + th {
                        assert_eq!(after.get(x, y), nested_image.get(x, y), "{kind} {place:?}: nested push changed lower underlay at {x},{y}");
                    }
                }
            }
            let (restored, _) = draw(&mut layout, &open);
            assert_eq!(restored.pixels, after.pixels, "{kind} {place:?}: top dismissal restores exact prior composition");
        }
    }
}
