//! A window that draws a [`scene::Display`]'s scene (every view of it) with a
//! compiled render description, and sends the user's input back.
//!
//! Nothing here knows which game it is showing.

mod input;

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
    let placed = |s: &scene::FreeSprite, view: Option<u32>| tile_render::PlacedSprite {
        set: tile_render::SpriteSetId::from_raw(s.set),
        x: s.x,
        y: s.y,
        screen: s.screen,
        after_pass: s.after_pass as usize,
        phase: s.phase,
        ticks_per_frame: s.ticks_per_frame.max(1),
        blend: if s.scale_den == 0 { tile_render::Blend::Alpha } else { tile_render::Blend::Scale { denominator: s.scale_den } },
        clip: s.clip,
        view,
    };
    world.free = scene.sprites.iter().map(|s| placed(s, None)).collect();
    for (i, v) in scene.views.iter().enumerate() {
        world.free.extend(v.overlay.iter().map(|s| placed(s, Some(i as u32))));
    }
    world
}

/// The cameras of the scene's views, in its view order. A view with an
/// empty viewport fills `window`.
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
                zoom: (v.zoom.0.max(1), v.zoom.1.max(1)),
                viewport,
                overhang,
                cull: v.cull,
                origin: v.origin,
                cells: !v.interface_only,
            }
        })
        .collect()
}

/// Writes run-time sprite pixels into the atlas.
fn apply_patches<'p>(renderer: &TileRenderer, queue: &wgpu::Queue, patches: impl IntoIterator<Item = &'p scene::Patch>) {
    for p in patches {
        if p.rgba.len() != (p.width * p.height * 4) as usize {
            continue;
        }
        let image = RgbaImage::from_fn(p.width, p.height, |x, y| {
            let i = ((y * p.width + x) * 4) as usize;
            [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
        });
        if !renderer.patch_sprite(queue, tile_render::SpriteId(p.sprite), &image) {
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
    apply_patches(&renderer, &gpu.queue, &scene.patches);
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
        error: None,
    };
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
    patches: std::collections::BTreeMap<u32, scene::Patch>,
    input: input::InputState,
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
        self.gpu = Some(Gpu { window, surface, device, queue, config, renderer });
        Ok(())
    }

    fn draw(&mut self) {
        let Some(gpu) = &mut self.gpu else { return };
        let (version, compiled) = self.assets.current();
        let new_assets = version != self.assets_version;
        if new_assets {
            log::info!("viewer: new assets ({} sprites)", compiled.sprite_names.len());
            gpu.renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
            // The pixels are on the GPU now: keep only the rest.
            let light = Arc::new(compiled.without_pixels());
            drop(compiled);
            self.assets.uploaded(version, light.clone());
            self.compiled = light;
            self.assets_version = version;
        }
        for p in self.display.take_patches() {
            self.patches.insert(p.sprite, p);
        }
        let scene = self.display.scene();
        if scene.seq == 0 {
            return;
        }
        if new_assets {
            gpu.renderer.set_world(&gpu.device, &scene_world(&self.compiled, scene));
        } else if scene.seq != self.drawn_seq {
            gpu.renderer.update_world(&gpu.device, &gpu.queue, &scene_world(&self.compiled, scene));
        }
        if new_assets || scene.seq != self.drawn_seq {
            apply_patches(&gpu.renderer, &gpu.queue, self.patches.values());
        }
        self.drawn_seq = scene.seq;
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
        let cameras = scene_cameras(scene, (gpu.config.width, gpu.config.height), self.config.overhang);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.renderer.render_views(&gpu.device, &gpu.queue, &mut encoder, &frame.texture, &cameras, scene.tick, self.config.clear);
        gpu.queue.submit([encoder.finish()]);
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
        for input in self.input.translate(&event) {
            self.display.send_input(input);
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
