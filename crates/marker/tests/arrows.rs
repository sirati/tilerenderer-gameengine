//! The selection marker's arrow and shadow, painted shapes laid out at the
//! scale of the stage drawing them: the GPU paints them as the reference
//! oracle does, within float tolerance.
use tile_render::{AtlasBuilder, Camera, Grid, Headless, Layout, PlacedSprite, Shape, SpriteSetId, SpriteSets, TileRenderer, TileWorld};

/// The viewer's shader module, drawing with its painter (the marker's).
const SHADERS: tile_render::Shaders = tile_render::Shaders::new(viewer_spirv::SPIRV, viewer_spirv::SPIRV_FALLBACK).with_painter(viewer_spirv::PAINTER);

/// The oracle of a renderer painting the marker.
const REFERENCE: tile_render::reference::Oracle<'static> = tile_render::reference::Oracle { effect: None, painter: Some(&marker::PAINTER) };

fn world() -> TileWorld {
    TileWorld::new(Grid { cols: 4, rows: 4, layout: Layout::Orthogonal, step: (32, 32), wrap_x: false, wrap_y: false }, SpriteSets::default())
}

fn shape(x: i32, y: i32, shape: Shape) -> PlacedSprite {
    // Deliberately invalid atlas/set metadata: shapes must not read it.
    PlacedSprite { shape: Some(shape), palette: u32::MAX, ..PlacedSprite::new(SpriteSetId::from_raw(u32::MAX), x, y) }
}

fn compare(gpu: &Headless, world: &TileWorld, cameras: &[Camera], tolerance: u8) {
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, SHADERS);
    renderer.set_world(&gpu.device, world);
    let atlas = AtlasBuilder::new(16).finish();
    for camera in cameras {
        let got = gpu.render(&mut renderer, camera, 0, [40, 60, 80, 255]);
        let want = REFERENCE.render(world, &atlas, camera, 0, [40, 60, 80, 255]);
        let mut off = 0;
        for y in 0..got.height {
            for x in 0..got.width {
                let (a, b) = (got.get(x, y), want.get(x, y));
                let diff = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                assert!(diff <= tolerance, "zoom {:?}, zoomed art {}, downscale {}: pixel {x},{y} {a:?}, reference {b:?}", camera.zoom, camera.zoomed_art, camera.downscale);
                off += usize::from(diff != 0);
            }
        }
        assert!(off * 50 <= (got.width * got.height) as usize, "zoom {:?}: {off} pixels differ", camera.zoom);
    }
}

#[test]
fn arrows_paint_as_the_reference_does() {
    let gpu = Headless::new().expect("GPU");
    let mut world = world();
    for (i, (size, bob)) in [(24, 0), (32, 100), (48, -150)].into_iter().enumerate() {
        let (x, y) = (24 + 40 * i as i32, 110);
        world.free.push(shape(x, y, marker::shadow(size, (8, 4), bob, [32, 23, 14, 255])));
        world.free.push(shape(x, y, marker::arrow(size, (0, -68), bob, [232, 183, 87, 255])));
    }
    let cameras: Vec<_> = [(1, 1), (2, 1), (3, 2)].into_iter().map(|zoom| Camera { zoom, zoomed_art: true, ..Camera::new((150 * zoom.0 / zoom.1, 130 * zoom.0 / zoom.1)) }).collect();
    // (Their sines and square roots may differ by an ulp, which can move a
    // sample at an edge across it: a sixteenth of a pixel's coverage.)
    compare(&gpu, &world, &cameras, 20);
}
