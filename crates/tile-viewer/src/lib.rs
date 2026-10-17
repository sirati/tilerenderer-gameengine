//! A window that draws a [`scene::Display`]'s scene (every view of it) with a
//! compiled render description, and sends the user's input back.
//!
//! Nothing here knows which game it is showing.

mod constraints;
pub mod cursor;
pub mod fps;
mod input;
pub mod layout;
pub mod looks;
pub mod scaling;
pub mod snap;

use std::sync::Arc;
use std::time::{Duration, Instant};

use cursor::SnapMode;
use render_desc::Compiled;
use scene::{Display, Input, Scene};
use tile_render::{Camera, RgbaImage, TileRenderer, TileWorld};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

#[derive(Clone, Debug)]
pub struct ViewerConfig {
    pub title: String,
    /// Initial inner size in pixels.
    pub size: (u32, u32),
    pub fullscreen: bool,
    pub clear: [u8; 4],
    /// Sprites may extend this far beyond their cell (left, top, right,
    /// bottom), see `tile_render::Camera::overhang`.
    pub overhang: (u32, u32, u32, u32),
    /// On Windows: whether the viewer makes the whole process DPI aware
    /// (winit's default). A viewer in someone else's process leaves that
    /// to the process (its thread made aware on its own instead).
    pub process_dpi_aware: bool,
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self { title: "tile-viewer".into(), size: (1280, 960), fullscreen: false, clear: [0, 0, 0, 255], overhang: (0, 0, 0, 0), process_dpi_aware: true }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ViewerError {
    #[error(transparent)]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error(transparent)]
    Gpu(#[from] tile_render::gpu::GpuError),
    #[error(transparent)]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error(transparent)]
    Os(#[from] winit::error::OsError),
}

/// The compiled description a viewer draws with, replaceable while it runs
/// (e.g. when a game loads a scenario with its own art), and growing
/// (`render_desc::Growth`): each whole description and each growth has a
/// generation (what scenes refer to, `Scene::assets`); a growth keeps what
/// was there, so a scene of an earlier generation of the same whole
/// description is drawn with a later one.
#[derive(Clone)]
pub struct Assets(Arc<std::sync::Mutex<Slot>>);

struct Slot {
    /// The latest whole description's generation, and the description
    /// until the viewer has uploaded it.
    generation: u64,
    whole: Option<Arc<Compiled>>,
    /// Its growths not yet applied by the viewer, in order.
    growths: Vec<(u64, Arc<render_desc::Growth>)>,
    /// The latest generation given (whole or grown).
    latest: u64,
    /// Atlas pages (colour, index) to make room for, if asked (see
    /// [`Assets::reserve`]).
    reserve: Option<(u32, u32)>,
}

impl Assets {
    pub fn new(compiled: Arc<Compiled>) -> Self {
        Self(Arc::new(std::sync::Mutex::new(Slot { generation: 1, whole: Some(compiled), growths: Vec::new(), latest: 1, reserve: None })))
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn replace(&self, compiled: Arc<Compiled>) {
        let generation = self.slot().latest + 1;
        self.set(compiled, generation);
    }

    /// Replaces the description by `compiled` of generation `generation`
    /// (what scenes refer to, `Scene::assets`).
    pub fn set(&self, compiled: Arc<Compiled>, generation: u64) {
        *self.slot() = Slot { generation, whole: Some(compiled), growths: Vec::new(), latest: generation, reserve: None };
    }

    /// Grows the latest description by `growth` (made for it as grown so
    /// far), as generation `generation`.
    pub fn grow(&self, growth: Arc<render_desc::Growth>, generation: u64) {
        let mut slot = self.slot();
        slot.growths.push((generation, growth));
        slot.latest = slot.latest.max(generation);
    }

    /// Asks the viewer to make room in its atlas for `pages` colour and
    /// `index_pages` index pages (art expected soon), so that growing it
    /// does not move it again and again (`TileRenderer::reserve_atlas`).
    pub fn reserve(&self, pages: u32, index_pages: u32) {
        self.slot().reserve = Some((pages, index_pages));
    }

    fn take_reserve(&self) -> Option<(u32, u32)> {
        self.slot().reserve.take()
    }

    /// The latest generation given (whole or grown).
    pub fn latest(&self) -> u64 {
        self.slot().latest
    }

    /// The latest whole description's generation, and the description if
    /// the viewer has not uploaded it yet.
    fn whole(&self) -> (u64, Option<Arc<Compiled>>) {
        let slot = self.slot();
        (slot.generation, slot.whole.clone())
    }

    /// The viewer has uploaded `generation` (its pixels are on the GPU: the
    /// slot lets go of them, unless a newer one came meanwhile).
    fn uploaded(&self, generation: u64) {
        let mut slot = self.slot();
        if slot.generation == generation {
            slot.whole = None;
        }
    }

    /// The growths of whole generation `generation` not applied yet.
    fn take_growths(&self, generation: u64) -> Vec<(u64, Arc<render_desc::Growth>)> {
        let mut slot = self.slot();
        if slot.generation == generation { std::mem::take(&mut slot.growths) } else { Vec::new() }
    }
}

/// Turns a scene into the renderer's world: cells, the world's sprites
/// (drawn in every view) and each view's overlay (drawn in that view, see
/// [`scene_cameras`] for the view order).
pub fn scene_world(compiled: &Compiled, scene: &Scene) -> TileWorld {
    let identity: Vec<Option<u32>> = (0..scene.views.len() as u32).map(Some).collect();
    scene_world_for(compiled, scene, &identity)
}

/// [`scene_world`] for cameras arranged otherwise: scene view `i`'s overlay
/// is drawn by camera `view_cameras[i]` (none: not drawn).
pub fn scene_world_for(compiled: &Compiled, scene: &Scene, view_cameras: &[Option<u32>]) -> TileWorld {
    scene_world_arranged(compiled, scene, view_cameras, &[])
}

/// [`scene_world_for`] with the overlays of some scene views drawn by
/// several cameras: per such view (its index), each overlay sprite's camera
/// (`scaling::Arranged::overlay_cameras`).
pub fn scene_world_arranged(compiled: &Compiled, scene: &Scene, view_cameras: &[Option<u32>], overlay_cameras: &[(usize, Vec<u32>)]) -> TileWorld {
    let s = scene.shape;
    let mut world = compiled.world(s.cols.max(1), s.rows.max(1), s.wrap_x, s.wrap_y);
    let cells = s.cols * s.rows;
    for (i, layer) in scene.layers.iter().enumerate().take(compiled.layers.len()) {
        let id = tile_render::LayerId(i);
        for (cell, &v) in layer.values.iter().enumerate().take(cells as usize) {
            if v != 0 {
                world.set(id, cell as u32 % s.cols, cell as u32 / s.cols, v);
            }
        }
        if let (Some(phases), true) = (&layer.phases, compiled.layers[i].per_cell_phase) {
            for (cell, &p) in phases.iter().enumerate().take(cells as usize) {
                world.set_phase(id, cell as u32 % s.cols, cell as u32 / s.cols, p);
            }
        }
        if let (Some(offsets), true) = (&layer.offsets, compiled.layers[i].per_cell_offset) {
            for (cell, &o) in offsets.iter().enumerate().take(cells as usize).filter(|(_, o)| **o != (0, 0)) {
                world.set_offset(id, cell as u32 % s.cols, cell as u32 / s.cols, o);
            }
        }
    }
    // The camera showing scene view `id`, if one does.
    let camera_of = |id: u32| scene.views.iter().zip(view_cameras).find(|(v, _)| v.id == id).and_then(|(_, c)| *c);
    world.free = scene.sprites.iter().map(|s| placed_sprite(compiled, s, None, s.not_in_view.and_then(camera_of))).collect();
    for (i, (v, camera)) in scene.views.iter().zip(view_cameras).enumerate() {
        match overlay_cameras.iter().find(|(view, _)| *view == i) {
            Some((_, cameras)) => world.free.extend(v.overlay.iter().zip(cameras).map(|(s, c)| placed_sprite(compiled, s, Some(*c), None))),
            None => {
                if let Some(c) = camera {
                    world.free.extend(v.overlay.iter().map(|s| placed_sprite(compiled, s, Some(*c), None)));
                }
            }
        }
    }
    world
}

/// Where [`scene_world_for`] puts each drawn view's overlay among the
/// world's free sprites: (view id, first index, how many, camera).
pub fn overlay_spans(scene: &Scene, view_cameras: &[Option<u32>]) -> Vec<looks::Span> {
    overlay_spans_arranged(scene, view_cameras, &[])
}

/// [`overlay_spans`] with each scaled panel's sprite cameras retained.
pub fn overlay_spans_arranged(scene: &Scene, view_cameras: &[Option<u32>], overlay_cameras: &[(usize, Vec<u32>)]) -> Vec<looks::Span> {
    let mut at = scene.sprites.len();
    let mut out = Vec::new();
    for (i, (v, camera)) in scene.views.iter().zip(view_cameras).enumerate() {
        let cameras = match overlay_cameras.iter().find(|(view, _)| *view == i) {
            Some((_, cameras)) => cameras.iter().copied().take(v.overlay.len()).collect(),
            None => match camera {
                Some(c) => vec![*c; v.overlay.len()],
                None => continue,
            },
        };
        let len = cameras.len();
        out.push(looks::Span { view: v.id, start: at, len, cameras });
        at += len;
    }
    out
}

/// A scene's sprite as the renderer places it: in every view, or only in
/// the one drawn by camera `view` (an overlay's); `not_in`: drawn in every
/// view but the one of that camera.
pub fn placed_sprite(compiled: &Compiled, s: &scene::FreeSprite, view: Option<u32>, not_in: Option<u32>) -> tile_render::PlacedSprite {
    tile_render::PlacedSprite {
        set: tile_render::SpriteSetId::from_raw(s.set),
        x: s.x,
        y: s.y,
        screen: s.screen,
        after_pass: s.after_pass as usize,
        phase: s.phase,
        ticks_per_frame: s.ticks_per_frame.max(1),
        blend: tile_render::Blend::from_scale_den(s.scale_den, s.tint),
        clip: s.clip,
        view: view.or(not_in),
        except_view: view.is_none() && not_in.is_some(),
        zoom_art: match s.zoom_art {
            1 => tile_render::ZoomArt::Low,
            2 => tile_render::ZoomArt::Zoomed { zoom: (u32::from(s.zoom.0), u32::from(s.zoom.1)), offset: s.zoom_offset },
            _ => tile_render::ZoomArt::Any,
        },
        text: s.glyph.and_then(|g| outline_glyph(compiled, &g, s.tint)),
        palette: s.palette,
    }
}

/// A scene's glyph (in `tint`, 0xRRGGBB) as the renderer draws it from its
/// font's outline; `None` if the description has no such font (a
/// character the font lacks shows its first glyph, as fonts draw them).
fn outline_glyph(compiled: &Compiled, g: &scene::Glyph, tint: u32) -> Option<tile_render::OutlineGlyph> {
    let face = &compiled.fonts.get(g.font as usize)?.1;
    let shape = match g.bar {
        Some((top, height)) => tile_render::GlyphShape::Bar { top, height },
        None => tile_render::GlyphShape::Glyph(face.glyph(g.ch).unwrap_or(0)),
    };
    Some(tile_render::OutlineGlyph {
        font: g.font,
        shape,
        em64: g.em64,
        advance: g.advance,
        bold: g.bold,
        italic: g.italic,
        color: [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8],
        key: g.key,
    })
}

/// The cameras of the scene's views, in its view order, 1:1 (a view's
/// `zoom` is that of its zoomed art). A view with an empty viewport fills
/// `window`.
pub fn scene_cameras(scene: &Scene, window: (u32, u32), overhang: (u32, u32, u32, u32)) -> Vec<Camera> {
    scene
        .views
        .iter()
        .map(|v| {
            let v = v.view;
            let viewport = if v.viewport.0 == 0 || v.viewport.1 == 0 { window } else { v.viewport };
            Camera {
                x: v.camera_x,
                y: v.camera_y,
                zoom: (1, 1),
                viewport,
                overhang,
                cull: v.cull,
                origin: v.origin,
                cells: !v.interface_only,
                screen_shift: (0, 0),
                zoomed_art: false,
                stacked: false,
                hairline: false,
                downscale: v.downscale.max(1),
            }
        })
        .collect()
}

/// How many times smaller the scene shows its world: the downscale of its
/// first view showing the world (`scene::View::downscale`), whose pixels
/// are the world pixels the viewer arranges its own views in.
pub fn world_downscale(scene: &Scene) -> u32 {
    scene.views.iter().find(|v| !v.view.interface_only).map_or(1, |v| v.view.downscale.max(1))
}

/// The world's extent in pixels (at the scale its views show it, see
/// [`world_downscale`]) and how it wraps.
fn world_bounds(compiled: &Compiled, scene: &Scene) -> layout::WorldBounds {
    let s = scene.shape;
    let n = world_downscale(scene);
    let step = (compiled.step.0 / n, compiled.step.1 / n);
    let col = match compiled.layout {
        tile_render::Layout::Staggered => 2 * step.0,
        tile_render::Layout::Orthogonal => step.0,
    };
    layout::WorldBounds { size: ((s.cols * col) as i32, (s.rows * step.1) as i32), wrap: (s.wrap_x, s.wrap_y) }
}

/// Sprite sets the viewer draws its markers with, if the description has
/// them (anchored at the point they mark).
pub const CURSOR_SPRITE: &str = "Viewer@cursor/0";
/// The divider's line, light, drawn over the dark gap where it is closed
/// (side by side: a column; stacked: a row); its top-left at the anchor.
pub const DIVIDER_SPRITE: &str = "Viewer@divider_v/0";
pub const DIVIDER_ROW_SPRITE: &str = "Viewer@divider_h/0";
pub const ARROW_SPRITE: &str = "Viewer@arrow/0";

/// The markers as world sprites on top of everything (but the world
/// cursor, which `cursor` draws). The arrow blinks twice every three
/// seconds (`time`: since the viewer started). The markers' points are in
/// the pixels of views showing the world `downscale` times smaller; world
/// sprites are placed at full size.
fn marker_sprites(compiled: &Compiled, markers: &[layout::Marker], downscale: u32, time: Duration) -> Vec<tile_render::PlacedSprite> {
    let blink = time.as_millis() % 3000;
    let arrow_off = blink < 150 || (300..450).contains(&blink);
    markers
        .iter()
        .filter_map(|m| {
            let (name, (x, y)) = match *m {
                layout::Marker::Cursor(_) => return None,
                layout::Marker::Focus(_) if arrow_off => return None,
                layout::Marker::Focus(p) => (ARROW_SPRITE, p),
            };
            let (set, _) = compiled.set(name)?;
            let n = downscale.max(1) as i32;
            Some(tile_render::PlacedSprite::new(set, x * n, y * n))
        })
        .collect()
}

/// The divider's line over its closed stretches (`layout::Frame::divider`):
/// a light line two pixels wide in the middle of the dark four-pixel gap,
/// seen on any background; nothing without the art.
fn divider_sprites(compiled: &Compiled, pieces: &[(u32, [i32; 4], bool)]) -> Vec<tile_render::PlacedSprite> {
    let mut out = Vec::new();
    for &(camera, [x, y, w, h], vertical) in pieces {
        let Some((set, _)) = compiled.set(if vertical { DIVIDER_SPRITE } else { DIVIDER_ROW_SPRITE }) else { continue };
        let clip = Some([x, y, x + w, y + h]);
        let (len, step) = if vertical { (h, 64) } else { (w, 64) };
        for k in (0..len).step_by(step) {
            let (px, py) = if vertical { (x + (w - 2) / 2, y + k) } else { (x + k, y + (h - 2) / 2) };
            out.push(tile_render::PlacedSprite { screen: true, view: Some(camera), clip, ..tile_render::PlacedSprite::new(set, px, py) });
        }
    }
    out
}

/// The window rectangle (x, y, w, h) a screen sprite covers: its first
/// frame (a glyph: its em box), clipped.
fn sprite_rect(compiled: &Compiled, s: &scene::FreeSprite) -> Option<[i32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = match s.glyph {
        Some(g) => {
            let em = (g.em64 as i32 + 63) / 64;
            (s.x, s.y - em, s.x + g.advance.max(1), s.y + em / 3)
        }
        None => {
            let id = compiled.sets.frame(tile_render::SpriteSetId::from_raw(s.set), 0)?;
            let sprite = compiled.atlas.sprites.get(id.0 as usize)?;
            let (x0, y0) = (s.x + sprite.anchor_x, s.y + sprite.anchor_y);
            (x0, y0, x0 + sprite.width as i32, y0 + sprite.height as i32)
        }
    };
    if let Some([cx0, cy0, cx1, cy1]) = s.clip {
        (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// The window rectangles (x, y, w, h) the scene's interface-only views
/// cover: their sprites' first frames, clipped (not the parts the layout
/// may draw elsewhere, `scene::Interface`, which it knows itself).
fn interface_rects(compiled: &Compiled, scene: &Scene) -> Vec<[i32; 4]> {
    let parts = [scene.interface.roaming, scene.interface.popup, scene.interface.screen_view, scene.interface.capture_hint];
    let views = scene.views.iter().filter(|v| v.view.interface_only && !parts.contains(&Some(v.id)));
    views.flat_map(|v| v.overlay.iter().filter(|s| s.glyph.is_none()).filter_map(|s| sprite_rect(compiled, s))).collect()
}

/// Writes run-time sprite pixels into the atlas; patches for sprites the
/// description does not have (yet) wait for one that has.
fn apply_patches<'p>(
    renderer: &TileRenderer,
    queue: &wgpu::Queue,
    compiled: &Compiled,
    patches: impl IntoIterator<Item = &'p scene::Patch>,
) {
    for p in patches {
        let Some(id) = compiled.sprite(&p.sprite) else { continue };
        if p.rgba.len() != (p.width * p.height * 4) as usize {
            continue;
        }
        let image = RgbaImage::from_fn(p.width, p.height, |x, y| {
            let i = ((y * p.width + x) * 4) as usize;
            [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
        });
        if !renderer.patch_sprite(queue, id, &image) {
            log::debug!("viewer: patch for sprite {} does not fit", p.sprite);
        }
    }
}

/// Renders every view of `scene` into an offscreen target of `size` (for
/// tests and pixel comparisons).
pub fn render_offscreen(
    gpu: &tile_render::Headless,
    compiled: &Compiled,
    scene: &Scene,
    size: (u32, u32),
    config: &ViewerConfig,
) -> RgbaImage {
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
    renderer.set_world(&gpu.device, &scene_world(compiled, scene));
    apply_patches(&renderer, &gpu.queue, compiled, &scene.patches);
    gpu.render_views(&mut renderer, &scene_cameras(scene, size, config.overhang), size, scene.tick, config.clear)
}

/// Renders `scene` as a freshly opened viewer arranges it (`layout`; the
/// user's settings left at their defaults) into an offscreen target of
/// `size` (for tools and pixel comparisons).
pub fn render_arranged(
    gpu: &tile_render::Headless,
    compiled: &Compiled,
    scene: &Scene,
    size: (u32, u32),
    config: &ViewerConfig,
) -> RgbaImage {
    let mut layout = layout::Layout::default();
    layout.grid_step = compiled.step;
    let frame = layout.frame(scene, size, config.overhang, interface_rects(compiled, scene), Some(world_bounds(compiled, scene)));
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
    renderer.set_world(&gpu.device, &scene_world_for(compiled, scene, &frame.view_cameras));
    apply_patches(&renderer, &gpu.queue, compiled, &scene.patches);
    gpu.render_views(&mut renderer, &frame.cameras, size, scene.tick, config.clear)
}

/// A device on `adapter` with `surface` configured for a window of `size`
/// (a non-sRGB format, so atlas colours reach the screen unchanged); or the
/// adapter and why not.
async fn show_with(
    surface: &wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    size: winit::dpi::PhysicalSize<u32>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue, wgpu::SurfaceConfiguration), (wgpu::Adapter, String)> {
    let (device, queue) = match tile_render::gpu::request_device(&adapter).await {
        Ok(d) => d,
        Err(e) => return Err((adapter, e.to_string())),
    };
    let caps = surface.get_capabilities(&adapter);
    let Some(&first) = caps.formats.first() else { return Err((adapter, "no surface format".into())) };
    let format = caps.formats.iter().copied().find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm)).unwrap_or(first);
    // Scale-blend passes copy the target, so ask for COPY_SRC.
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
    let config = wgpu::SurfaceConfiguration {
        usage,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        // No frame waits behind another: the cursor drawn here lags as
        // little as it can.
        desired_maximum_frame_latency: 1,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Auto,
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(&device, &config);
    match scope.pop().await {
        Some(e) => Err((adapter, e.to_string())),
        None => Ok((adapter, device, queue, config)),
    }
}

/// Opens the window and runs until it is closed, drawing what `display`
/// receives. Blocks the calling thread, which need not be the process's
/// main thread (the viewer usually runs inside someone else's process).
pub fn run(config: ViewerConfig, assets: Assets, display: Display) -> Result<(), ViewerError> {
    let mut builder = EventLoop::builder();
    #[cfg(windows)]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true).with_dpi_aware(config.process_dpi_aware);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    let event_loop = builder.build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    // The driver's updates wake the event loop (it waits for its window's
    // events, which are taken at once, the pointer's among them).
    let proxy = event_loop.create_proxy();
    display.set_wake(move || {
        let _ = proxy.send_event(());
    });
    let (assets_version, whole) = assets.whole();
    let whole = whole.expect("a new description");
    let mut app = App {
        config,
        assets,
        assets_version,
        whole_version: assets_version,
        compiled: whole.without_pixels(),
        upload: Some(whole),
        display,
        gpu: None,
        drawn_seq: 0,
        patches: std::collections::BTreeMap::new(),
        input: input::InputState::default(),
        layout: layout::Layout::default(),
        drawn_views: Vec::new(),
        world: None,
        drawn_markers: Vec::new(),
        frame_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        drawn_tick: 0,
        drawn_window: (0, 0),
        plan: None,
        drawn_overlay: Vec::new(),
        built_overlay: Vec::new(),
        overlay_camera: None,
        reported_size: (0, 0),
        reported_window: (0, 0, 0),
        sized_to_screen: None,
        fitted_on_screen: None,
        started: std::time::Instant::now(),
        art_waiting: None,
        drawn_frame: None,
        drawn_at: None,
        settings: viewer_settings::Settings::default(),
        error: None,
        snap: snap::SnapInput::default(),
        fps: fps::Overlay::default(),
        snap_grabbed: false,
        cursor_art: cursor::Art::default(),
        custom_cursors: std::collections::HashMap::new(),
        cursor_builds: CursorBuilds::default(),
        shown_pointer: None,
        capture_icon: false,
        pointer_inside: true,
        pointer_since: None,
        drawn_cursor: Vec::new(),
        latency: cursor::Latency::default(),
        latency_logged: Instant::now(),
        frames_presented: 0,
        frame_log: frame_times().then(fps::FrameLog::default),
        beside_drawn: None,
        staged: None,
        unlock_pending: None,
        machine: cursor::Machine::default(),
        system_pointer: (0, 0),
        cursor: scene::Cursor::Default,
        presented: false,
        woken_idle: false,
        cursor_cameras: None,
        next_frame: None,
        map_drawn: false,
        constraints: constraints::Constraints::default(),
        feedback: looks::Feedback::from_env(),
        drawn_looks: Vec::new(),
        base_free: Vec::new(),
        spans: Vec::new(),
        look_scene: looks::LookScene::default(),
    };
    // The arrangement to start with: the user's settings.
    app.set_settings(load_settings());
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// The settings saved at `viewer_settings::Settings::default_path()`; the
/// defaults if there are none or they do not load (logged).
fn load_settings() -> viewer_settings::Settings {
    let path = viewer_settings::Settings::default_path();
    viewer_settings::Settings::load(&path).unwrap_or_else(|e| {
        log::warn!("viewer settings: {e}; using the defaults");
        viewer_settings::Settings::default()
    })
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: TileRenderer,
    /// A software renderer (e.g. WARP, llvmpipe): a new frame only once the
    /// last is done (it would pile up frames and their uploads) and only if
    /// it differs; hardware ones keep the surface's own frame latency.
    software: bool,
    /// The surface was configured anew since the last frame (drawn even if
    /// unchanged: the old picture may be gone).
    reconfigured: bool,
}

struct App {
    config: ViewerConfig,
    assets: Assets,
    /// The generation drawn (the whole description's, or a growth's of
    /// it), and that whole description's.
    assets_version: u64,
    whole_version: u64,
    /// The description as drawn (without the pixels: those are on the GPU).
    compiled: Compiled,
    /// The first description's pixels, until the GPU is there to take them.
    upload: Option<Arc<Compiled>>,
    display: Display,
    gpu: Option<Gpu>,
    /// The scene version the renderer's world was built from.
    drawn_seq: u64,
    /// The latest pixels of every patched sprite (written again into a new
    /// atlas).
    patches: std::collections::BTreeMap<String, scene::Patch>,
    input: input::InputState,
    /// The viewer's own arrangement (zoom, split, insets, magnifier).
    layout: layout::Layout,
    /// Which camera drew each scene view's overlay in the renderer's world.
    drawn_views: Vec<Option<u32>>,
    /// The renderer's world without the markers, and the markers drawn.
    world: Option<TileWorld>,
    drawn_markers: Vec<tile_render::PlacedSprite>,
    /// Whether the last frame submitted is done (see `Gpu::software`).
    frame_done: Arc<std::sync::atomic::AtomicBool>,
    /// The animation tick and window size drawn last.
    drawn_tick: u32,
    drawn_window: (u32, u32),
    /// How the driver's screen is shown in the window
    /// ([`scaling::plan`]), and its size as last told to the driver
    /// (`Input::Resized`).
    plan: Option<scaling::Plan>,
    reported_size: (u32, u32),
    /// The window and the desktop's scale as last told to the driver
    /// (`Input::Window`).
    reported_window: (u32, u32, u32),
    /// Which camera drew each sprite of overlays drawn by several
    /// (`scaling::Arranged::overlay_cameras`).
    drawn_overlay: Vec<(usize, Vec<u32>)>,
    /// The same, as the renderer's world was built.
    built_overlay: Vec<(usize, Vec<u32>)>,
    /// The camera for the viewer's own screen sprites, if the driver's
    /// screen is mapped onto the picture (`scaling::Arranged::overlay`).
    overlay_camera: Option<u32>,
    /// The driver's screen and the desktop's scale a window (not
    /// fullscreen) last asked to be sized for (asked again when either
    /// changes, e.g. the game switching its screen as it starts).
    sized_to_screen: Option<((u32, u32), u64)>,
    /// The window size (and `sized_to_screen` key) last checked for being on
    /// its screen.
    fitted_on_screen: Option<((u32, u32), ((u32, u32), u64))>,
    started: std::time::Instant,
    /// Since when newer art is waiting for a scene that refers to it.
    art_waiting: Option<std::time::Instant>,
    /// The arrangement drawn last (drawn again while a scene waits for its
    /// art).
    drawn_frame: Option<layout::Frame>,
    /// When the last frame was drawn (the layout's smooth movement).
    drawn_at: Option<std::time::Instant>,
    error: Option<ViewerError>,
    /// The viewer's own settings (resolution, layout, input): read from
    /// `viewer_settings::Settings::default_path()` at start, replaced by those
    /// the driver sends (`scene::Update::Settings`; the driver keeps and saves
    /// them, e.g. edited on its preferences screen).
    settings: viewer_settings::Settings,
    /// Button snapping (a mode Alt turns on and off), and whether it holds
    /// the mouse itself (the mouse not captured).
    snap: snap::SnapInput,
    snap_grabbed: bool,
    /// The frame counter (`settings.display.show_fps`).
    fps: fps::Overlay,
    /// The cursor art kept for the system pointer, the system cursors made
    /// of it (by set), what the system pointer shows, and whether it is the
    /// capture pointer (the layout's `PointerIcon::Capture`).
    cursor_art: cursor::Art,
    custom_cursors: std::collections::HashMap<(u32, u32), winit::window::CustomCursor>,
    /// System cursors being made (off the event loop: the picture never
    /// waits for one).
    cursor_builds: CursorBuilds,
    shown_pointer: Option<cursor::Shown>,
    capture_icon: bool,
    /// Whether the system pointer is in the window.
    pointer_inside: bool,
    /// The first pointer motion not drawn yet, the cursor sprites drawn
    /// last, and how long they took to show (`TILE_VIEWER_LATENCY` logs it).
    pointer_since: Option<Instant>,
    drawn_cursor: Vec<tile_render::PlacedSprite>,
    latency: cursor::Latency,
    latency_logged: Instant,
    frames_presented: u32,
    /// Frame times for the log (`TILE_VIEWER_FRAME_TIMES`), and the world
    /// last drawn beside a screen (`scene::Beside`, for the log).
    frame_log: Option<fps::FrameLog>,
    beside_drawn: Option<(scene::WorldShape, Option<[i32; 4]>)>,
    /// New art on its way to the GPU (see `ATLAS_PART`).
    staged: Option<Staged>,
    /// What happens to the system pointer after the next frame (see
    /// `unlock_after_frame`).
    unlock_pending: Option<Unlock>,
    /// The cursor's state (`cursor::Machine`), and where the system pointer
    /// is (window pixels, as the window last reported it or put it).
    machine: cursor::Machine,
    system_pointer: (i32, i32),
    /// The driver's cursor (see `driver_cursor`).
    cursor: scene::Cursor,
    /// A frame was presented since `release_after_frame` last looked.
    presented: bool,
    /// The event loop was woken without an event of its own (see
    /// `about_to_wait`).
    woken_idle: bool,
    /// The cameras of the frame drawn last (for the latency: the motion
    /// moved them).
    cursor_cameras: Option<Vec<Camera>>,
    /// When the next frame is due (the animation clock), without updates.
    next_frame: Option<Instant>,
    /// Whether a scene with the world in a view was drawn yet.
    map_drawn: bool,
    /// The system's pointer constraints (Wayland).
    constraints: constraints::Constraints,
    /// The interaction looks drawn here (hover, pressed, selected), and
    /// those the last frame drew.
    feedback: looks::Feedback,
    drawn_looks: Vec<(u64, scene::LookState)>,
    /// The world's free sprites as built (no looks drawn), where the
    /// views' overlays lie among them, and the interactables and looks of
    /// the scene it was built from.
    base_free: Vec<tile_render::PlacedSprite>,
    spans: Vec<looks::Span>,
    look_scene: looks::LookScene,
}

/// What happens to the system pointer once a frame went out after it was
/// moved (on Wayland a captured pointer is moved by a hint the window's
/// next frame applies when it is let go).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Unlock {
    /// The mouse is let go.
    Release,
    /// It shows at the world cursor, kept inside the window, moving it
    /// (`cursor::Machine::edge`).
    Edge,
}

/// Captures the mouse again (locked where the system can, else kept in the
/// window); whether it was locked.
fn layout_captured_grab(window: &Window) -> bool {
    use winit::window::CursorGrabMode;
    if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
        return true;
    }
    let _ = window.set_cursor_grab(CursorGrabMode::Confined);
    false
}

/// The longest time between frames (the animation clock).
const FRAME: Duration = Duration::from_millis(16);

/// How long the event loop waits for the driver when it was woken for
/// nothing (see `about_to_wait`).
const IDLE_WAIT: Duration = Duration::from_millis(1);
/// About how many bytes of new art go up to the GPU a frame (at least a
/// page; a colour page of 2048² is 16 MiB).
const ATLAS_PART: u64 = 16 << 20;

/// New art being uploaded (`TileRenderer::stage_atlas`), and since when, in
/// how many frames.
struct Staged {
    version: u64,
    atlas: tile_render::StagedAtlas,
    compiled: Arc<Compiled>,
    since: Instant,
    frames: u32,
}

/// Whether frame times are logged (`TILE_VIEWER_FRAME_TIMES`, see `fps::FrameLog`).
fn frame_times() -> bool {
    std::env::var("TILE_VIEWER_FRAME_TIMES").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Whether to log the cursor's latency (`TILE_VIEWER_LATENCY=1`).
fn log_latency() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TILE_VIEWER_LATENCY").is_ok_and(|v| !v.is_empty() && v != "0"))
}

impl App {
    /// Takes the user's settings: the layout is arranged as they say, then
    /// `TILE_VIEWER_LAYOUT` (a spec, `layout::Layout::configure`) on top,
    /// for testing.
    fn set_settings(&mut self, settings: viewer_settings::Settings) {
        self.layout.apply_settings(&settings.layout);
        self.layout.set_map_movement(settings.input.map_movement);
        if let Ok(spec) = std::env::var("TILE_VIEWER_LAYOUT") {
            if let Err(e) = self.layout.configure(&spec) {
                log::warn!("TILE_VIEWER_LAYOUT: {e}");
            }
        }
        // The counter counts while it shows, afresh each time it is shown.
        if settings.display.show_fps && !self.settings.display.show_fps {
            self.fps = fps::Overlay::default();
        }
        self.settings = settings;
    }

    /// Input for the driver: also what the interaction looks follow (a
    /// frame is drawn at once when they may change).
    fn send(&mut self, input: Input) {
        let now = self.now();
        if self.feedback.sent(&input, &self.look_scene, now) {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
        self.display.send_input(input);
    }

    /// Seconds since the viewer started (the snapping pointer's clock).
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether the viewer draws the cursor itself rather than the system:
    /// while the mouse is captured, or the pointer snaps to buttons.
    fn pointer_hidden(&self) -> bool {
        (self.layout.captured() && !self.machine.edge()) || self.snap.snap_mode_active() || self.unlock_pending.is_some() || self.constraints.moving()
    }

    /// The driver's cursor, as long as its scene refers to the art the
    /// renderer has (its sprite set ids are that art's), else the last one
    /// that did.
    fn driver_cursor(&mut self) -> scene::Cursor {
        let scene = self.display.scene();
        if scene.assets == 0 || scene.assets == self.assets_version {
            self.cursor = scene.cursor;
        }
        self.cursor
    }

    /// Shows the system pointer as it is to be now (`cursor`): hidden
    /// while the viewer draws the cursor, else the driver's cursor (made
    /// from its art), or the capture pointer.
    fn update_system_pointer(&mut self, event_loop: &ActiveEventLoop) {
        // (Button snapping says when it hides the pointer; whether it is
        // hidden is asked anew every time.)
        let _ = self.snap.take_hide_pointer();
        // System cursors made since (each asked for again).
        for (key, source) in self.cursor_builds.take_ready() {
            self.custom_cursors.insert(key, event_loop.create_custom_cursor(source));
            self.shown_pointer = None;
        }
        let shown = cursor::system_pointer(self.driver_cursor(), &self.cursor_art, self.pointer_hidden(), self.capture_icon);
        let Some(gpu) = &self.gpu else { return };
        if self.shown_pointer == Some(shown) {
            return;
        }
        self.shown_pointer = Some(shown);
        log::debug!("viewer: system pointer {shown:?}");
        match shown {
            cursor::Shown::Hidden => gpu.window.set_cursor_visible(false),
            cursor::Shown::Icon(icon) => {
                gpu.window.set_cursor(icon);
                gpu.window.set_cursor_visible(true);
            }
            cursor::Shown::Custom(set) => {
                // At the pixel scale: each of its pixels an n×n box, as the
                // picture's (a custom cursor's pixels are the window's
                // physical ones, on Wayland too). Over menus or the frame
                // at a scale of their own it stays at the pixel scale.
                let n = self.plan.map_or(1, |p| p.present.zoom.0.max(1) / p.present.zoom.1.max(1)).clamp(1, 8);
                match self.custom_cursors.get(&(set, n)) {
                    Some(c) => gpu.window.set_cursor(c.clone()),
                    None => match self.cursor_art.get(set) {
                        // Made on a thread of its own; until it is, the
                        // pointer stays as it is (asked again next frame).
                        Some(image) => {
                            self.cursor_builds.start((set, n), image.clone());
                            self.shown_pointer = None;
                        }
                        None => gpu.window.set_cursor(winit::window::CursorIcon::Default),
                    },
                }
                gpu.window.set_cursor_visible(true);
            }
        }
    }

    /// What the layout asks of the window: the mouse captured or let go,
    /// the system pointer moved (held at the divider, or put where the world
    /// cursor was), another pointer shape. The pointer never shows where it
    /// is moved from: it is hidden before the mouse is captured, and moved
    /// before it shows again (while still captured: on Wayland the place
    /// is a hint the release takes).
    fn apply_layout_requests(&mut self, event_loop: &ActiveEventLoop) {
        let grab = self.layout.take_grab();
        let warp = self.layout.take_warp();
        if let Some(icon) = self.layout.take_pointer_icon() {
            // A click captures the mouse there.
            self.capture_icon = icon == layout::PointerIcon::Capture;
        }
        if grab == Some(true) {
            self.update_system_pointer(event_loop);
            self.grab(true);
        }
        // (The layout's pixels are the driver's: in the window's.)
        if let (Some(p), Some(gpu)) = (warp, &self.gpu) {
            let (x, y) = self.plan.map_or(p, |plan| plan.to_window(p));
            if self.constraints.move_pointer(&gpu.window, (x, y)) {
                log::debug!("viewer: moving the pointer to {x}, {y} (lock and hint)");
                // Input follows it after the compositor activates the lock.
            } else {
                match gpu.window.set_cursor_position(winit::dpi::PhysicalPosition::new(x, y)) {
                    Ok(()) => self.pointer_placed((x, y)),
                    Err(e) => {
                        log::debug!("viewer: moving the pointer to {x}, {y}: {e}");
                        // (The divider cannot hold back what it cannot move.)
                        self.layout.pointer_cannot_be_moved();
                        self.pointer_placement_failed();
                    }
                }
            }
        }
        if grab == Some(false) {
            if warp.is_some() {
                // (On Wayland the place is applied with the window's next
                // frame: the mouse is let go after it, `unlock_after_frame`.)
                self.unlock_pending = Some(Unlock::Release);
                self.presented = false;
                if let Some(gpu) = &self.gpu {
                    gpu.window.request_redraw();
                }
            } else {
                self.unlock_pending = None;
                self.grab(false);
            }
        }
        // Button snapping with the mouse not captured holds the mouse itself
        // (its motion steers the snapping pointer, the system pointer stays
        // hidden in the window); let go, the system pointer shows where the
        // snapping pointer ended (moved there above).
        let snap_grab = self.snap.snap_mode_active() && !self.layout.captured();
        if snap_grab != self.snap_grabbed {
            self.snap_grabbed = snap_grab;
            if snap_grab {
                if self.unlock_pending == Some(Unlock::Release) {
                    self.unlock_pending = None;
                }
                self.update_system_pointer(event_loop);
                self.grab(true);
            } else if !self.layout.captured() && grab != Some(true) {
                if warp.is_some() {
                    self.unlock_pending = Some(Unlock::Release);
                    self.presented = false;
                    if let Some(gpu) = &self.gpu {
                        gpu.window.request_redraw();
                    }
                } else {
                    self.grab(false);
                }
            }
        }
        self.update_system_pointer(event_loop);
        if let Some(gpu) = &self.gpu {
            match self.constraints.update(&gpu.window, &mut self.layout, self.plan.map(|p| p.present), self.pointer_inside) {
                Some(constraints::Placement::Hinted(at)) => self.pointer_placed(at),
                Some(constraints::Placement::Cancelled) => self.pointer_placement_failed(),
                None => {}
            }
        }
    }

    fn pointer_placed(&mut self, (x, y): (i32, i32)) {
        // A Wayland hint can move the pointer without CursorMoved. Clicks
        // and wheels must use its new position once the lock is released.
        let at = (x as f64, y as f64);
        self.system_pointer = self.input.place_pointer(at);
        self.constraints.pointer_moved(at);
        self.reconcile_system_pointer();
    }

    fn pointer_placement_failed(&mut self) {
        // Use the latest physical motion, including any received while the
        // move was pending. Do not replace a captured or snapping pointer.
        self.system_pointer = self.input.pointer();
        self.reconcile_system_pointer();
    }

    fn reconcile_system_pointer(&mut self) {
        if self.pointer_inside && self.gpu.as_ref().is_some_and(|gpu| gpu.window.has_focus()) && !self.layout.captured() && !self.snap.snap_mode_active() {
            for input in self.layout.system_pointer_placed(self.system_pointer) {
                self.send(input);
            }
        }
    }

    /// Once a frame went out after the system pointer was moved (see
    /// `Unlock`): lets go of the mouse, or keeps the pointer inside the
    /// window only (the edge mode); the pointer shows then.
    fn unlock_after_frame(&mut self) {
        if !std::mem::take(&mut self.presented) {
            return;
        }
        match self.unlock_pending.take() {
            Some(Unlock::Release) => self.grab(false),
            Some(Unlock::Edge) => {
                if let Some(gpu) = &self.gpu {
                    let _ = gpu.window.set_cursor_grab(winit::window::CursorGrabMode::Confined);
                }
            }
            None => {}
        }
    }

    /// The pointer moved: if the viewer draws the cursor, a frame at once.
    fn pointer_moved(&mut self) {
        self.pointer_since.get_or_insert_with(Instant::now);
        if self.pointer_hidden() || !self.drawn_cursor.is_empty() {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ViewerError> {
        // A window: as large as the screen allows from the start (the
        // driver's screen is the desktop's at its scale; the compositor
        // places a new window in the middle, but may not move it again).
        let mut size = winit::dpi::PhysicalSize::new(self.config.size.0, self.config.size.1);
        // (Wayland has no primary monitor: the first one.)
        let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
        if let Some(m) = monitor.map(|m| m.size()).filter(|m| m.width > 0 && m.height > 0) {
            size = within_screen(m, m);
        }
        let mut attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(size)
            .with_resizable(true);
        if self.config.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        // (Shown once it has its size: a system placing a new window where
        // it first shows, e.g. Wine's on Wayland at a default size, places
        // it for that size.)
        let window = Arc::new(event_loop.create_window(attrs.with_visible(false))?);
        self.constraints.attach(&window);
        let instance = wgpu::Instance::new(tile_render::gpu::instance_descriptor());
        let surface = instance.create_surface(window.clone())?;
        let size = window.inner_size();
        // The window's size is in physical pixels (winit follows the
        // compositor's scale, `wl_output` / fractional scale on Wayland),
        // and so is the surface: the picture reaches the screen 1:1.
        log::info!(
            "viewer: window {}×{} pixels at scale {} ({}×{} logical)",
            size.width,
            size.height,
            window.scale_factor(),
            (size.width as f64 / window.scale_factor()).round(),
            (size.height as f64 / window.scale_factor()).round()
        );
        // The adapter asked for (`TILE_RENDER_ADAPTER`); if it cannot show
        // this window after all (e.g. a GPU that does not drive the
        // display), the default one for it.
        let shown = pollster::block_on(async {
            let adapter = tile_render::gpu::select_adapter(&instance, Some(&surface), None).await?;
            match show_with(&surface, adapter, size).await {
                Ok(shown) => Ok(shown),
                Err((adapter, e)) => {
                    let name = adapter.get_info().name;
                    log::error!("viewer: {name} cannot show this window ({e}); trying the default adapter for it");
                    let adapter = tile_render::gpu::default_adapter(&instance, Some(&surface)).await?;
                    show_with(&surface, adapter, size).await.map_err(|(a, e)| {
                        log::error!("viewer: {} cannot show this window either ({e})", a.get_info().name);
                        ViewerError::Gpu(tile_render::gpu::GpuError::NoAdapter(Some(name)))
                    })
                }
            }
        })?;
        let (adapter, device, queue, config) = shown;
        log::info!("viewer adapter: {:?}", adapter.get_info());
        let mut renderer = TileRenderer::new(&device, config.format);
        if let Some(whole) = self.upload.take() {
            renderer.set_atlas(&device, &queue, &whole.atlas);
            self.cursor_art = cursor::Art::of(&whole);
        }
        self.assets.uploaded(self.whole_version);
        self.display.send_input(Input::Resized { width: config.width, height: config.height });
        let software = adapter.get_info().device_type == wgpu::DeviceType::Cpu;
        if software {
            log::info!("viewer: software renderer, one frame in flight, unchanged frames skipped");
        }
        window.set_visible(true);
        self.gpu = Some(Gpu { window, surface, device, queue, config, renderer, software, reconfigured: true });
        self.update_plan();
        Ok(())
    }

    /// Follows the window's size, the driver's screen and the resolution
    /// settings ([`scaling::plan`]): tells the driver the size of its screen
    /// as the viewer arranges it when that changes, and maps the pointer
    /// accordingly.
    fn update_plan(&mut self) {
        let Some(gpu) = &self.gpu else { return };
        let window = (gpu.config.width, gpu.config.height);
        let screen = self.display.scene().screen_size;
        // A window: as large as the driver's screen at the desktop's scale,
        // if the compositor lets it.
        let scale = gpu.window.scale_factor();
        let key = (screen, (scale * 1000.0).round() as u64);
        if !self.config.fullscreen && self.sized_to_screen != Some(key) && screen.0 > 0 && screen.1 > 0 {
            self.sized_to_screen = Some(key);
            let mut want = winit::dpi::PhysicalSize::new((screen.0 as f64 * scale).round() as u32, (screen.1 as f64 * scale).round() as u32);
            // (No larger than the screen it is on, with room for its frame:
            // nine tenths of it at most, the same shape.)
            if let Some(m) = gpu.window.current_monitor().map(|m| m.size()).filter(|m| m.width > 0 && m.height > 0) {
                want = within_screen(want, m);
            }
            log::info!("viewer: asking for a {}×{} window (the driver's {}×{} screen at scale {scale})", want.width, want.height, screen.0, screen.1);
            let _ = gpu.window.request_inner_size(want);
            // In the middle of its screen (where a system lets windows place
            // themselves; Wayland's compositors place them).
            if let Some(m) = gpu.window.current_monitor() {
                let (at, size) = (m.position(), m.size());
                let x = at.x + (size.width as i32 - want.width as i32) / 2;
                let y = at.y + (size.height as i32 - want.height as i32) / 2;
                gpu.window.set_outer_position(winit::dpi::PhysicalPosition::new(x.max(at.x), y.max(at.y)));
            }
        }
        // A window the system placed partly off its screen (e.g. Wine's
        // windows on Wayland, placed at a first size, not moved after): as
        // large as fits where it is, and moved into the middle where the
        // system lets it.
        if !self.config.fullscreen && self.fitted_on_screen != Some((window, key)) {
            self.fitted_on_screen = Some((window, key));
            if let (Ok(at), Some(m)) = (gpu.window.outer_position(), gpu.window.current_monitor()) {
                let (outer, (mp, ms)) = (gpu.window.outer_size(), (m.position(), m.size()));
                let over = |at: i32, len: u32, from: i32, side: u32| (at + len as i32 - (from + side as i32)).max(0) as u32;
                let (ox, oy) = (over(at.x, outer.width, mp.x, ms.width), over(at.y, outer.height, mp.y, ms.height));
                if ox > 0 || oy > 0 {
                    let inner = gpu.window.inner_size();
                    let fits = winit::dpi::PhysicalSize::new(inner.width.saturating_sub(ox).max(160), inner.height.saturating_sub(oy).max(120));
                    log::info!("viewer: the window is {ox}×{oy} pixels off its screen: {}×{} instead", fits.width, fits.height);
                    let _ = gpu.window.request_inner_size(fits);
                    let frame = (outer.width - inner.width, outer.height - inner.height);
                    let x = mp.x + (ms.width as i32 - (fits.width + frame.0) as i32) / 2;
                    let y = mp.y + (ms.height as i32 - (fits.height + frame.1) as i32) / 2;
                    gpu.window.set_outer_position(winit::dpi::PhysicalPosition::new(x.max(mp.x), y.max(mp.y)));
                }
            }
        }
        // The desktop's scale (pixel scale Auto): the window's, or a native
        // viewer's launcher's (`CIV3R_SCALE`, which sizes the game too; a
        // viewer in a Wine program has its window in Wine's own pixels,
        // which Wine scales for the desktop itself).
        let launcher = std::env::var("CIV3R_SCALE").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 1.0);
        let desktop = if cfg!(windows) { scale } else { launcher.unwrap_or(scale) };
        let native = (window.0, window.1, (desktop * 1000.0).round() as u32);
        if native != self.reported_window {
            self.reported_window = native;
            self.display.send_input(Input::Window { width: native.0, height: native.1, scale_milli: native.2 });
        }
        let plan = scaling::plan(window, desktop, screen, &self.settings.resolution);
        if Some(plan) != self.plan {
            let p = plan.present;
            log::info!(
                "viewer: a {}×{} picture shown {}×{} in the {}×{} window at {:?} (pixel scale {}); the driver's screen {}×{}{}",
                p.canvas.0,
                p.canvas.1,
                p.zoom.0,
                p.zoom.0,
                window.0,
                window.1,
                p.at,
                self.settings.resolution.describe_pixel_scale(desktop),
                plan.screen.0,
                plan.screen.1,
                if plan.menu.zoom != (1, 1) || plan.frame != (1, 1) || plan.world != (1, 1) {
                    format!("; menus {}/{}, frame {}/{}, world {}/{}", plan.menu.zoom.0, plan.menu.zoom.1, plan.frame.0, plan.frame.1, plan.world.0, plan.world.1)
                } else {
                    String::new()
                }
            );
            // (The system pointer's art at the new pixel scale.)
            if self.plan.map(|p| p.present.zoom) != Some(plan.present.zoom) {
                self.shown_pointer = None;
            }
            self.plan = Some(plan);
            self.layout.world_scale = plan.world;
            if let Some(moved) = self.input.set_plan(plan) {
                for input in self.layout.route(moved) {
                    self.display.send_input(input);
                }
            }
        }
        if plan.screen != self.reported_size {
            self.reported_size = plan.screen;
            self.display.send_input(Input::Resized { width: plan.screen.0, height: plan.screen.1 });
        }
    }

    /// Captures (holds) or releases the mouse (the pointer is hidden and
    /// shown by `update_system_pointer`).
    fn grab(&mut self, lock: bool) {
        let Some(gpu) = &self.gpu else { return };
        // (Before winit's: the pointer takes one constraint at a time.)
        self.constraints.winit_grab(lock);
        use winit::window::CursorGrabMode;
        if lock {
            // Locked where the system can, else kept inside the window.
            if gpu.window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                let _ = gpu.window.set_cursor_grab(CursorGrabMode::Confined);
            }
        } else {
            let _ = gpu.window.set_cursor_grab(CursorGrabMode::None);
        }
    }

    fn draw(&mut self) {
        // (Measuring: frames that take long.)
        let began = std::time::Instant::now();
        self.draw_frame();
        let took = began.elapsed();
        if took >= Duration::from_millis(40) && self.map_drawn {
            log::info!("viewer: a frame took {} ms", took.as_millis());
        }
    }

    fn draw_frame(&mut self) {
        let started = Instant::now();
        self.update_plan();
        let Some(plan) = self.plan else { return };
        let Some(gpu) = &mut self.gpu else { return };
        // A software renderer still at the last frame: no new one yet (the
        // event loop goes on meanwhile).
        let done = || self.frame_done.load(std::sync::atomic::Ordering::Acquire);
        if gpu.software && !done() {
            let _ = gpu.device.poll(wgpu::PollType::Poll);
            // The cursor drawn here waits for nothing but that frame.
            let cursor_waits = self.pointer_since.is_some() && (self.layout.captured() || self.snap.snap_mode_active() || !self.drawn_cursor.is_empty());
            if !done() && cursor_waits {
                let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_millis(100)) });
            }
            if !done() {
                return;
            }
        }
        let (version, whole) = self.assets.whole();
        // A new whole description goes up to the GPU a part a frame
        // (`ATLAS_PART`), frames drawn with the art before meanwhile (nothing
        // waits for the GPU); a newer one replaces it on the way.
        if let Some(c) = whole.as_ref().filter(|_| version != self.whole_version && self.staged.as_ref().is_none_or(|s| s.version != version)) {
            self.staged = Some(Staged { version, atlas: gpu.renderer.stage_atlas(&gpu.device, &c.atlas), compiled: c.clone(), since: Instant::now(), frames: 0 });
        }
        drop(whole);
        let uploaded = match &mut self.staged {
            Some(s) if s.version == version => {
                s.frames += 1;
                gpu.renderer.upload_staged(&gpu.queue, &mut s.atlas, &s.compiled.atlas, ATLAS_PART)
            }
            _ => false,
        };
        // It is taken, once all of it is there, together with a scene that
        // refers to it (sprite ids change with every compile: taken before,
        // the scenes drawn till then would show other sprites); growths of
        // the one drawn at once (they keep what was there).
        let refers_to = self.display.scene().assets;
        let drawable = |from: u64, to: u64| refers_to == 0 || (from..=to).contains(&refers_to);
        let waits = self.art_waiting.get_or_insert_with(std::time::Instant::now).elapsed();
        let gave_up = waits > Duration::from_secs(3);
        let new_assets = version != self.whole_version && uploaded && (refers_to == 0 || refers_to >= version);
        if let Some(s) = new_assets.then(|| self.staged.take()).flatten() {
            let started = std::time::Instant::now();
            gpu.renderer.adopt_atlas(&gpu.device, s.atlas, &s.compiled.atlas);
            // (Set ids change with every compile.)
            self.cursor_art = cursor::Art::of(&s.compiled);
            self.custom_cursors.clear();
            self.cursor_builds.forget();
            self.shown_pointer = None;
            log::info!(
                "viewer: new assets ({} sprites, uploaded over {} ms in {} frames, taken in {} ms)",
                s.compiled.sprite_names.len(),
                s.since.elapsed().as_millis(),
                s.frames,
                started.elapsed().as_millis()
            );
            // The pixels are on the GPU now: keep only the rest.
            self.compiled = s.compiled.without_pixels();
            self.assets.uploaded(s.version);
            self.whole_version = s.version;
            self.assets_version = s.version;
        }
        if let Some((pages, index_pages)) = self.assets.take_reserve() {
            let started = std::time::Instant::now();
            gpu.renderer.reserve_atlas(&gpu.device, &gpu.queue, pages, index_pages);
            let (rgba, index) = gpu.renderer.atlas_room();
            log::info!("viewer: atlas room for {rgba} colour and {index} index pages made in {} ms", started.elapsed().as_millis());
        }
        let mut grown = false;
        if version == self.whole_version {
            let growths = self.assets.take_growths(version);
            if !growths.is_empty() {
                let started = std::time::Instant::now();
                let (mut sprites, mut bytes) = (0, 0);
                for (generation, growth) in growths {
                    self.compiled.apply(&growth);
                    gpu.renderer.grow_atlas(&gpu.device, &gpu.queue, &growth.atlas);
                    sprites += growth.atlas.sprites.len();
                    bytes += growth.atlas.bytes();
                    self.assets_version = generation;
                }
                let (rgba, index) = gpu.renderer.atlas_room();
                log::info!(
                    "viewer: grown by {sprites} sprites ({} KiB of pixels) in {} ms; {} sprites, atlas room for {rgba} colour and {index} index pages",
                    bytes >> 10,
                    started.elapsed().as_millis(),
                    self.compiled.atlas.sprites.len(),
                );
                grown = true;
            }
        }
        if version == self.whole_version && drawable(self.whole_version, self.assets_version) {
            self.art_waiting = None;
        }
        let new_assets = new_assets || grown;
        for p in self.display.take_patches() {
            self.patches.insert(p.sprite.clone(), p);
        }
        let scene = self.display.scene();
        if scene.seq == 0 {
            return;
        }
        // The picture (the viewer arranges and draws in its pixels; the
        // driver's interface goes onto it, `scaling`).
        let window = plan.canvas();
        // A scene for other art than the renderer has waits: the last frame
        // is drawn meanwhile as it was, its world in its arrangement (the
        // waiting scene's views may differ, e.g. a screen opened with a view
        // of its own).
        let matches = scene.assets == 0 || (self.whole_version..=self.assets_version).contains(&scene.assets) || gave_up;
        let arranged = match &self.drawn_frame {
            Some(frame) if !matches && self.world.is_some() && window == self.drawn_window => frame.clone(),
            _ => {
                let now = std::time::Instant::now();
                if let Some(at) = self.drawn_at.replace(now) {
                    self.layout.advance(now - at);
                }
                let bounds = world_bounds(&self.compiled, scene);
                self.layout.grid_step = self.compiled.step;
                self.layout.driver_screen = Some(plan.screen);
                let mut arranged = self.layout.frame(scene, window, self.config.overhang, interface_rects(&self.compiled, scene), Some(bounds));
                let compiled = &self.compiled;
                let shown = scaling::arrange(&plan, scene, &mut arranged, |s| sprite_rect(compiled, s));
                self.layout.interface_shown = shown.shown;
                self.drawn_overlay = shown.overlay_cameras;
                self.overlay_camera = shown.overlay;
                self.drawn_frame = Some(arranged.clone());
                arranged
            }
        };
        // Views following something move without input: tell the driver.
        if let Some((x, y, w, h)) = self.layout.take_main_view_change() {
            self.display.send_input(Input::MainView { x, y, w, h });
        }
        if let Some(views) = self.layout.take_views_change() {
            log::debug!("viewer: own views now show {views:?}");
            self.display.send_input(Input::Views(views));
        }
        let views_moved = arranged.view_cameras != self.drawn_views || self.drawn_overlay != self.built_overlay;
        let rebuilt = new_assets || ((scene.seq != self.drawn_seq || views_moved) && matches) || self.world.is_none();
        if rebuilt {
            let world = scene_world_arranged(&self.compiled, scene, &arranged.view_cameras, &self.drawn_overlay);
            self.base_free = world.free.clone();
            self.spans = overlay_spans_arranged(scene, &arranged.view_cameras, &self.drawn_overlay);
            self.look_scene = looks::LookScene::of(scene);
            self.world = Some(world);
            self.built_overlay = self.drawn_overlay.clone();
        }
        // The interaction looks drawn here, in the world as built (from the
        // scene it was built from, also while a newer one waits for its
        // art).
        let now = self.started.elapsed().as_secs_f64();
        let replacements = self.feedback.replacements(&self.look_scene, now);
        let looks_changed = rebuilt || self.feedback.drawn() != &self.drawn_looks[..];
        if looks_changed {
            if let Some(world) = self.world.as_mut() {
                let compiled = &self.compiled;
                world.free = looks::splice(&self.base_free, &self.spans, &replacements, |s, camera| placed_sprite(compiled, s, Some(camera), None));
            }
        }
        self.drawn_looks = self.feedback.drawn().to_vec();
        drop(replacements);
        // The viewer's markers on top of the world, for this frame only.
        let mut markers = marker_sprites(&self.compiled, &arranged.markers, world_downscale(scene), self.started.elapsed());
        markers.extend(divider_sprites(&self.compiled, &arranged.divider));
        // The cursor where the viewer draws it, in a camera of its own on
        // top (only when there is one: the frame is as before otherwise).
        let captured = self.layout.captured();
        let facts = cursor::Facts {
            captured,
            snap_mode: self.snap.snap_mode_active(),
            unlocking: self.unlock_pending.is_some(),
            world_cursor: arranged.markers.iter().filter_map(|m| if let layout::Marker::Cursor(p) = m { Some(*p) } else { None }).collect(),
            cursor_view: self.layout.world_cursor_view(),
            pointer: if captured { self.system_pointer } else { self.layout.pointer_in_window() },
            pointer_inside: self.pointer_inside,
            pointer_world: self.layout.pointer_world(),
            now: self.started.elapsed().as_secs_f64(),
        };
        let steps = self.machine.step(&facts);
        let marks = steps.marks;
        if let Some(q) = steps.enter_edge {
            // The view following the world cursor is held at the world's
            // edge: the system pointer goes where the drawn cursor is and
            // shows there once this frame is out (`unlock_after_frame`).
            let w = plan.to_window(q);
            match gpu.window.set_cursor_position(winit::dpi::PhysicalPosition::new(w.0, w.1)) {
                Ok(()) => {
                    // Like pointer_placed, without borrowing the whole App
                    // while this frame still holds the GPU mutably.
                    let at = (w.0 as f64, w.1 as f64);
                    self.system_pointer = self.input.place_pointer(at);
                    self.constraints.pointer_moved(at);
                }
                Err(e) => {
                    log::debug!("viewer: moving the pointer to the world cursor: {e}");
                    self.system_pointer = q;
                }
            }
            self.unlock_pending = Some(Unlock::Edge);
            self.presented = false;
            log::info!("viewer: world cursor at the world's edge: the system pointer shows it at {q:?}");
        }
        if steps.leave_edge {
            // Centred again: captured and hidden (at once; this frame
            // draws the cursor where the pointer is).
            let locked = layout_captured_grab(&gpu.window);
            log::info!("viewer: world cursor centred again: drawn, the system pointer captured (locked: {locked})");
        }
        let mut cameras = arranged.cameras.clone();
        if scene.assets == 0 || scene.assets == self.assets_version {
            self.cursor = scene.cursor;
        }
        let cursor_sprites = match cursor::sprite_set(&self.compiled, self.cursor) {
            Some(set) if cameras.len() < tile_render::MAX_VIEWS => {
                let bounds = world_bounds(&self.compiled, scene);
                let period = (if bounds.wrap.0 { bounds.size.0 } else { 0 }, if bounds.wrap.1 { bounds.size.1 } else { 0 });
                cursor::sprites(set, &marks, &cameras, cameras.len() as u32, period)
            }
            _ => Vec::new(),
        };
        // The snapping pointer and its ghost (while snapping, the only
        // cursor), in the same camera on top, in window pixels.
        let snap_sprites = if cameras.len() < tile_render::MAX_VIEWS { self.snap.sprites(&self.compiled, cameras.len() as u32) } else { Vec::new() };
        if !cursor_sprites.is_empty() || !snap_sprites.is_empty() {
            cameras.push(cursor::overlay_camera(window));
        }
        // The frame counter over everything (the cursor too), by a camera of
        // its own showing the whole window 1:1 (whatever the split and
        // scales).
        let show_fps = self.settings.display.show_fps && cameras.len() < tile_render::MAX_VIEWS;
        let seconds = self.started.elapsed().as_secs_f64();
        if show_fps {
            let camera = cameras.len() as u32;
            markers.extend_from_slice(self.fps.sprites(&self.compiled, self.assets_version, seconds, window, camera, gpu.software));
            cameras.push(Camera { cells: false, ..Camera::new(window) });
        }
        let cursor_moved = cursor_sprites != self.drawn_cursor;
        markers.extend_from_slice(&cursor_sprites);
        markers.extend(snap_sprites);
        let world = self.world.as_mut().expect("built above");
        let own = world.free.len();
        world.free.extend_from_slice(&markers);
        if new_assets {
            gpu.renderer.set_world(&gpu.device, world);
        } else if rebuilt {
            gpu.renderer.update_world(&gpu.device, &gpu.queue, world);
        }
        let markers_moved = markers != self.drawn_markers || looks_changed;
        if !new_assets && !rebuilt && markers_moved {
            gpu.renderer.update_free(&gpu.device, &gpu.queue, world);
        }
        world.free.truncate(own);
        self.drawn_markers = markers;
        self.drawn_cursor = cursor_sprites;
        self.drawn_views = arranged.view_cameras.clone();
        if new_assets || (scene.seq != self.drawn_seq && matches) {
            apply_patches(&gpu.renderer, &gpu.queue, &self.compiled, self.patches.values());
        }
        if matches {
            self.drawn_seq = scene.seq;
            // (For measuring how soon a world asked for beside a screen shows.)
            let beside = scene.interface.beside.map(|b| (scene.shape, b.world));
            if beside.is_some() && beside != self.beside_drawn {
                log::info!("viewer: the world beside the screen drawn: {:?} (scene {})", beside, scene.seq);
            }
            self.beside_drawn = beside;
        }
        // (For measuring how long a game takes to show its map.)
        if !self.map_drawn && matches && scene.shape.cols > 0 && scene.views.iter().any(|v| !v.view.interface_only) {
            self.map_drawn = true;
            log::info!("viewer: first map drawn (scene {})", scene.seq);
        }
        // A software renderer draws a frame only when it differs from the last.
        let unchanged = !new_assets && !rebuilt && !markers_moved && scene.tick == self.drawn_tick && window == self.drawn_window && !gpu.reconfigured;
        // The pointer's motion is drawn (or there was nothing to draw): the
        // cursor moved, or the views following it.
        let views_followed = self.drawn_frame.as_ref().map(|f| &f.cameras) != self.cursor_cameras.as_ref();
        self.cursor_cameras = self.drawn_frame.as_ref().map(|f| f.cameras.clone());
        let since = self.pointer_since.take().filter(|_| cursor_moved || views_followed);
        // (Not while the mouse waits to be let go after a frame.)
        if gpu.software && unchanged && self.unlock_pending.is_none() {
            if show_fps {
                self.fps.counter.skipped(seconds);
            }
            return;
        }
        self.drawn_tick = scene.tick;
        self.drawn_window = window;
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
            other => {
                log::warn!("surface: {other:?}");
                return;
            }
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = &frame.texture;
        let copies = &arranged.copies;
        gpu.reconfigured = false;
        gpu.renderer.render_presented(&gpu.device, &gpu.queue, &mut encoder, target, &cameras, copies, scene.tick, self.config.clear, plan.present);
        gpu.queue.submit([encoder.finish()]);
        if let (Some(since), true) = (since, log_latency()) {
            let done = self.latency.done.clone();
            gpu.queue.on_submitted_work_done(move || done.lock().unwrap_or_else(|e| e.into_inner()).push(since.elapsed().as_secs_f64() * 1000.0));
        }
        if gpu.software {
            // (Called back from a later poll once this submission is done.)
            self.frame_done.store(false, std::sync::atomic::Ordering::Release);
            let done = self.frame_done.clone();
            gpu.queue.on_submitted_work_done(move || done.store(true, std::sync::atomic::Ordering::Release));
        }
        gpu.window.pre_present_notify();
        gpu.queue.present(frame);
        self.feedback.presented(self.started.elapsed().as_secs_f64());
        if show_fps {
            self.fps.counter.presented(seconds);
        }
        self.frames_presented += 1;
        if let Some(line) = self.frame_log.as_mut().and_then(|l| l.presented(seconds, started.elapsed().as_secs_f64() * 1000.0)) {
            log::info!("viewer: {line}");
        }
        self.presented = true;
        if let (Some(since), true) = (since, log_latency()) {
            self.latency.start.push(started.saturating_duration_since(since).as_secs_f64() * 1000.0);
            self.latency.present.push(since.elapsed().as_secs_f64() * 1000.0);
            self.latency.last = Some(Instant::now());
        }
        // Logged every 10 s, and once the pointer has rested for 2 s.
        let rested = self.latency.last.is_some_and(|t| t.elapsed() >= Duration::from_secs(2));
        if !self.latency.present.is_empty() && (self.latency_logged.elapsed() >= Duration::from_secs(10) || rested) {
            let span = std::mem::replace(&mut self.latency_logged, Instant::now()).elapsed();
            let _ = gpu.device.poll(wgpu::PollType::Poll);
            let frames = std::mem::take(&mut self.frames_presented);
            let mut done = std::mem::take(&mut *self.latency.done.lock().unwrap_or_else(|e| e.into_inner()));
            let fmt = |s: Option<(f64, f64, f64, usize)>| s.map_or("-".to_owned(), |(p50, p95, max, n)| format!("p50 {p50:.1} ms, p95 {p95:.1} ms, max {max:.1} ms ({n})"));
            log::info!(
                "viewer: cursor latency, motion to its frame started: {}; to frame presented: {}; to frame done on the GPU: {}; {frames} frames in {:.1} s",
                fmt(cursor::Latency::summary(&mut std::mem::take(&mut self.latency.start))),
                fmt(cursor::Latency::summary(&mut std::mem::take(&mut self.latency.present))),
                fmt(cursor::Latency::summary(&mut done)),
                span.as_secs_f64()
            );
        }
    }

}

impl ApplicationHandler for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        self.woken_idle = matches!(cause, winit::event::StartCause::WaitCancelled { .. });
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            if let Err(e) = self.init(event_loop) {
                log::error!("viewer init: {e}");
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.woken_idle = false;
        match &event {
            WindowEvent::CloseRequested => {
                self.display.send_input(Input::Quit);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.reconfigured = true;
                    log::debug!("viewer: window {}×{} pixels at scale {}", size.width, size.height, gpu.window.scale_factor());
                }
                self.update_plan();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                log::info!("viewer: the window's scale is now {scale_factor}");
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                self.unlock_after_frame();
                self.update_system_pointer(event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.constraints.pointer_moved((position.x, position.y));
                // (In the layout's pixels, the driver's.)
                self.system_pointer = self.plan.map_or((position.x as i32, position.y as i32), |p| p.to_screen((position.x, position.y)));
                // At the world's edge the system pointer moves the world
                // cursor (`cursor::Machine::edge`).
                if self.machine.edge() && self.unlock_pending.is_none() {
                    for input in self.layout.pointer_to(self.system_pointer) {
                        self.display.send_input(input);
                    }
                }
                self.pointer_moved();
            }
            WindowEvent::CursorEntered { .. } => self.pointer_inside = true,
            WindowEvent::CursorLeft { .. } => self.pointer_inside = false,
            _ => {}
        }
        if let WindowEvent::Focused(false) = event {
            for input in self.snap.focus_lost(&mut self.layout) {
                self.send(input);
            }
            self.layout.released();
            self.grab(false);
        }
        // The pointer is at no edge of the window once it left (or the
        // window is no longer the one the user works in).
        if matches!(event, WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. }) {
            for input in self.layout.pointer_gone() {
                self.send(input);
            }
            self.feedback.pointer_gone(self.started.elapsed().as_secs_f64());
        }
        let (now, rates) = (self.now(), snap::Rates::of(&self.settings.input));
        for input in self.input.translate(&event) {
            // (Through button snapping, which passes on what is not its.)
            let routed = self.snap.route(input.clone(), &mut self.layout, self.display.scene(), now, rates);
            // The pointer moved where the driver does not get it, a release
            // the driver does not get: no longer over its interactables.
            match input {
                Input::MouseMove { .. } if !self.layout.captured() && !routed.iter().any(|i| matches!(i, Input::MouseMove { .. })) => self.feedback.pointer_gone(now),
                Input::MouseButton { button: scene::Button::Left, down: false, .. } if !routed.iter().any(|i| matches!(i, Input::MouseButton { button: scene::Button::Left, down: false, .. })) => self.feedback.released(now),
                _ => {}
            }
            for input in routed {
                self.send(input);
            }
        }
        // (After a redraw too: what the pointer is over may have changed.)
        self.apply_layout_requests(event_loop);
        if let Some(zooms) = self.layout.take_zooms_change() {
            self.display.send_input(Input::Zooms(zooms));
        }
        if let Some((x, y, w, h)) = self.layout.take_main_view_change() {
            self.display.send_input(Input::MainView { x, y, w, h });
        }
        if let Some(views) = self.layout.take_views_change() {
            log::debug!("viewer: own views now show {views:?}");
            self.display.send_input(Input::Views(views));
        }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _id: winit::event::DeviceId, event: winit::event::DeviceEvent) {
        self.woken_idle = false;
        // While the mouse is captured its motion moves the world cursor.
        // (While snapping, it moves the snapping pointer's hidden pointer.)
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            // (At the world's edge the system pointer moves it instead; in
            // the driver's pixels.)
            let edge = self.machine.edge() && self.unlock_pending.is_none();
            let s = self.plan.map_or(1.0, |p| p.factor());
            let delta = (delta.0 / s, delta.1 / s);
            let inputs = self.snap.motion(delta).unwrap_or_else(|| if edge { vec![] } else { self.layout.motion(delta) });
            for input in inputs {
                self.send(input);
            }
            if self.pointer_hidden() {
                self.pointer_moved();
            }
            // (The world cursor may have left the view a click captures in.)
            self.apply_layout_requests(event_loop);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, (): ()) {
        // The driver sent updates: taken in `about_to_wait`, next.
        self.woken_idle = false;
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Take what the driver sent (it wakes the loop when it sends), then
        // redraw: at once for new updates, and on the animation clock's
        // beat without them. The loop waits for events meanwhile, so the
        // pointer's are taken (and drawn) the moment they come. Woken for
        // nothing (some systems' message loops wake the event loop over and
        // over, e.g. Wine's), it waits for the driver a little instead of
        // spinning; the window's events wait at most that long.
        let idle = std::mem::take(&mut self.woken_idle) && self.next_frame.is_some_and(|t| t > Instant::now());
        let changed = self.display.receive(if idle { IDLE_WAIT } else { Duration::ZERO });
        if let Some(settings) = self.display.take_settings() {
            if settings != self.settings {
                log::info!("viewer: new settings from the driver");
                self.set_settings(settings);
            }
        }
        let (now, rates) = (self.now(), snap::Rates::of(&self.settings.input));
        for input in self.snap.tick(&mut self.layout, self.display.scene(), now, rates) {
            self.send(input);
        }
        // (Snapping may have put the pointer somewhere, or let go of it.)
        self.apply_layout_requests(event_loop);
        let now = Instant::now();
        let due = self.next_frame.is_none_or(|t| now >= t);
        if due {
            self.next_frame = Some(now + FRAME);
        }
        if let Some(gpu) = &self.gpu {
            if self.display.scene().seq != 0 && (changed || due) {
                gpu.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame.unwrap_or(now + FRAME)));
    }
}

/// A window's inner size `want` no larger than a screen `screen` with room
/// for its frame: nine tenths of it at most, the same shape.
fn within_screen(want: winit::dpi::PhysicalSize<u32>, screen: winit::dpi::PhysicalSize<u32>) -> winit::dpi::PhysicalSize<u32> {
    let room = (screen.width as f64 * 0.9 / want.width.max(1) as f64).min(screen.height as f64 * 0.9 / want.height.max(1) as f64);
    if room < 1.0 {
        winit::dpi::PhysicalSize::new((want.width as f64 * room) as u32, (want.height as f64 * room) as u32)
    } else {
        want
    }
}

/// System cursors made from cursor art at a pixel scale, each on a thread
/// of its own; by (sprite set, pixel scale).
#[derive(Default)]
struct CursorBuilds {
    /// What is being made, and of which art (see `forget`).
    pending: std::collections::HashSet<(u32, u32)>,
    generation: u64,
    ready: Option<(std::sync::mpsc::Sender<CursorBuilt>, std::sync::mpsc::Receiver<CursorBuilt>)>,
}

type CursorBuilt = (u64, (u32, u32), Option<winit::window::CustomCursorSource>);

impl CursorBuilds {
    /// Makes `image` at pixel scale `key.1`, unless it is being made.
    fn start(&mut self, key: (u32, u32), image: cursor::Image) {
        if !self.pending.insert(key) {
            return;
        }
        let tx = self.ready.get_or_insert_with(std::sync::mpsc::channel).0.clone();
        let generation = self.generation;
        std::thread::spawn(move || {
            let n = key.1;
            let (rgba, (w, h)) = cursor::upscaled(&image.rgba, image.size, n);
            let hotspot = (image.hotspot.0 * n as u16, image.hotspot.1 * n as u16);
            let source = winit::window::CustomCursor::from_rgba(rgba, w, h, hotspot.0, hotspot.1)
                .inspect_err(|e| log::warn!("viewer: cursor of set {}: {e}", key.0))
                .ok();
            let _ = tx.send((generation, key, source));
        });
    }

    /// The cursors made since last asked (of the current art).
    fn take_ready(&mut self) -> Vec<((u32, u32), winit::window::CustomCursorSource)> {
        let Some((_, rx)) = &self.ready else { return Vec::new() };
        let mut out = Vec::new();
        while let Ok((generation, key, source)) = rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            // (One that could not be made is not tried again.)
            if let Some(source) = source {
                self.pending.remove(&key);
                out.push((key, source));
            }
        }
        out
    }

    /// New art (new set ids): what is being made is of no use.
    fn forget(&mut self) {
        self.generation += 1;
        self.pending.clear();
    }
}
