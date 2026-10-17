//! A window that draws a [`scene::Display`]'s scene (every view of it) with a
//! compiled render description, and sends the user's input back.
//!
//! Nothing here knows which game it is showing.

mod input;
pub mod layout;

use std::sync::Arc;
use std::time::Duration;

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
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self { title: "tile-viewer".into(), size: (1280, 960), fullscreen: false, clear: [0, 0, 0, 255], overhang: (0, 0, 0, 0) }
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
/// (e.g. when a game loads a scenario with its own art).
#[derive(Clone)]
pub struct Assets(Arc<std::sync::Mutex<(u64, Arc<Compiled>)>>);

impl Assets {
    pub fn new(compiled: Arc<Compiled>) -> Self {
        Self(Arc::new(std::sync::Mutex::new((1, compiled))))
    }

    pub fn replace(&self, compiled: Arc<Compiled>) {
        let mut slot = self.0.lock().unwrap();
        *slot = (slot.0 + 1, compiled);
    }

    /// Replaces the description by `compiled` of generation `generation`
    /// (what scenes refer to, `Scene::assets`).
    pub fn set(&self, compiled: Arc<Compiled>, generation: u64) {
        *self.0.lock().unwrap() = (generation, compiled);
    }

    /// Replaces version `version` by `light` (the same without its atlas
    /// pixels) once they are uploaded, unless a newer one came meanwhile.
    fn uploaded(&self, version: u64, light: Arc<Compiled>) {
        let mut slot = self.0.lock().unwrap();
        if slot.0 == version {
            slot.1 = light;
        }
    }

    /// The current description and its version (bumped by `replace`).
    pub fn current(&self) -> (u64, Arc<Compiled>) {
        let slot = self.0.lock().unwrap();
        (slot.0, slot.1.clone())
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
    }
    // The camera showing scene view `id`, if one does.
    let camera_of = |id: u32| scene.views.iter().zip(view_cameras).find(|(v, _)| v.id == id).and_then(|(_, c)| *c);
    let placed = |s: &scene::FreeSprite, view: Option<u32>| tile_render::PlacedSprite {
        set: tile_render::SpriteSetId::from_raw(s.set),
        x: s.x,
        y: s.y,
        screen: s.screen,
        after_pass: s.after_pass as usize,
        phase: s.phase,
        ticks_per_frame: s.ticks_per_frame.max(1),
        blend: tile_render::Blend::from_scale_den(s.scale_den, s.tint),
        clip: s.clip,
        view: view.or_else(|| s.not_in_view.and_then(camera_of)),
        except_view: view.is_none() && s.not_in_view.and_then(camera_of).is_some(),
        zoom_art: match s.zoom_art {
            1 => tile_render::ZoomArt::Low,
            2 => tile_render::ZoomArt::Zoomed { zoom: (u32::from(s.zoom.0), u32::from(s.zoom.1)), offset: s.zoom_offset },
            _ => tile_render::ZoomArt::Any,
        },
    };
    world.free = scene.sprites.iter().map(|s| placed(s, None)).collect();
    for (v, camera) in scene.views.iter().zip(view_cameras) {
        if let Some(c) = camera {
            world.free.extend(v.overlay.iter().map(|s| placed(s, Some(*c))));
        }
    }
    world
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
                downscale: 1,
            }
        })
        .collect()
}

/// The world's extent in pixels and how it wraps.
fn world_bounds(compiled: &Compiled, scene: &Scene) -> layout::WorldBounds {
    let s = scene.shape;
    let col = match compiled.layout {
        tile_render::Layout::Staggered => 2 * compiled.step.0,
        tile_render::Layout::Orthogonal => compiled.step.0,
    };
    layout::WorldBounds { size: ((s.cols * col) as i32, (s.rows * compiled.step.1) as i32), wrap: (s.wrap_x, s.wrap_y) }
}

/// Sprite sets the viewer draws its markers with, if the description has
/// them (anchored at the point they mark).
pub const CURSOR_SPRITE: &str = "Viewer@cursor/0";
pub const ARROW_SPRITE: &str = "Viewer@arrow/0";

/// The markers as world sprites on top of everything. The arrow blinks
/// twice every three seconds (`time`: since the viewer started).
fn marker_sprites(compiled: &Compiled, markers: &[layout::Marker], time: Duration) -> Vec<tile_render::PlacedSprite> {
    let blink = time.as_millis() % 3000;
    let arrow_off = blink < 150 || (300..450).contains(&blink);
    markers
        .iter()
        .filter_map(|m| {
            let (name, (x, y)) = match *m {
                layout::Marker::Cursor(p) => (CURSOR_SPRITE, p),
                layout::Marker::Focus(_) if arrow_off => return None,
                layout::Marker::Focus(p) => (ARROW_SPRITE, p),
            };
            let (set, _) = compiled.set(name)?;
            Some(tile_render::PlacedSprite::new(set, x, y))
        })
        .collect()
}

/// The window rectangles (x, y, w, h) the scene's interface-only views
/// cover: their sprites' first frames, clipped.
fn interface_rects(compiled: &Compiled, scene: &Scene) -> Vec<[i32; 4]> {
    let mut out = Vec::new();
    for v in scene.views.iter().filter(|v| v.view.interface_only) {
        for s in &v.overlay {
            let Some(id) = compiled.sets.frame(tile_render::SpriteSetId::from_raw(s.set), 0) else { continue };
            let Some(sprite) = compiled.atlas.sprites.get(id.0 as usize) else { continue };
            let (mut x0, mut y0) = (s.x + sprite.anchor_x, s.y + sprite.anchor_y);
            let (mut x1, mut y1) = (x0 + sprite.width as i32, y0 + sprite.height as i32);
            if let Some([cx0, cy0, cx1, cy1]) = s.clip {
                (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
            }
            if x1 > x0 && y1 > y0 {
                out.push([x0, y0, x1 - x0, y1 - y0]);
            }
        }
    }
    out
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
        let Some(id) = compiled.sprite_names.iter().position(|n| *n == p.sprite) else { continue };
        if p.rgba.len() != (p.width * p.height * 4) as usize {
            continue;
        }
        let image = RgbaImage::from_fn(p.width, p.height, |x, y| {
            let i = ((y * p.width + x) * 4) as usize;
            [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
        });
        if !renderer.patch_sprite(queue, tile_render::SpriteId(id as u32), &image) {
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

/// Opens the window and runs until it is closed, drawing what `display`
/// receives. Blocks the calling thread, which need not be the process's
/// main thread (the viewer usually runs inside someone else's process).
pub fn run(config: ViewerConfig, assets: Assets, display: Display) -> Result<(), ViewerError> {
    let mut builder = EventLoop::builder();
    #[cfg(windows)]
    winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(&mut builder, true);
    #[cfg(all(unix, not(target_os = "macos")))]
    winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    let event_loop = builder.build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let (assets_version, compiled) = assets.current();
    let mut app = App {
        config,
        assets,
        assets_version,
        compiled,
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
        started: std::time::Instant::now(),
        art_waiting: None,
        drawn_frame: None,
        error: None,
    };
    // The arrangement to start with (see `layout::Layout::configure`).
    if let Ok(spec) = std::env::var("TILE_VIEWER_LAYOUT") {
        if let Err(e) = app.layout.configure(&spec) {
            log::warn!("TILE_VIEWER_LAYOUT: {e}");
        }
    }
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
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
}

struct App {
    config: ViewerConfig,
    assets: Assets,
    assets_version: u64,
    compiled: Arc<Compiled>,
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
    started: std::time::Instant,
    /// Since when newer art is waiting for a scene that refers to it.
    art_waiting: Option<std::time::Instant>,
    /// The arrangement drawn last (drawn again while a scene waits for its
    /// art).
    drawn_frame: Option<layout::Frame>,
    error: Option<ViewerError>,
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), ViewerError> {
        let mut attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(self.config.size.0, self.config.size.1))
            .with_resizable(true);
        if self.config.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs)?);
        let instance = wgpu::Instance::new(tile_render::gpu::instance_descriptor());
        let surface = instance.create_surface(window.clone())?;
        let (adapter, device, queue) = pollster::block_on(async {
            let adapter = tile_render::gpu::select_adapter(&instance, Some(&surface), None).await?;
            let (device, queue) = tile_render::gpu::request_device(&adapter).await?;
            Ok::<_, ViewerError>((adapter, device, queue))
        })?;
        log::info!("viewer adapter: {:?}", adapter.get_info());
        // A non-sRGB format, so atlas colours reach the screen unchanged.
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        // Scale-blend passes copy the target, so ask for COPY_SRC.
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & wgpu::TextureUsages::COPY_SRC);
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let mut renderer = TileRenderer::new(&device, config.format);
        renderer.set_atlas(&device, &queue, &self.compiled.atlas);
        let light = Arc::new(self.compiled.without_pixels());
        self.assets.uploaded(self.assets_version, light.clone());
        self.compiled = light;
        self.display.send_input(Input::Resized { width: config.width, height: config.height });
        let software = adapter.get_info().device_type == wgpu::DeviceType::Cpu;
        if software {
            log::info!("viewer: software renderer, one frame in flight, unchanged frames skipped");
        }
        self.gpu = Some(Gpu { window, surface, device, queue, config, renderer, software });
        Ok(())
    }

    /// Captures (hides, holds) or releases the mouse.
    fn grab(&self, lock: bool) {
        let Some(gpu) = &self.gpu else { return };
        use winit::window::CursorGrabMode;
        if lock {
            // Locked where the system can, else kept inside the window.
            if gpu.window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                let _ = gpu.window.set_cursor_grab(CursorGrabMode::Confined);
            }
        } else {
            let _ = gpu.window.set_cursor_grab(CursorGrabMode::None);
        }
        gpu.window.set_cursor_visible(!lock);
    }

    fn draw(&mut self) {
        let Some(gpu) = &mut self.gpu else { return };
        // A software renderer still at the last frame: no new one yet (the
        // event loop goes on meanwhile).
        let done = || self.frame_done.load(std::sync::atomic::Ordering::Acquire);
        if gpu.software && !done() {
            let _ = gpu.device.poll(wgpu::PollType::Poll);
            if !done() {
                return;
            }
        }
        let (version, compiled) = self.assets.current();
        // New art is taken together with a scene that refers to it (sprite
        // ids change with every compile), or when none came for a while.
        let refers_to = self.display.scene().assets;
        if version == self.assets_version && (refers_to == 0 || refers_to == version) {
            self.art_waiting = None;
        }
        let gave_up = self.art_waiting.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(3);
        let new_assets = version != self.assets_version && (refers_to == 0 || refers_to == version || gave_up);
        if new_assets {
            let started = std::time::Instant::now();
            gpu.renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
            log::info!("viewer: new assets ({} sprites, uploaded in {} ms)", compiled.sprite_names.len(), started.elapsed().as_millis());
            // The pixels are on the GPU now: keep only the rest.
            let light = Arc::new(compiled.without_pixels());
            drop(compiled);
            self.assets.uploaded(version, light.clone());
            self.compiled = light;
            self.assets_version = version;
        }
        for p in self.display.take_patches() {
            self.patches.insert(p.sprite.clone(), p);
        }
        let scene = self.display.scene();
        if scene.seq == 0 {
            return;
        }
        let window = (gpu.config.width, gpu.config.height);
        // A scene for other art than the renderer has waits: the last frame
        // is drawn meanwhile as it was, its world in its arrangement (the
        // waiting scene's views may differ, e.g. a screen opened with a view
        // of its own).
        let matches = scene.assets == 0 || scene.assets == self.assets_version || gave_up;
        let arranged = match &self.drawn_frame {
            Some(frame) if !matches && self.world.is_some() && window == self.drawn_window => frame.clone(),
            _ => {
                let bounds = world_bounds(&self.compiled, scene);
                let arranged = self.layout.frame(scene, window, self.config.overhang, interface_rects(&self.compiled, scene), Some(bounds));
                self.drawn_frame = Some(arranged.clone());
                arranged
            }
        };
        // Views following something move without input: tell the driver.
        if let Some(views) = self.layout.take_views_change() {
            self.display.send_input(Input::Views(views));
        }
        let views_moved = arranged.view_cameras != self.drawn_views;
        let rebuilt = new_assets || ((scene.seq != self.drawn_seq || views_moved) && matches) || self.world.is_none();
        if rebuilt {
            self.world = Some(scene_world_for(&self.compiled, scene, &arranged.view_cameras));
        }
        // The viewer's markers on top of the world, for this frame only.
        let markers = marker_sprites(&self.compiled, &arranged.markers, self.started.elapsed());
        let world = self.world.as_mut().expect("built above");
        let own = world.free.len();
        world.free.extend_from_slice(&markers);
        if new_assets {
            gpu.renderer.set_world(&gpu.device, world);
        } else if rebuilt {
            gpu.renderer.update_world(&gpu.device, &gpu.queue, world);
        }
        let markers_moved = markers != self.drawn_markers;
        if !new_assets && !rebuilt && markers_moved {
            gpu.renderer.update_free(&gpu.device, &gpu.queue, world);
        }
        world.free.truncate(own);
        self.drawn_markers = markers;
        self.drawn_views = arranged.view_cameras.clone();
        if new_assets || (scene.seq != self.drawn_seq && matches) {
            apply_patches(&gpu.renderer, &gpu.queue, &self.compiled, self.patches.values());
        }
        if matches {
            self.drawn_seq = scene.seq;
        }
        // A software renderer draws a frame only when it differs from the last.
        let unchanged = !new_assets && !rebuilt && !markers_moved && scene.tick == self.drawn_tick && window == self.drawn_window;
        if gpu.software && unchanged {
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
        let (cameras, copies) = (&arranged.cameras, &arranged.copies);
        gpu.renderer.render_composed(&gpu.device, &gpu.queue, &mut encoder, target, cameras, copies, scene.tick, self.config.clear);
        gpu.queue.submit([encoder.finish()]);
        if gpu.software {
            // (Called back from a later poll once this submission is done.)
            self.frame_done.store(false, std::sync::atomic::Ordering::Release);
            let done = self.frame_done.clone();
            gpu.queue.on_submitted_work_done(move || done.store(true, std::sync::atomic::Ordering::Release));
        }
        gpu.window.pre_present_notify();
        gpu.queue.present(frame);
    }
}

impl ApplicationHandler for App {
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
        match &event {
            WindowEvent::CloseRequested => {
                self.display.send_input(Input::Quit);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.display.send_input(Input::Resized { width: size.width, height: size.height });
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
        if let WindowEvent::Focused(false) = event {
            self.layout.released();
            self.grab(false);
        }
        // The pointer is at no edge of the window once it left (or the
        // window is no longer the one the user works in).
        if matches!(event, WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. }) {
            for input in self.layout.pointer_gone() {
                self.display.send_input(input);
            }
        }
        for input in self.input.translate(&event) {
            for input in self.layout.route(input) {
                self.display.send_input(input);
            }
        }
        if let Some(lock) = self.layout.take_grab() {
            self.grab(lock);
        }
        if let Some(zooms) = self.layout.take_zooms_change() {
            self.display.send_input(Input::Zooms(zooms));
        }
        if let Some(views) = self.layout.take_views_change() {
            self.display.send_input(Input::Views(views));
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: winit::event::DeviceId, event: winit::event::DeviceEvent) {
        // While the mouse is captured its motion moves the world cursor.
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            for input in self.layout.motion(delta) {
                self.display.send_input(input);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Take what the driver sent (waiting a frame's time for it), then
        // redraw: the animation clock moves on even without updates.
        self.display.receive(Duration::from_millis(16));
        if let Some(gpu) = &self.gpu {
            if self.display.scene().seq != 0 {
                gpu.window.request_redraw();
            }
        }
    }
}
