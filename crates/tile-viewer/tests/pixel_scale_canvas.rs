//! The pixel scale (Preferences, Resolution: Auto, ×1 … ×6) sets the size
//! the frame is rendered at: the window's physical pixels divided by `n`
//! each way, enlarged once onto the window in `n`×`n` boxes. The GPU then
//! renders `n²` times fewer pixels; input maps through the same boxes.
use tile_render::{Camera, ColorModel, Grid, Headless, Layout, LayerSpec, Present, RgbaImage, SpriteSets, TileRenderer, TileWorld};
use tile_viewer::scaling;
use viewer_settings::Resolution;

/// Physical window sizes and desktop scales: TS Neo's Retina display (the
/// macOS case), a MacBook's, and a plain desktop.
const WINDOWS: [((u32, u32), f64); 3] = [((3258, 1904), 2.0), ((2880, 1800), 2.0), ((2560, 1440), 1.0)];

/// A frame as the layout makes it, in the canvas's pixels: the world, and
/// a menu scaled by the menu scale (a zoomed view, text from outlines).
fn cameras(canvas: (u32, u32)) -> Vec<Camera> {
    vec![Camera::new(canvas), Camera { cells: false, zoom: (3, 2), ..Camera::new(canvas) }]
}

#[test]
fn every_pixel_scale_renders_the_window_divided_by_it() {
    for (window, desktop) in WINDOWS {
        for n in 1..=6u32 {
            let settings = Resolution { pixel_scale: n, ..Default::default() };
            assert_eq!(settings.pixel_scale_in(window, desktop), n, "{window:?} allows ×{n}");
            let plan = scaling::plan(window, desktop, (1024, 768), &settings);
            let canvas = (window.0 / n, window.1 / n);
            assert_eq!(plan.canvas(), canvas);
            // Text from outlines in every zoomed view: what made the frame
            // be composed at the window's resolution before.
            let drawn = scaling::drawn_on_canvas(&cameras(canvas), &[], plan.present, |zoom| zoom.0 != zoom.1);
            assert_eq!(drawn.pixel_box, 1, "×{n} on {window:?}");
            assert_eq!(drawn.present, Present { canvas, zoom: (n, 1), at: plan.present.at, target: window }, "×{n} on {window:?}");
            assert!(drawn.cameras.iter().all(|c| c.viewport == canvas), "×{n}: views in the canvas's pixels");
            assert!(drawn.cameras[1].zoomed_art, "the scaled menu keeps its text from outlines");
            // The opt-in (`text=window`) renders every window pixel.
            let window_text = scaling::drawn(&cameras(canvas), &[], plan.present, |zoom| zoom.0 != zoom.1);
            if n > 1 {
                assert_eq!(window_text.present.canvas, (canvas.0 * n, canvas.1 * n));
            }
            // Pointer input: a window pixel on the picture is in the box of
            // the canvas pixel it maps to, and that box's middle maps back
            // to it. (The border, under n pixels, maps to the edge.)
            let at = plan.present.at;
            let end = (at.0 + (canvas.0 * n) as i32 - 1, at.1 + (canvas.1 * n) as i32 - 1);
            assert_eq!(plan.to_screen((0.0, 0.0)), (0, 0));
            for p in [(at.0 as f64, at.1 as f64), (window.0 as f64 / 3.0, window.1 as f64 / 2.0), (end.0 as f64, end.1 as f64)] {
                let c = plan.to_screen(p);
                let (x0, y0) = (plan.present.at.0 + c.0 * n as i32, plan.present.at.1 + c.1 * n as i32);
                assert!((x0..x0 + n as i32).contains(&(p.0 as i32)) && (y0..y0 + n as i32).contains(&(p.1 as i32)), "×{n}: {p:?} → {c:?}");
                assert_eq!(plan.to_screen({ let w = plan.to_window(c); (w.0 as f64, w.1 as f64) }), c);
            }
        }
    }
}

/// A Civ III-like world: RGB555, two passes (terrain and a translucent
/// one) of 64×32 cells, enough to cover a 4K canvas.
fn world() -> (tile_render::Atlas, TileWorld) {
    let mut atlas = tile_render::AtlasBuilder::new(512);
    let mut sets = SpriteSets::default();
    let ground: Vec<_> = (0..4u32).map(|k| sets.single(atlas.add(&RgbaImage::from_fn(64, 32, |x, y| [(x * 4) as u8, (y * 8) as u8, (k * 60) as u8, 255]), (0, 0)).unwrap())).collect();
    let shade = sets.single(atlas.add(&RgbaImage::from_fn(64, 32, |x, _| [0, 0, 0, if x % 2 == 0 { 128 } else { 64 }]), (0, 0)).unwrap());
    let grid = Grid { cols: 80, rows: 160, layout: Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (64, 32) };
    let mut world = TileWorld::new(grid, sets);
    world.color = ColorModel::Quantized { bits: 5 };
    let terrain = world.add_layer(LayerSpec::new("terrain", ground));
    let over = world.add_layer(LayerSpec { pass: 1, ..LayerSpec::new("over", vec![tile_render::SpriteSetId::EMPTY, shade]) });
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            world.set(terrain, c, r, (c + r) % 4);
            world.set(over, c, r, (c + r) % 2);
        }
    }
    (atlas.finish(), world)
}

/// On a GPU: the picture the views are drawn into is the canvas (the
/// window over `n`), and the window shows it in whole `n`×`n` boxes. With
/// timestamp queries, prints the GPU time of a frame per pixel scale
/// (`cargo test -p tile-viewer --test pixel_scale_canvas -- --ignored --nocapture`).
#[test]
#[ignore = "requires a GPU"]
fn gpu_renders_the_canvas_not_the_window() {
    let gpu = Headless::new().expect("GPU");
    let (atlas, world) = world();
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.set_atlas(&gpu.device, &gpu.queue, &atlas);
    renderer.set_world(&gpu.device, &world);
    let timing = renderer.enable_timing(&gpu.device, &gpu.queue);
    let window = (3840u32, 2160u32);
    for text_at_window in [false, true] {
        for n in 1..=6u32 {
            let settings = Resolution { pixel_scale: n, ..Default::default() };
            let plan = scaling::plan(window, 1.0, (1024, 768), &settings);
            let canvas = plan.canvas();
            let cams = vec![Camera::new(canvas)];
            let drawn = if text_at_window {
                scaling::drawn(&cams, &[], plan.present, |zoom| zoom.0 != zoom.1)
            } else {
                scaling::drawn_on_canvas(&cams, &[], plan.present, |zoom| zoom.0 != zoom.1)
            };
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("window"),
                size: wgpu::Extent3d { width: window.0, height: window.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let mut spans = Vec::new();
            let mut image = None;
            for frame in 0..24u32 {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                // (A new tick each frame: drawn whole, not reused.)
                renderer.render_layered(&gpu.device, &gpu.queue, &mut encoder, &target, &drawn.cameras, drawn.cameras.len(), &drawn.copies, frame, [0, 0, 0, 255], drawn.present);
                if let Some(t) = renderer.timer() { t.finish(&mut encoder); }
                if frame == 23 {
                    image = Some(tile_render::gpu::read_texture(&gpu.device, &gpu.queue, encoder, &target));
                } else {
                    gpu.queue.submit([encoder.finish()]);
                }
                renderer.controls_submitted();
                if let Some(t) = renderer.timer() { t.submitted(); }
                let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                if let Some(t) = renderer.timer() {
                    for f in t.collect(&gpu.device) {
                        if frame == 23 && n % 5 == 1 { eprintln!("  ×{n} passes: {:?}", f.passes.iter().map(|(l, ms)| format!("{l} {ms:.3}")).collect::<Vec<_>>()); }
                        spans.push(f.span);
                    }
                }
            }
            let composed = if text_at_window { (canvas.0 * n, canvas.1 * n) } else { canvas };
            assert_eq!(renderer.composed_size(), Some(composed), "×{n}, text at window {text_at_window}");
            if !text_at_window {
                // Whole boxes: each canvas pixel n×n identical window pixels.
                let image = image.unwrap();
                let at = plan.present.at;
                for (cx, cy) in [(0u32, 0u32), (canvas.0 / 2, canvas.1 / 3), (canvas.0 - 1, canvas.1 - 1)] {
                    let p = |dx: u32, dy: u32| image.get(at.0 as u32 + cx * n + dx, at.1 as u32 + cy * n + dy);
                    assert!((0..n).all(|dx| (0..n).all(|dy| p(dx, dy) == p(0, 0))), "×{n}: box of canvas pixel ({cx}, {cy})");
                }
            }
            if timing && !spans.is_empty() {
                spans.drain(..spans.len().min(4));
                spans.sort_by(f64::total_cmp);
                let median = spans[spans.len() / 2];
                eprintln!("pixel scale ×{n} on {}×{}{}: composed {}×{}, GPU {median:.3} ms per frame (median of {})",
                    window.0, window.1, if text_at_window { " (text=window)" } else { "" }, composed.0, composed.1, spans.len());
            }
        }
    }
}
