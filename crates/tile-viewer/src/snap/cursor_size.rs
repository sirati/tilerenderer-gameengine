//! The snapping pointer at the cursor's size (`viewer_settings::Display::
//! cursor_size`), as the viewer's sprite cursor.

use super::*;
use crate::cursor::{self, Marks, Scale, SetArt};
use scene::{Scene, View, ViewState};

struct NoFiles;
impl render_desc::ImageLoader for NoFiles {
    fn load(&self, source: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
        Err(format!("no file {source}"))
    }
}

/// The driver's arrow (`Viewer@cursors/0`), 14×22, its tip 3, 2 in; the
/// ghost the same.
fn compiled() -> render_desc::Compiled {
    render_desc::Description::parse(
        r#"
        grid = { layout = "orthogonal", step = [8, 8] }
        [images.p]
        source = "pointer:255,255,255,255:14x22"
        [images.g]
        source = "pointer:255,255,255,128:14x22"
        [sheets.p]
        image = "p"
        cell = [14, 22]
        origin = [0, 0]
        anchor = [3, 2]
        group = "Viewer@cursors"
        [sheets.g]
        image = "g"
        cell = [14, 22]
        anchor = [3, 2]
        group = "Viewer@cursor_ghost"
        "#,
    )
    .unwrap()
    .compile(&NoFiles, 256)
    .unwrap()
}

fn scene() -> Scene {
    let main = View { camera_x: 1000, camera_y: 500, zoom: (1, 1), ..Default::default() };
    let ui = View { interface_only: true, ..Default::default() };
    Scene {
        seq: 1,
        views: vec![ViewState { id: 0, view: main, overlay: vec![] }, ViewState { id: 255, view: ui, overlay: vec![] }],
        shape: scene::WorldShape { cols: 62, rows: 125, wrap_x: true, wrap_y: false },
        interactables: vec![Interactable { region: None, alt_right_click: false, anchor: None, rect: [300, 520, 60, 30], kind: InteractableKind::Button, id: 2 }],
        ..Default::default()
    }
}

#[test]
fn the_snapping_pointer_is_the_sprite_cursors_size_and_hot_spot() {
    let compiled = compiled();
    let s = scene();
    let mut l = Layout::default();
    l.configure("main=pointer").unwrap();
    let bounds = crate::layout::WorldBounds { size: (8000, 4000), wrap: (true, false) };
    l.frame(&s, (800, 600), (0, 0, 0, 0), vec![[0, 500, 800, 100]], Some(bounds));
    let mut snap = SnapInput::default();
    snap.route(Input::Key { vk: 0x12, down: true, text: None }, &mut l, &s, 0.0, Rates::default());
    snap.tick(&mut l, &s, 1.0, Rates::default());
    // (The pointer alone: the attractor's art is `targeting`'s.)
    assert_eq!(snap.attractors.len(), 1);
    snap.attractors.clear();
    let at = snap.snapper.pointer().unwrap();
    let at = (at.0.round() as i32, at.1.round() as i32);
    let set = compiled.set(ARROW_SPRITE).unwrap().0;
    let art = SetArt::of(&compiled, set).unwrap();
    let (overlay, window) = (3, (800, 600));
    for present in [1, 2, 3] {
        for size in [24, 32, 40, 48, 64, 96] {
            let scale = Scale::new(size, present);
            // The sprite cursor at that point, and the snapping pointer.
            let marks = Marks { window: vec![at], ..Default::default() };
            let cursor = cursor::compose(art, &marks, &[], &[], scale, window, overlay, (0, 0));
            let snapped = snap.compose(&compiled, overlay, overlay + 1, scale, window);
            assert_eq!(snapped.sprites, cursor.sprites, "size {size} at pixel scale {present}");
            assert_eq!(snapped.top, cursor.top, "size {size} at pixel scale {present}: the same cameras, size and hot spot");
            if !scale.unscaled() {
                assert_eq!(snapped.top[0].viewport, (scale.span(14), scale.span(22)));
            }
        }
    }
    // Numbered after cameras taken already.
    let snapped = snap.compose(&compiled, overlay, overlay + 3, Scale::new(64, 1), window);
    assert_eq!(snapped.sprites[0].view, Some(overlay + 3));
    // 1:1: as before, in the overlay.
    assert_eq!(snap.sprites(&compiled, overlay), snap.compose(&compiled, overlay, overlay + 1, Scale::default(), window).sprites);
}
