//! A window that draws a [`scene::Display`]'s scene (every view of it) with a
//! compiled render description, and sends the user's input back.
//!
//! Nothing here knows which game it is showing.

pub mod cursor;
pub mod fps;
mod input;
pub mod layout;
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
        if let (Some(offsets), true) = (&layer.offsets, compiled.layers[i].per_cell_offset) {
            for (cell, &o) in offsets.iter().enumerate().take(cells as usize).filter(|(_, o)| **o != (0, 0)) {
                world.set_offset(id, cell as u32 % s.cols, cell as u32 / s.cols, o);
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
        text: s.glyph.and_then(|g| outline_glyph(compiled, &g, s.tint)),
        palette: s.palette,
    };
    world.free = scene.sprites.iter().map(|s| placed(s, None)).collect();
    for (v, camera) in scene.views.iter().zip(view_cameras) {
        if let Some(c) = camera {
            world.free.extend(v.overlay.iter().map(|s| placed(s, Some(*c))));
        }
    }
    world
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

/// The window rectangles (x, y, w, h) the scene's interface-only views
/// cover: their sprites' first frames, clipped (not the parts the layout
/// may draw elsewhere, `scene::Interface`, which it knows itself).
fn interface_rects(compiled: &Compiled, scene: &Scene) -> Vec<[i32; 4]> {
    let mut out = Vec::new();
    let parts = [scene.interface.roaming, scene.interface.popup, scene.interface.screen_view];
    for v in scene.views.iter().filter(|v| v.view.interface_only && !parts.contains(&Some(v.id))) {
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
    // The driver's updates wake the event loop (it waits for its window's
    // events, which are taken at once, the pointer's among them).
    let proxy = event_loop.create_proxy();
    display.set_wake(move || {
        let _ = proxy.send_event(());
    });
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
        drawn_at: None,
        settings: viewer_settings::Settings::default(),
        error: None,
        snap: snap::SnapInput::default(),
        fps: fps::Overlay::default(),
        cursor_art: cursor::Art::default(),
        custom_cursors: std::collections::HashMap::new(),
        shown_pointer: None,
        capture_icon: false,
        pointer_inside: true,
        pointer_since: None,
        drawn_cursor: Vec::new(),
        latency: cursor::Latency::default(),
        latency_logged: Instant::now(),
        frames_presented: 0,
        unlock_pending: None,
        machine: cursor::Machine::default(),
        system_pointer: (0, 0),
        cursor: scene::Cursor::Default,
        presented: false,
        woken_idle: false,
        cursor_cameras: None,
        next_frame: None,
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
    /// When the last frame was drawn (the layout's smooth movement).
    drawn_at: Option<std::time::Instant>,
    error: Option<ViewerError>,
    /// The viewer's own settings (resolution, layout, input): read from
    /// `viewer_settings::Settings::default_path()` at start, replaced by those
    /// the driver sends (`scene::Update::Settings`; the driver keeps and saves
    /// them, e.g. edited on its preferences screen).
    settings: viewer_settings::Settings,
    /// Button snapping (Alt while the mouse is captured).
    snap: snap::SnapInput,
    /// The frame counter (`settings.display.show_fps`).
    fps: fps::Overlay,
    /// The cursor art kept for the system pointer, the system cursors made
    /// of it (by set), what the system pointer shows, and whether it is the
    /// capture pointer (the layout's `PointerIcon::Capture`).
    cursor_art: cursor::Art,
    custom_cursors: std::collections::HashMap<u32, winit::window::CustomCursor>,
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

    /// Seconds since the viewer started (the snapping pointer's clock).
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether the viewer draws the cursor itself rather than the system:
    /// while the mouse is captured, or the pointer snaps to buttons.
    fn pointer_hidden(&self) -> bool {
        (self.layout.captured() && !self.machine.edge()) || self.snap.snap_mode_active() || self.unlock_pending.is_some()
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
                let made = match self.custom_cursors.get(&set) {
                    Some(c) => Some(c.clone()),
                    None => self.cursor_art.get(set).and_then(|image| {
                        let source = winit::window::CustomCursor::from_rgba(image.rgba.clone(), image.size.0, image.size.1, image.hotspot.0, image.hotspot.1);
                        match source {
                            Ok(source) => {
                                let c = event_loop.create_custom_cursor(source);
                                self.custom_cursors.insert(set, c.clone());
                                Some(c)
                            }
                            Err(e) => {
                                log::warn!("viewer: cursor of set {set}: {e}");
                                None
                            }
                        }
                    }),
                };
                match made {
                    Some(c) => gpu.window.set_cursor(c),
                    None => gpu.window.set_cursor(winit::window::CursorIcon::Default),
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
        if let (Some((x, y)), Some(gpu)) = (warp, &self.gpu) {
            if let Err(e) = gpu.window.set_cursor_position(winit::dpi::PhysicalPosition::new(x, y)) {
                log::debug!("viewer: moving the pointer to {x}, {y}: {e}");
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
        self.update_system_pointer(event_loop);
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
            // No frame waits behind another: the cursor drawn here lags as
            // little as it can.
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let mut renderer = TileRenderer::new(&device, config.format);
        renderer.set_atlas(&device, &queue, &self.compiled.atlas);
        self.cursor_art = cursor::Art::of(&self.compiled);
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

    /// Captures (holds) or releases the mouse (the pointer is hidden and
    /// shown by `update_system_pointer`).
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
    }

    fn draw(&mut self) {
        let started = Instant::now();
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
            // (Set ids change with every compile.)
            self.cursor_art = cursor::Art::of(&compiled);
            self.custom_cursors.clear();
            self.shown_pointer = None;
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
                let now = std::time::Instant::now();
                if let Some(at) = self.drawn_at.replace(now) {
                    self.layout.advance(now - at);
                }
                let bounds = world_bounds(&self.compiled, scene);
                let arranged = self.layout.frame(scene, window, self.config.overhang, interface_rects(&self.compiled, scene), Some(bounds));
                self.drawn_frame = Some(arranged.clone());
                arranged
            }
        };
        // Views following something move without input: tell the driver.
        if let Some(views) = self.layout.take_views_change() {
            log::debug!("viewer: own views now show {views:?}");
            self.display.send_input(Input::Views(views));
        }
        let views_moved = arranged.view_cameras != self.drawn_views;
        let rebuilt = new_assets || ((scene.seq != self.drawn_seq || views_moved) && matches) || self.world.is_none();
        if rebuilt {
            self.world = Some(scene_world_for(&self.compiled, scene, &arranged.view_cameras));
        }
        // The viewer's markers on top of the world, for this frame only.
        let mut markers = marker_sprites(&self.compiled, &arranged.markers, world_downscale(scene), self.started.elapsed());
        // The snapping pointer and its ghost (while snapping, the only cursor).
        markers.extend(self.snap.sprites(&self.compiled, scene, &arranged, &self.layout));
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
        let plan = self.machine.step(&facts);
        let marks = plan.marks;
        if let Some(q) = plan.enter_edge {
            // The view following the world cursor is held at the world's
            // edge: the system pointer goes where the drawn cursor is and
            // shows there once this frame is out (`unlock_after_frame`).
            if let Err(e) = gpu.window.set_cursor_position(winit::dpi::PhysicalPosition::new(q.0, q.1)) {
                log::debug!("viewer: moving the pointer to the world cursor: {e}");
            }
            self.system_pointer = q;
            self.unlock_pending = Some(Unlock::Edge);
            self.presented = false;
            log::info!("viewer: world cursor at the world's edge: the system pointer shows it at {q:?}");
        }
        if plan.leave_edge {
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
        if !cursor_sprites.is_empty() {
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
        self.drawn_cursor = cursor_sprites;
        self.drawn_views = arranged.view_cameras.clone();
        if new_assets || (scene.seq != self.drawn_seq && matches) {
            apply_patches(&gpu.renderer, &gpu.queue, &self.compiled, self.patches.values());
        }
        if matches {
            self.drawn_seq = scene.seq;
        }
        // A software renderer draws a frame only when it differs from the last.
        let unchanged = !new_assets && !rebuilt && !markers_moved && scene.tick == self.drawn_tick && window == self.drawn_window;
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
        gpu.renderer.render_composed(&gpu.device, &gpu.queue, &mut encoder, target, &cameras, copies, scene.tick, self.config.clear);
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
        if show_fps {
            self.fps.counter.presented(seconds);
        }
        self.frames_presented += 1;
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
                self.display.send_input(Input::Resized { width: size.width, height: size.height });
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                self.unlock_after_frame();
                self.update_system_pointer(event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.system_pointer = (position.x as i32, position.y as i32);
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
                self.display.send_input(input);
            }
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
        let (now, rates) = (self.now(), snap::Rates::of(&self.settings.input));
        for input in self.input.translate(&event) {
            // (Through button snapping, which passes on what is not its.)
            for input in self.snap.route(input, &mut self.layout, self.display.scene(), now, rates) {
                self.display.send_input(input);
            }
        }
        // (After a redraw too: what the pointer is over may have changed.)
        self.apply_layout_requests(event_loop);
        if let Some(zooms) = self.layout.take_zooms_change() {
            self.display.send_input(Input::Zooms(zooms));
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
            // (At the world's edge the system pointer moves it instead.)
            let edge = self.machine.edge() && self.unlock_pending.is_none();
            let inputs = self.snap.motion(delta).unwrap_or_else(|| if edge { vec![] } else { self.layout.motion(delta) });
            for input in inputs {
                self.display.send_input(input);
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
            self.display.send_input(input);
        }
        self.update_system_pointer(event_loop);
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
