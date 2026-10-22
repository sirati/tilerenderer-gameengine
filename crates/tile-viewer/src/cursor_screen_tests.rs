//! The cursor on every screen, as the viewer's frame decides it
//! (`App::draw_frame`): with the mouse free the system pointer is the
//! cursor, never a sprite of ours, on the title and setup screens, Load
//! Scenario with its preview beside it, Preferences, an advisor and the
//! plain map; captured, or with button snapping on, the viewer draws it.

use super::*;
use crate::cursor::{self, Facts, Machine, Marks, Plan, SetArt, Shown};
use crate::{interface_rects, named_screen_content, scaling, sprite_rect, world_bounds};
use render_desc::Compiled;
use scene::{FreeSprite, View, ViewState};
use tile_render::RgbaImage;
use viewer_settings::{Layout as Settings, Split};

struct NoFiles;
impl render_desc::ImageLoader for NoFiles {
    fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
        panic!("generated art only")
    }
}

fn compiled() -> Compiled {
    render_desc::Description::parse(
        r#"
        [grid]
        layout = "staggered"
        step = [64, 32]
        [images.ground]
        source = "solid:20,160,40,255:128x64"
        [images.panel]
        source = "solid:200,30,40,255:64x64"
        [images.arrow]
        source = "pointer:255,255,255,255:14x22"
        [sheets.ground]
        image = "ground"
        cell = [128, 64]
        group = "Ground"
        [sheets.panel]
        image = "panel"
        cell = [64, 64]
        group = "Panel"
        [sheets.arrow]
        image = "arrow"
        cell = [14, 22]
        group = "Viewer@cursors"
        [[layers]]
        name = "ground"
        sets = ["Ground/*"]
    "#,
    )
    .unwrap()
    .compile(&NoFiles, 1024)
    .unwrap()
}

const SCREEN: (u32, u32) = (1920, 1080);
const DIALOG: [i32; 4] = [448, 156, 1024, 768];

fn panel(compiled: &Compiled, [x, y, w, h]: [i32; 4]) -> Vec<FreeSprite> {
    let set = compiled.set("Panel/0").unwrap().0.raw();
    let mut out = Vec::new();
    for py in (y..y + h).step_by(64) {
        for px in (x..x + w).step_by(64) {
            out.push(FreeSprite { set, x: px, y: py, screen: true, after_pass: u32::MAX, ticks_per_frame: 1, clip: Some([x, y, x + w, y + h]), ..Default::default() });
        }
    }
    out
}

fn world(compiled: &Compiled) -> Scene {
    let shape = scene::WorldShape { cols: 40, rows: 60, wrap_x: true, wrap_y: false };
    let mut s = Scene::empty_with(shape, compiled.layers.iter().map(|l| (l.per_cell_phase, l.per_cell_offset)));
    let ground = compiled.value("ground", "Ground/0").unwrap();
    for row in 0..60 {
        for col in 0..40 {
            s.set(0, col, row, ground);
        }
    }
    s
}

fn game_view() -> ViewState {
    ViewState { id: 0, view: View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() }, overlay: vec![] }
}

fn menu_view(compiled: &Compiled, r: [i32; 4]) -> ViewState {
    ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay: panel(compiled, r) }
}

/// The scenes as the bridge publishes them (`civ3_bridge::live`).
fn screens(compiled: &Compiled) -> Vec<(&'static str, Scene)> {
    let no_world = || Scene::empty(scene::WorldShape::default(), compiled.layers.len());
    // The title: no world, our own menu fitted to the window.
    let mut title = no_world();
    title.views = vec![ViewState { id: 0, view: View::default(), overlay: vec![] }, menu_view(compiled, [0, 0, 1920, 1080])];
    title.interface.menu_fit = true;
    // A setup screen: the game's own, mirrored, no world.
    let mut setup = no_world();
    setup.views = vec![ViewState { id: 0, view: View::default(), overlay: vec![] }, menu_view(compiled, DIALOG)];
    // Load Scenario: its screen at the left, the scenario's world beside it.
    let mut preview = world(compiled);
    preview.views = vec![game_view(), menu_view(compiled, [0, 0, 640, 1080])];
    preview.interface.beside = Some(scene::Beside { screen: [0, 0, 640, 1080], edge: scene::Edge::Left, world: None });
    // Preferences: a named menu screen in a part of its own, fitted.
    let mut prefs = no_world();
    prefs.views = vec![
        ViewState { id: 0, view: View::default(), overlay: vec![] },
        ViewState { id: 258, view: View { interface_only: true, origin: (DIALOG[0], DIALOG[1]), viewport: (DIALOG[2] as u32, DIALOG[3] as u32), ..Default::default() }, overlay: panel(compiled, DIALOG) },
    ];
    prefs.interface = scene::Interface { menu_fit: true, screen: Some("Menu".into()), screen_view: Some(258), ..Default::default() };
    // Preferences over the map.
    let mut prefs_map = world(compiled);
    prefs_map.views = vec![game_view(), prefs.views[1].clone()];
    prefs_map.interface = prefs.interface.clone();
    // An advisor: a screen part over the map.
    let mut advisor = world(compiled);
    advisor.views = vec![
        game_view(),
        ViewState { id: 258, view: View { interface_only: true, origin: (DIALOG[0], DIALOG[1]), viewport: (DIALOG[2] as u32, DIALOG[3] as u32), ..Default::default() }, overlay: panel(compiled, DIALOG) },
    ];
    advisor.interface.screen = Some("F1".into());
    advisor.interface.screen_view = Some(258);
    // A popup over the map.
    let mut popup = world(compiled);
    popup.views = vec![game_view(), ViewState { id: 257, view: View { interface_only: true, ..Default::default() }, overlay: panel(compiled, [800, 400, 320, 200]) }];
    popup.interface.popup = Some(257);
    vec![("title", title), ("setup", setup), ("Load Scenario", preview), ("Preferences", prefs), ("Preferences over the map", prefs_map), ("advisor", advisor), ("popup", popup)]
}

fn map(compiled: &Compiled) -> Scene {
    let mut s = world(compiled);
    s.views = vec![game_view()];
    s
}

/// What the frame shows of the cursor with the pointer at `p`: the plan,
/// whether the system pointer shows (and what), and the cursor sprites
/// drawn (as `App::draw_frame`).
struct Shot {
    plan: Plan,
    shown: Shown,
    sprites: usize,
}

fn shoot(compiled: &Compiled, settings: &Settings, scene: &Scene, p: Option<(i32, i32)>, snap_mode: bool) -> Shot {
    let plan = scaling::plan(SCREEN, SCREEN, &viewer_settings::Resolution::default());
    let mut l = Layout::default();
    let capture = p.is_none();
    l.apply_settings(&Settings { roles: if capture { viewer_settings::Roles::UnitMouse } else { settings.roles }, ..settings.clone() });
    l.grid_step = compiled.step;
    l.driver_screen = Some(plan.screen);
    l.menu_scale = plan.menu.zoom;
    let bounds = Some(world_bounds(compiled, scene));
    let before = map(compiled);
    l.frame(&before, plan.canvas(), (0, 0, 0, 0), vec![], Some(world_bounds(compiled, &before)));
    if capture {
        let [x, y, w, h] = l.panes().1.expect("B");
        assert!(l.capture_at((x + w / 2, y + h / 2)));
    }
    let draw = |l: &mut Layout| {
        l.screen_content = named_screen_content(compiled, scene);
        let mut frame = l.frame(scene, plan.canvas(), (0, 0, 0, 0), interface_rects(compiled, scene), bounds);
        let arranged = scaling::arrange_panned(&plan, scene, &mut frame, &mut l.menu_pan, |s| sprite_rect(compiled, s));
        l.set_interface_shown(arranged.shown.clone());
        frame
    };
    draw(&mut l);
    if let Some(p) = p {
        l.place_pointer(p);
    }
    let frame = draw(&mut l);
    let captured = l.captured();
    let facts = Facts {
        captured,
        snap_mode,
        unlocking: false,
        world_cursor: frame.markers.iter().filter_map(|m| if let Marker::Cursor(p) = m { Some(*p) } else { None }).collect(),
        cursor_view: l.world_cursor_view(),
        cursor_axes: l.world_cursor_axes().unwrap_or((false, false)),
        pointer: l.pointer_in_window(),
        pointer_inside: true,
        pointer_world: l.pointer_world(),
        interface_open: cursor::interface_open(scene),
        now: 1.0,
    };
    let steps = Machine::default().step(&facts);
    let snapping = cursor::snaps_to_tiles(&scene.interface, &facts);
    let marks: Marks = steps.marks.clone();
    let set = compiled.set("Viewer@cursors/0").unwrap().0;
    let art = SetArt::of(compiled, set).unwrap();
    let composed = cursor::compose(art, &marks, &frame.cameras, &frame.cursor_slots, cursor::Scale::default(), plan.canvas(), frame.cameras.len() as u32, (0, 0));
    let shown = cursor::system_pointer(scene::Cursor::Sprite(set.raw()), &cursor::Art::of(compiled), steps.hidden || snapping, false);
    Shot { plan: steps, shown, sprites: composed.sprites.len() }
}

fn points() -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in (40..1080).step_by(200) {
        for x in (40..1920).step_by(230) {
            out.push((x, y));
        }
    }
    out
}

#[test]
fn with_the_mouse_free_every_screen_has_the_system_pointer_and_no_cursor_sprite() {
    let compiled = compiled();
    let set = compiled.set("Viewer@cursors/0").unwrap().0.raw();
    for (name, scene) in screens(&compiled) {
        assert!(cursor::interface_open(&scene), "{name}");
        for split in [Split::Standard, Split::Vertical1To1, Split::Vertical2To1, Split::Horizontal1To1] {
            let settings = Settings { split, ..Default::default() };
            for p in points() {
                let what = format!("{name}, {split:?}, pointer at {p:?}");
                let shot = shoot(&compiled, &settings, &scene, Some(p), false);
                assert!(!shot.plan.hidden, "{what}: {:?}", shot.plan);
                assert_eq!(shot.shown, Shown::Custom(set), "{what}: the game's cursor art as the system pointer");
                assert!(shot.plan.marks.world.is_empty() && shot.plan.marks.input.is_none(), "{what}: {:?}", shot.plan.marks);
                assert_eq!(shot.sprites, 0, "{what}: no cursor sprite");
            }
        }
    }
}

#[test]
fn on_the_plain_map_the_free_pointer_is_the_systems_and_only_mirrored_in_another_pane() {
    let compiled = compiled();
    let scene = map(&compiled);
    assert!(!cursor::interface_open(&scene));
    for split in [Split::Standard, Split::Vertical1To1, Split::Horizontal1To1] {
        let settings = Settings { split, ..Default::default() };
        for p in points() {
            let what = format!("{split:?}, pointer at {p:?}");
            let shot = shoot(&compiled, &settings, &scene, Some(p), false);
            assert!(!shot.plan.hidden, "{what}");
            assert!(matches!(shot.shown, Shown::Custom(_)), "{what}: {:?}", shot.shown);
            if split == Split::Standard {
                assert_eq!(shot.sprites, 0, "{what}: one view, nothing mirrored");
            } else {
                assert!(shot.sprites <= 1, "{what}: at most the mirror in the other pane");
            }
        }
    }
}

#[test]
fn captured_or_snapping_the_viewer_draws_the_cursor() {
    let compiled = compiled();
    let scene = map(&compiled);
    let settings = Settings { split: Split::Vertical1To1, ..Default::default() };
    let shot = shoot(&compiled, &settings, &scene, None, false);
    assert!(shot.plan.hidden && shot.shown == Shown::Hidden, "{:?}", shot.plan);
    assert!(shot.sprites >= 1, "captured: the world cursor drawn");
    // Button snapping, on any screen: no system pointer, its own pointer
    // (`crate::snap`) the only cursor.
    for (name, scene) in screens(&compiled).into_iter().chain([("map", map(&compiled))]) {
        let shot = shoot(&compiled, &Settings::default(), &scene, Some((700, 500)), true);
        assert!(shot.plan.hidden && shot.shown == Shown::Hidden, "{name}: {:?}", shot.plan);
        assert_eq!(shot.plan.marks, Marks::default(), "{name}");
    }
}
