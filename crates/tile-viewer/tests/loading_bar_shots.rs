//! Pictures of a game loading with popups over its loading screen, as the
//! viewer arranges them (`scaling::bar_clear_of`): plain rectangles for the
//! menu (orange), the loading bar (green, its hourglass slot grey), a modal
//! popup (beige, brown border) and queued reports (light blue). With
//! `C3R_SHOTS=dir` the pictures are written there as PPMs, before (the bar
//! a part of the menu, as the driver sent it until now) and after.
//! `cargo test -p tile-viewer --test loading_bar_shots -- --ignored`
use render_desc::{Compiled, Description, ImageLoader};
use scene::{FreeSprite, QueuedPopup, Scene, SolidRect, View, ViewState};

fn compiled() -> Compiled {
    struct NoFiles;
    impl ImageLoader for NoFiles {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { Err("no art".into()) }
    }
    Description::parse("[grid]\nlayout=\"orthogonal\"\nstep=[1,1]\n").unwrap().compile(&NoFiles, 16).unwrap()
}

fn rect(x: i32, y: i32, w: u32, h: u32, rgba: [u8; 4]) -> FreeSprite {
    FreeSprite { x, y, screen: true, solid: Some(SolidRect { width: w, height: h, rgba }), ..Default::default() }
}

fn dialog([x, y, w, h]: [i32; 4], fill: [u8; 4]) -> Vec<FreeSprite> {
    vec![rect(x, y, w as u32, h as u32, [90, 50, 20, 255]), rect(x + 3, y + 3, w as u32 - 6, h as u32 - 6, fill), rect(x + w - 40, y + h - 34, 24, 24, [120, 120, 120, 255])]
}

/// The game's 1920×1080 screen loading a game: its 1024×768 menu in the
/// middle, the bar where the game draws it (a 200×24 bar beside the menu's
/// entries, its hourglass slot to its left), a tall report screen over
/// it, and `queued` reports the game no longer waits for.
fn loading(split_bar: bool, queued: usize) -> Scene {
    let screen = (1920u32, 1080u32);
    let menu = [448, 156, 1024, 768];
    let bar = [828, 530, 236, 24];
    let popup = [710, 300, 500, 420];
    let mut s = Scene { screen_size: screen, ..Default::default() };
    let mut menu_sprites = vec![rect(menu[0], menu[1], 1024, 768, [200, 110, 50, 255]), rect(menu[0] + 100, menu[1] + 60, 824, 180, [150, 30, 30, 255])];
    let bar_sprites = vec![rect(bar[0], bar[1], 32, 24, [160, 160, 160, 255]), rect(bar[0] + 36, bar[1], 200, 24, [40, 200, 60, 255])];
    if split_bar {
        s.views.push(ViewState { id: 0xFFFF_0100, view: View { interface_only: true, origin: (bar[0], bar[1]), viewport: (bar[2] as u32, bar[3] as u32), ..Default::default() }, overlay: bar_sprites });
        s.interface.loading_bar = Some(0xFFFF_0100);
    } else {
        menu_sprites.extend(bar_sprites);
    }
    s.views.push(ViewState { id: 255, view: View { interface_only: true, menu: true, ..Default::default() }, overlay: menu_sprites });
    s.views.push(ViewState { id: 0x9B_7784, view: View { interface_only: true, origin: (popup[0], popup[1]), viewport: (popup[2] as u32, popup[3] as u32), ..Default::default() }, overlay: dialog(popup, [235, 225, 190, 255]) });
    s.interface.popup = Some(0x9B_7784);
    for k in 0..queued {
        let id = 0x1100 + k as u32;
        let r = [628, 4, 376, 297];
        s.views.push(ViewState { id, view: View { interface_only: true, origin: (r[0], r[1]), viewport: (r[2] as u32, r[3] as u32), ..Default::default() }, overlay: dialog(r, [190, 215, 235, 255]) });
        s.interface.queued.insert(0, QueuedPopup { view: id, serial: 1 + k as u64, button: Some([336, 263, 24, 24]) });
    }
    s.views.sort_by_key(|v| v.id);
    s
}

#[test]
#[ignore = "requires a GPU"]
fn gpu_loading_bar_beside_the_popups() {
    let gpu = tile_render::Headless::new().expect("GPU");
    let compiled = compiled();
    let out = std::env::var_os("C3R_SHOTS").map(std::path::PathBuf::from);
    let config = tile_viewer::ViewerConfig::default();
    for (window, pixel_scale, menu_scale) in [((1920u32, 1080u32), 1u32, 100.0f32), ((1280, 720), 1, 100.0), ((2560, 1440), 2, 150.0)] {
        let resolution = viewer_settings::Resolution { pixel_scale, scale_menus: true, menu_scale_percent: menu_scale, ..Default::default() };
        for (name, split, queued) in [("before", false, 0), ("after", true, 0), ("after-queued", true, 2)] {
            let scene = loading(split, queued);
            let img = tile_viewer::render_planned(&gpu, &compiled, &scene, window, &resolution, &viewer_settings::Layout::default(), &config);
            // The bar's green shows whole after, never covered.
            let green = img.pixels.iter().filter(|p| p[..3] == [40, 200, 60]).count();
            if split {
                let (n, d) = { let p = tile_viewer::scaling::plan(window, scene.screen_size, &resolution); p.menu.zoom };
                assert!(green as u64 >= (200 * 24 * n as u64 * n as u64) / (d as u64 * d as u64) * 9 / 10, "{name} {window:?}: the bar shows whole ({green} px)");
            }
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let path = dir.join(format!("layout-loading-{name}-{}x{}-x{pixel_scale}-{menu_scale}pct.ppm", window.0, window.1));
                image_write(&path, &img);
            }
        }
    }
}

/// As a binary PPM (convert it to PNG outside).
fn image_write(path: &std::path::Path, img: &tile_render::RgbaImage) {
    let mut data = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
    data.extend(img.pixels.iter().flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(path, data).unwrap();
}
