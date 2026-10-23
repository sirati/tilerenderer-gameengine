//! A window that draws a [`scene::Display`]'s scene (every view of it) with a
//! compiled render description, and sends the user's input back.
//!
//! Nothing here knows which game it is showing.

mod sprite_lifetimes;
mod unit_motions;
mod scene_prefix;
mod scene_snapshot;
mod ui_submission;
mod render_worker;
mod atlas_reservation;
mod thread_start;
mod constraints;
pub mod cursor;
mod system_cursor;
pub mod fps;
mod startup_ready;
pub mod host_status;
pub mod settings_preview;
mod input;
pub mod layout;
pub mod looks;
pub mod scaling;
pub mod profile;
mod presentation;
pub mod snap;
pub mod smooth_menus;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
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
    /// Window/application icon, including the process-wide Dock icon on macOS.
    pub application_icon: Option<application_icon::ApplicationIcon>,
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
    /// Host context established first on viewer-owned GPU and cursor threads.
    /// In-process hosts can supply signal/thread setup without exposing their
    /// platform details to the renderer. Never called on the event-loop thread.
    pub native_thread_init: Option<fn()>,
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self { title: "tile-viewer".into(), application_icon: None, size: (1280, 960), fullscreen: false, clear: [0, 0, 0, 255], overhang: (0, 0, 0, 0), process_dpi_aware: true, native_thread_init: None }
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
    #[error("software presentation: {0}")]
    SoftwarePresent(String),
    #[error("GPU worker: {0}")]
    RenderWorker(String),
    #[error("host application: {0}")]
    Host(String),
}

/// The compiled description a viewer draws with, replaceable while it runs
/// (e.g. when a game loads a scenario with its own art), and growing
/// (`render_desc::Growth`): each whole description and each growth has a
/// generation (what scenes refer to, `Scene::assets`); a growth keeps what
/// was there, so a scene of an earlier generation of the same whole
/// description is drawn with a later one.
///
/// The producers' end, cloned freely; no call waits for another thread.
/// The viewer reads it through its [`ArtInbox`].
#[derive(Clone)]
pub struct Assets(Arc<Shared>);

/// What producers publish and the viewer takes. Each published whole
/// description has an epoch; growths and reservations name the epoch they
/// were made for, so those of a replaced whole are dropped as before.
struct Shared {
    /// The latest whole description, until the viewer has uploaded it.
    whole: civ3r_sync::Latest<Whole>,
    /// Epochs handed out, and the epoch of the whole last published.
    epochs: AtomicU64,
    base: AtomicU64,
    /// Growths and reservations in the order given.
    added: civ3r_sync::Mailbox<Added>,
    /// The latest generation given (whole or grown).
    latest: AtomicU64,
    /// Highest generation committed to the drawing renderer. CPU art
    /// publication and replacement requests do not advance this.
    uploaded: AtomicU64,
}

#[derive(Clone)]
struct Whole {
    epoch: u64,
    generation: u64,
    /// CPU cursor pixels survive renderer uploads and are available immediately.
    cursor_art: Arc<cursor::Art>,
    /// Dropped once uploaded.
    compiled: Option<Arc<Compiled>>,
    /// Atlas pages (colour, index) to make room for, given with it.
    reserve: (u32, u32),
}

struct Added {
    epoch: u64,
    growth: Option<(u64, Arc<render_desc::Growth>)>,
    reserve: (u32, u32),
}

impl Assets {
    pub fn new(compiled: Arc<Compiled>) -> Self {
        Self::with_whole(Some(compiled), 1)
    }

    fn with_whole(compiled: Option<Arc<Compiled>>, generation: u64) -> Self {
        let shared = Shared {
            whole: civ3r_sync::Latest::new(),
            epochs: AtomicU64::new(0),
            base: AtomicU64::new(0),
            added: civ3r_sync::Mailbox::new(),
            latest: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
        };
        let assets = Self(Arc::new(shared));
        assets.publish(compiled, generation, (0, 0));
        assets
    }

    fn publish(&self, compiled: Option<Arc<Compiled>>, generation: u64, reserve: (u32, u32)) {
        let s = &self.0;
        let epoch = s.epochs.fetch_add(1, SeqCst) + 1;
        // The whole first: whatever is sent for its epoch afterwards finds
        // it published.
        let cursor_art = Arc::new(compiled.as_deref().map(cursor::Art::of).unwrap_or_default());
        s.whole.publish(Whole { epoch, generation, compiled, cursor_art, reserve });
        s.latest.store(generation, SeqCst);
        s.base.store(epoch, SeqCst);
    }

    fn add(&self, growth: Option<(u64, Arc<render_desc::Growth>)>, reserve: (u32, u32)) {
        let s = &self.0;
        if let Some((generation, _)) = &growth {
            s.latest.fetch_max(*generation, SeqCst);
        } else if reserve == (0, 0) {
            return;
        }
        s.added.send(Added { epoch: s.base.load(SeqCst), growth, reserve });
    }

    pub fn replace(&self, compiled: Arc<Compiled>) {
        let generation = self.latest() + 1;
        self.set(compiled, generation);
    }

    /// Replaces the description by `compiled` of generation `generation`
    /// (what scenes refer to, `Scene::assets`).
    pub fn set(&self, compiled: Arc<Compiled>, generation: u64) {
        self.publish(Some(compiled), generation, (0, 0));
    }

    /// Grows the latest description by `growth` (made for it as grown so
    /// far), as generation `generation`.
    pub fn grow(&self, growth: Arc<render_desc::Growth>, generation: u64) {
        self.add(Some((generation, growth)), (0, 0));
    }

    /// Asks the viewer to make room in its atlas for `pages` colour and
    /// `index_pages` index pages (art expected soon), so that growing it
    /// does not move it again and again (`TileRenderer::reserve_atlas`).
    pub fn reserve(&self, pages: u32, index_pages: u32) {
        self.add(None, (pages, index_pages));
    }

    /// Publishes whole art and its reservation together. A reservation
    /// from an older whole description never resizes its replacement.
    pub fn set_with_reserve(&self, compiled: Arc<Compiled>, generation: u64, pages: u32, index_pages: u32) {
        self.publish(Some(compiled), generation, (pages, index_pages));
    }

    /// Publishes an addition and its reservation in the same update.
    pub fn grow_with_reserve(&self, growth: Arc<render_desc::Growth>, generation: u64, pages: u32, index_pages: u32) {
        self.add(Some((generation, growth)), (pages, index_pages));
    }

    /// The latest generation given (whole or grown).
    pub fn latest(&self) -> u64 {
        self.0.latest.load(SeqCst)
    }

    /// The highest art generation committed to the drawing renderer.
    /// Zero until the initial atlas is uploaded. Unlike [`Assets::latest`],
    /// preparing CPU art cannot advance this acknowledgment. Replacement
    /// requests preserve it until the replacement is committed.
    pub fn uploaded_generation(&self) -> u64 {
        self.0.uploaded.load(SeqCst)
    }

    /// A whole base and its required growth prefix are committed. Release
    /// source pixels only for that base; a newer whole keeps its own pixels.
    fn uploaded(&self, base: u64, through: u64) {
        self.0.uploaded.fetch_max(through, SeqCst);
        let held = |w: &Whole| w.generation == base && w.compiled.is_some();
        if self.0.whole.load().is_some_and(|w| held(&w)) {
            // Retried against a whole published meanwhile, which is kept.
            self.0.whole.update(|old| old.map(|w| if held(w) { Whole { compiled: None, ..w.clone() } } else { w.clone() }));
        }
    }
}

/// The viewer's end of [`Assets`]: what it has taken but not applied yet,
/// owned by the viewer's thread alone.
struct ArtInbox {
    assets: Assets,
    /// The whole description these are for (its epoch and generation).
    epoch: u64,
    generation: u64,
    /// Its growths not yet applied by the viewer, in order.
    growths: std::collections::VecDeque<(u64, Arc<render_desc::Growth>)>,
    /// Atlas pages (colour, index) to make room for, if asked (see
    /// [`Assets::reserve`]), with the generation of their whole.
    reserve: Option<(u64, u32, u32)>,
    requested_room: atlas_reservation::Requests,
}

impl std::ops::Deref for ArtInbox {
    type Target = Assets;
    fn deref(&self) -> &Assets {
        &self.assets
    }
}

impl ArtInbox {
    fn new(assets: Assets) -> Self {
        Self { assets, epoch: 0, generation: 0, growths: Default::default(), reserve: None, requested_room: Default::default() }
    }

    /// Takes what was published, and returns the latest whole.
    fn receive(&mut self) -> Arc<Whole> {
        // Received before the whole is read: everything received names a
        // whole published already, so none is for one not seen yet.
        let added: Vec<Added> = self.assets.0.added.drain().collect();
        let whole = self.assets.0.whole.load().expect("published on creation");
        if whole.epoch != self.epoch {
            self.epoch = whole.epoch;
            self.generation = whole.generation;
            self.growths.clear();
            self.reserve = None;
            self.requested_room = Default::default();
            self.reserve_more(whole.reserve);
        }
        let epoch = self.epoch;
        for a in added.into_iter().filter(|a| a.epoch == epoch) {
            self.growths.extend(a.growth);
            self.reserve_more(a.reserve);
        }
        whole
    }

    fn reserve_more(&mut self, requested: (u32, u32)) {
        let Some((pages, index_pages)) = self.requested_room.request(requested) else { return };
        // A taken request belongs to the transfer until it commits, retries or
        // fails. Keep its high-water mark even after clearing this mailbox.
        self.reserve = Some((self.generation, pages, index_pages));
    }

    /// The latest whole description's generation.
    fn generation(&mut self) -> u64 {
        self.receive().generation
    }

    /// The latest whole description's generation, and the description if
    /// the viewer has not uploaded it yet.
    fn whole(&mut self) -> (u64, Option<Arc<Compiled>>) {
        let whole = self.receive();
        (whole.generation, whole.compiled.clone())
    }

    fn take_reserve(&mut self, generation: u64) -> Option<(u32, u32)> {
        self.receive();
        if self.reserve.is_some_and(|(base, _, _)| base == generation) {
            self.reserve.take().map(|(_, pages, index)| (pages, index))
        } else {
            None
        }
    }

    /// The growths of whole generation `generation` not applied yet.
    fn take_growth(&mut self, generation: u64) -> Option<(u64, Arc<render_desc::Growth>)> {
        self.receive();
        if self.generation == generation { self.growths.pop_front() } else { None }
    }
}

#[cfg(test)]
mod art_queue_tests {
    use super::{ArtInbox, Assets};
    use std::sync::Arc;

    fn assets() -> ArtInbox {
        ArtInbox::new(Assets::with_whole(None, 7))
    }

    #[test]
    fn growths_stay_ordered_while_one_transfer_is_in_flight() {
        let mut assets = assets();
        assets.grow_with_reserve(Arc::new(render_desc::Growth::default()), 8, 0, 12);
        assets.grow_with_reserve(Arc::new(render_desc::Growth::default()), 9, 0, 20);
        assert!(assets.take_growth(6).is_none());
        assert_eq!(assets.take_growth(7).map(|(generation, _)| generation), Some(8));
        assert_eq!(assets.growths.len(), 1);
        assert_eq!(assets.take_growth(7).map(|(generation, _)| generation), Some(9));
        assert!(assets.take_growth(7).is_none());
        assert_eq!(assets.take_reserve(7), Some((0, 20)));
    }

    fn empty_compiled() -> Arc<render_desc::Compiled> {
        struct NoImages;
        impl render_desc::ImageLoader for NoImages {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> {
                panic!("the empty description has no images")
            }
        }
        let description = render_desc::Description::parse(r#"
            grid = { layout = "orthogonal", step = [1, 1] }
        "#).unwrap();
        Arc::new(description.compile(&NoImages, 16).unwrap())
    }

    #[test]
    fn cpu_cursor_art_is_available_before_upload_and_survives_pixel_release() {
        let assets = Assets::new(empty_compiled());
        let before = assets.0.whole.load().unwrap();
        assert!(before.compiled.is_some());
        assert_eq!(assets.uploaded_generation(), 0);
        assets.uploaded(1, 1);
        let after = assets.0.whole.load().unwrap();
        assert!(after.compiled.is_none());
        assert!(Arc::ptr_eq(&before.cursor_art, &after.cursor_art));
    }

    #[test]
    fn cpu_art_publication_does_not_acknowledge_gpu_upload() {
        let assets = Assets::new(empty_compiled());
        assert_eq!(assets.latest(), 1);
        assert_eq!(assets.uploaded_generation(), 0);
        assets.uploaded(1, 1);
        assert_eq!(assets.uploaded_generation(), 1);
        assets.grow(Arc::new(render_desc::Growth::default()), 2);
        assert_eq!(assets.latest(), 2);
        assert_eq!(assets.uploaded_generation(), 1);
        assets.uploaded(1, 2);
        assert_eq!(assets.uploaded_generation(), 2);
    }

    #[test]
    fn replacement_preserves_acknowledgment_until_its_required_prefix_commits() {
        let mut assets = assets();
        assets.uploaded(7, 9);
        assets.set(empty_compiled(), 10);
        assert_eq!(assets.uploaded_generation(), 9);
        assets.grow(Arc::new(render_desc::Growth::default()), 11);
        assert_eq!(assets.uploaded_generation(), 9);
        assets.uploaded(10, 11);
        assert_eq!(assets.uploaded_generation(), 11);
        assert!(assets.whole().1.is_none());
        assets.set_with_reserve(empty_compiled(), 12, 0, 3);
        assert_eq!(assets.uploaded_generation(), 11);
        // Completion of an older transfer cannot release a newer whole's
        // raw pixels or move the acknowledgment backward.
        assets.uploaded(10, 10);
        assert_eq!(assets.uploaded_generation(), 11);
        assert!(assets.whole().1.is_some());
        assert_eq!(assets.clone().uploaded_generation(), 11);
    }

    #[test]
    fn reservations_coalesce_only_with_their_whole_atlas() {
        let mut assets = assets();
        assets.reserve(3, 12);
        assets.reserve(2, 20);
        assert_eq!(assets.take_reserve(8), None);
        assert_eq!(assets.take_reserve(7), Some((3, 20)));
        assets.reserve(8, 24);
        assets.publish(None, 10, (0, 0));
        assets.reserve(2, 4);
        assert_eq!(assets.take_reserve(7), None);
        assert_eq!(assets.take_reserve(10), Some((2, 4)));
    }

    #[test]
    fn consuming_a_reservation_does_not_make_repeated_snapshots_new_requests() {
        let mut assets = assets();
        assets.reserve(1, 147);
        assert_eq!(assets.take_reserve(7), Some((1, 147)));
        for _ in 0..100 {
            assets.reserve(1, 147);
            assets.reserve(0, 10);
            assert_eq!(assets.take_reserve(7), None);
        }
        assets.reserve(2, 10);
        assert_eq!(assets.take_reserve(7), Some((2, 147)));
    }

    #[test]
    fn a_new_whole_epoch_reuses_capacity_even_at_the_same_generation() {
        let mut assets = assets();
        assets.reserve(1, 147);
        assert_eq!(assets.take_reserve(7), Some((1, 147)));
        assets.set_with_reserve(empty_compiled(), 7, 1, 147);
        assert_eq!(assets.take_reserve(7), Some((1, 147)));
        assets.set_with_reserve(empty_compiled(), 8, 0, 3);
        assert_eq!(assets.take_reserve(8), Some((0, 3)));
    }

    #[test]
    fn a_replacement_drops_growths_and_reservations_of_the_whole_it_replaces() {
        let mut assets = assets();
        assets.grow_with_reserve(Arc::new(render_desc::Growth::default()), 8, 4, 4);
        // Not received yet when replaced, even at the same generation.
        assets.set_with_reserve(empty_compiled(), 7, 1, 0);
        assets.grow(Arc::new(render_desc::Growth::default()), 9);
        assert_eq!(assets.whole().0, 7);
        assert_eq!(assets.take_growth(7).map(|(generation, _)| generation), Some(9));
        assert!(assets.take_growth(7).is_none());
        assert_eq!(assets.take_reserve(7), Some((1, 0)));
        assert_eq!(assets.latest(), 9);
    }

    #[test]
    fn growths_from_another_thread_arrive_in_order_without_waiting() {
        let mut inbox = assets();
        let producer = inbox.assets.clone();
        let sender = std::thread::spawn(move || {
            for generation in 8..200 {
                producer.grow(Arc::new(render_desc::Growth::default()), generation);
                if generation % 50 == 0 {
                    producer.set(empty_compiled(), generation * 1000);
                }
            }
        });
        let mut seen = Vec::new();
        loop {
            let finished = sender.is_finished();
            let generation = inbox.generation();
            while let Some((g, _)) = inbox.take_growth(generation) {
                seen.push((generation, g));
            }
            if finished { break }
            std::thread::yield_now();
        }
        sender.join().unwrap();
        // Ordered within each whole, and each growth belongs to the whole
        // published before it.
        assert!(seen.windows(2).all(|w| w[0].0 != w[1].0 || w[0].1 < w[1].1));
        assert!(seen.iter().all(|&(whole, g)| whole == if g <= 50 { 7 } else { (g - 1) / 50 * 50_000 }));
        assert_eq!(inbox.generation(), 150_000);
        assert_eq!(seen.last().map(|s| s.1), Some(199));
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
    let mut world = scene_cells(compiled, scene);
    world.free = scene_free_arranged(compiled, scene, view_cameras, overlay_cameras);
    world
}

/// The scene's cells as the renderer's world (no free sprites).
pub fn scene_cells(compiled: &Compiled, scene: &Scene) -> TileWorld {
    let s = scene.shape;
    let mut world = compiled.world(s.cols.max(1), s.rows.max(1), s.wrap_x, s.wrap_y);
    let cells = (s.cols * s.rows) as usize;
    for (i, layer) in scene.layers.iter().enumerate().take(compiled.layers.len()) {
        let values = &layer.values[..layer.values.len().min(cells)];
        let phases = layer.phases.as_ref().map(|v| &v[..v.len().min(cells)]);
        let offsets = layer.offsets.as_ref().map(|v| &v[..v.len().min(cells)]);
        world.set_layer_cells(tile_render::LayerId(i), values, phases, offsets);
    }
    scene_slots(compiled, scene, &mut world);
    world
}

/// The scene's slots that `world` holds, in its slot order: (scene slot
/// index, world slot). Those of layers the description does not have, or
/// of another number of cells, are left out.
fn slot_map(compiled: &Compiled, scene: &Scene) -> Vec<(usize, tile_render::SlotId)> {
    let cells = (scene.shape.cols * scene.shape.rows) as usize;
    let held = scene.slots.iter().enumerate().filter(|(_, s)| (s.layer as usize) < compiled.layers.len() && s.cells.values.len() == cells);
    held.enumerate().map(|(k, (i, _))| (i, tile_render::SlotId(k as u32))).collect()
}

/// Makes `world`'s data slots and bindings the scene's (see
/// docs/per-view-layers.md): every slot of a world built otherwise freed,
/// the scene's added in its order (`slot_map`), filled, and bound as its
/// bindings say. For when which slots there are changes (a screen of a
/// view's own opens or closes): their cells' changes go cell by cell
/// ([`scene_slot_cells_changed`]).
pub fn scene_slots(compiled: &Compiled, scene: &Scene, world: &mut TileWorld) {
    let old: Vec<_> = world.slots().map(|(s, _)| s).collect();
    for s in old.into_iter().rev() {
        world.free_slot(s);
    }
    let cols = scene.shape.cols.max(1);
    let map = slot_map(compiled, scene);
    for &(i, _) in &map {
        let slot = &scene.slots[i];
        let (layer, spec) = (tile_render::LayerId(slot.layer as usize), &compiled.layers[slot.layer as usize]);
        let id = world.add_slot(layer);
        for (cell, &v) in slot.cells.values.iter().enumerate().filter(|(_, v)| **v != 0) {
            world.set_in(id, cell as u32 % cols, cell as u32 / cols, v);
        }
        if let (Some(phases), true) = (&slot.cells.phases, spec.per_cell_phase) {
            for (cell, &p) in phases.iter().enumerate().take(slot.cells.values.len()).filter(|(_, p)| **p != 0) {
                world.set_phase_in(id, cell as u32 % cols, cell as u32 / cols, p);
            }
        }
        if let (Some(offsets), true) = (&slot.cells.offsets, spec.per_cell_offset) {
            for (cell, &o) in offsets.iter().enumerate().take(slot.cells.values.len()).filter(|(_, o)| **o != (0, 0)) {
                world.set_offset_in(id, cell as u32 % cols, cell as u32 / cols, o);
            }
        }
    }
    for b in &scene.bindings {
        for &(layer, slot) in &b.slots {
            let held = map.iter().find(|(i, _)| scene.slots[*i].id == slot).filter(|(i, _)| scene.slots[*i].layer == layer);
            if let Some(&(_, id)) = held {
                world.bind(b.id, tile_render::LayerId(layer as usize), Some(id));
            }
        }
    }
}

/// Writes the values of the scene's slot `cells` (slot id, cell index)
/// into `world`, whose slots are the scene's ([`scene_slots`]).
pub fn scene_slot_cells_changed(compiled: &Compiled, scene: &Scene, world: &mut TileWorld, cells: &[(u32, u32)]) {
    let cols = scene.shape.cols.max(1);
    let map = slot_map(compiled, scene);
    for &(slot, cell) in cells {
        let Some(&(i, id)) = map.iter().find(|(i, _)| scene.slots[*i].id == slot) else { continue };
        let Some(&v) = scene.slots[i].cells.values.get(cell as usize) else { continue };
        world.set_in(id, cell % cols, cell / cols, v);
    }
}

/// Writes the scene's `cells` (layer, cell index: value, phase, offset)
/// into `world`, made from an earlier version of it of the same shape
/// (`scene_cells`).
pub fn scene_cells_changed(compiled: &Compiled, scene: &Scene, world: &mut TileWorld, cells: &[(u32, u32)]) {
    let s = scene.shape;
    let n = s.cols * s.rows;
    for &(layer, cell) in cells {
        let (Some(l), Some(spec)) = (scene.layers.get(layer as usize), compiled.layers.get(layer as usize)) else { continue };
        if cell >= n || cell as usize >= l.values.len() {
            continue;
        }
        let (id, col, row) = (tile_render::LayerId(layer as usize), cell % s.cols, cell / s.cols);
        world.set(id, col, row, l.values[cell as usize]);
        if let (Some(p), true) = (&l.phases, spec.per_cell_phase) {
            world.set_phase(id, col, row, p.get(cell as usize).copied().unwrap_or(0));
        }
        if let (Some(o), true) = (&l.offsets, spec.per_cell_offset) {
            world.set_offset(id, col, row, o.get(cell as usize).copied().unwrap_or((0, 0)));
        }
    }
}

/// The renderer's free sprites of `scene`: the world's sprites (in every
/// view) and each view's overlay, scene view `i`'s drawn by camera
/// `view_cameras[i]` (none: not drawn), or for the views
/// `overlay_cameras` lists, each sprite by the camera it gives.
pub fn scene_free_arranged(compiled: &Compiled, scene: &Scene, view_cameras: &[Option<u32>], overlay_cameras: &[(usize, Vec<u32>)]) -> Vec<tile_render::PlacedSprite> {
    // The camera showing scene view `id`, if one does.
    let camera_of = |id: u32| scene.views.iter().zip(view_cameras).find(|(v, _)| v.id == id).and_then(|(_, c)| *c);
    let mut free: Vec<_> = scene.sprites.iter().map(|s| placed_sprite(compiled, s, None, s.not_in_view.and_then(camera_of))).collect();
    for (i, (v, camera)) in scene.views.iter().zip(view_cameras).enumerate() {
        match overlay_cameras.iter().find(|(view, _)| *view == i) {
            Some((_, cameras)) => free.extend(v.overlay.iter().zip(cameras).map(|(s, c)| placed_sprite(compiled, s, Some(*c), None))),
            None => {
                if let Some(c) = camera {
                    free.extend(v.overlay.iter().map(|s| placed_sprite(compiled, s, Some(*c), None)));
                }
            }
        }
    }
    free
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
    let available = scene_prefix::sprite_available(compiled, s);
    tile_render::PlacedSprite {
        set: if available { tile_render::SpriteSetId::from_raw(s.set) } else { tile_render::SpriteSetId::EMPTY },
        x: s.x,
        y: s.y,
        screen: s.screen,
        after_pass: s.after_pass as usize,
        phase: s.phase,
        ticks_per_frame: s.ticks_per_frame.max(1),
        blend: match s.blend { scene::SpriteBlend::Alpha => tile_render::Blend::from_scale_den(s.scale_den, s.tint), scene::SpriteBlend::PaletteAlpha256 => tile_render::Blend::NativePaletteAlpha },
        clip: if available { s.clip } else { Some([0; 4]) },
        view: view.or(not_in),
        except_view: view.is_none() && not_in.is_some(),
        zoom_art: match s.zoom_art {
            1 => tile_render::ZoomArt::Low,
            2 => tile_render::ZoomArt::Zoomed { zoom: (u32::from(s.zoom.0), u32::from(s.zoom.1)), offset: s.zoom_offset },
            _ => tile_render::ZoomArt::Any,
        },
        text: (available && s.solid.is_none()).then(|| s.glyph.and_then(|g| outline_glyph(compiled, &g, s.tint))).flatten(),
        palette: if available { s.palette } else { 0 },
        category: s.category,
        solid: available.then_some(s.solid).flatten().map(|r| tile_render::SolidRect { width: r.width, height: r.height, rgba: r.rgba }),
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
                target_clip: None,
                lens: None,
                cells: !v.interface_only,
                screen_shift: (0, 0),
                zoomed_art: false,
                stacked: false,
                hairline: false,
                downscale: v.downscale.max(1),
                hidden_categories: v.hidden_categories,
                background_sampling: tile_render::BackgroundSampling::Dither,
                // (Its own fog of war, say: the slots its binding names.)
                binding: v.binding,
                on_target: false,
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
/// A light line two pixels wide (the keyboard focus's outline; once the
/// divider's): a column or a row, its top-left at the anchor.
pub const DIVIDER_SPRITE: &str = "Viewer@divider_v/0";
pub const DIVIDER_ROW_SPRITE: &str = "Viewer@divider_h/0";
pub const ARROW_SPRITE: &str = "Viewer@arrow/0";
pub const ARROW_SHADOW: &str = "Viewer@arrow_shadow";

/// The markers as world sprites on top of everything (but the world
/// cursor, which `cursor` draws). The arrow gently bobs when enabled
/// (`time`: since the viewer started). The markers' points are in
/// the pixels of views showing the world `downscale` times smaller; world
/// sprites are placed at full size.
pub fn marker_sprites(compiled: &Compiled, markers: &[layout::Marker], downscale: u32, time: Duration, bobbing: bool) -> Vec<tile_render::PlacedSprite> {
    markers
        .iter()
        .flat_map(|m| {
            let (name, (x, y)) = match *m {
                layout::Marker::Cursor(_) => return Vec::new(),
                layout::Marker::Focus(p) => (ARROW_SPRITE, p),
            };
            let Some((set, _)) = compiled.set(name) else { return Vec::new() };
            let n = downscale.max(1) as i32;
            let bob = if bobbing { arrow_bob_offset(time) } else { 0.0 };
            let offset = bob.round() as i32;
            let phase = (bob*4.0).round() as i32;
            let (set, offset) = if bobbing {
                compiled.set(&format!("Viewer@arrow_bob/{phase}/0")).map(|(set, _)| (set, 0)).unwrap_or((set, offset))
            } else { (set, 0) };
            let mut sprites = Vec::with_capacity(2);
            if let Some((shadow, _)) = compiled.set(&format!("{ARROW_SHADOW}/{}/0", bob.round() as i32)) {
                // The shadow stays on the ground. Higher arrows cast a
                // smaller, fainter patch; only the arrow's origin bobs.
                sprites.push(tile_render::PlacedSprite { category: viewer_settings::clean_map::UNITS,
                    ..tile_render::PlacedSprite::new(shadow, x*n, y*n) });
            }
            sprites.push(tile_render::PlacedSprite { category: viewer_settings::clean_map::UNITS,
                ..tile_render::PlacedSprite::new(set, x * n, y * n + offset) });
            sprites
        })
        .collect()
}

/// Full-size world pixels; cameras apply world zoom and pixel scale once.
pub fn arrow_bob_offset(time: Duration) -> f32 {
    (std::f64::consts::TAU * (time.as_secs_f64() % 1.3) / 1.3).sin() as f32 * 3.0
}

/// Only animate when at least one world camera can show the marker. Static
/// markers stay identical, allowing the renderer to reuse its last picture.
pub fn arrow_bobbing_visible(bobbing: bool, markers: &[layout::Marker], cameras: &[Camera], downscale: u32) -> bool {
    bobbing && markers.iter().any(|m| {
        let layout::Marker::Focus((x, y)) = *m else { return false };
        let n = downscale.max(1) as f64;
        cameras.iter().any(|c| {
            if !c.cells || c.hidden_categories & viewer_settings::clean_map::UNITS != 0 { return false; }
            let d = c.downscale.max(1) as f64;
            let z = c.zoom.0 as f64 / c.zoom.1.max(1) as f64;
            let (left, top) = (((x as f64*n - 16.0)/d - c.x as f64)*z, ((y as f64*n - 115.0)/d - c.y as f64)*z);
            left < c.viewport.0 as f64 && top < c.viewport.1 as f64 && left + 33.0*z/d > 0.0 && top + 54.0*z/d > 0.0
        })
    })
}

#[cfg(test)]
mod accessibility_arrow_tests {
    use super::*;
    #[test]
    fn bobbing_uses_elapsed_time_and_is_bounded_and_periodic() {
        for ms in 0..5200 {
            let t = Duration::from_millis(ms);
            assert!(arrow_bob_offset(t).abs() <= 3.0);
            assert!((arrow_bob_offset(t) - arrow_bob_offset(t+Duration::from_millis(1300))).abs() < 0.0001);
        }
        assert!((arrow_bob_offset(Duration::from_millis(325)) - 3.0).abs() < 0.0001);
        assert!((arrow_bob_offset(Duration::from_millis(975)) + 3.0).abs() < 0.0001);
    }
    #[test]
    fn redraw_changes_only_for_visible_bobbing_arrows() {
        struct NoFiles;
        impl render_desc::ImageLoader for NoFiles {
            fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<tile_render::RgbaImage>, String> { unreachable!() }
        }
        let desc = "grid = { layout = \"orthogonal\", step = [32,32] }\n[images.arrow]\nsource = \"gilded_arrow:248,194,56,255:33x48\"\n[sheets.arrow]\nimage = \"arrow\"\ncell = [33,48]\ngrid = [1,1]\nanchor = [-16,-112]\ngroup = \"Viewer@arrow\"\n";
        let shadows: String = (-3..=3).map(|offset| format!("\n[images.shadow{}]\nsource = \"marker_shadow:32,23,14,70:40x14\"\n[sheets.shadow{}]\nimage = \"shadow{}\"\ncell = [40,14]\ngrid = [1,1]\nanchor = [-12,-3]\ngroup = \"Viewer@arrow_shadow/{offset}\"\n", offset+3, offset+3, offset+3)).collect();
        let phases: String = (-12..=12).map(|phase| format!("\n[images.phase{}]\nsource = \"gilded_arrow:232,183,87,255:33x56@{phase}\"\n[sheets.phase{}]\nimage = \"phase{}\"\ncell = [33,56]\ngrid = [1,1]\nanchor = [-16,-116]\ngroup = \"Viewer@arrow_bob/{phase}\"\n", phase+12, phase+12, phase+12)).collect();
        let compiled = render_desc::Description::parse(&format!("{desc}{shadows}{phases}")).unwrap().compile(&NoFiles, 128).unwrap();
        let m = [layout::Marker::Focus((100, 160))];
        let cameras = [Camera::new((320, 240))];
        assert!(arrow_bobbing_visible(true, &m, &cameras, 1));
        assert!(!arrow_bobbing_visible(false, &m, &cameras, 1));
        assert!(!arrow_bobbing_visible(true, &[], &cameras, 1));
        assert!(!arrow_bobbing_visible(true, &[layout::Marker::Focus((1000, 160))], &cameras, 1));
        assert!(!arrow_bobbing_visible(true, &m, &[Camera { cells: false, ..cameras[0] }], 1));
        assert!(!arrow_bobbing_visible(true, &m, &[Camera { hidden_categories: viewer_settings::clean_map::UNITS, ..cameras[0] }], 1));
        let (a, b) = (Duration::from_millis(325), Duration::from_millis(975));
        assert_eq!(marker_sprites(&compiled, &m, 1, a, false), marker_sprites(&compiled, &m, 1, b, false));
        assert_ne!(marker_sprites(&compiled, &m, 1, a, true), marker_sprites(&compiled, &m, 1, b, true));
        // No blinking at any phase; native reduced positions return to full
        // world coordinates before the camera applies its own downscale.
        for ms in (0..3000).step_by(50) { assert_eq!(marker_sprites(&compiled, &m, 2, Duration::from_millis(ms), true).len(), 2); }
        let low = marker_sprites(&compiled, &m, 2, a, true);
        let high = marker_sprites(&compiled, &m, 2, b, true);
        assert_eq!((low[0].x, low[0].y), (high[0].x, high[0].y), "the shadow stays on the ground");
        assert_ne!(low[0].set, high[0].set, "shadow coverage changes with height");
        assert_eq!(low[1].y, 320, "motion is in the supersampled phase art");
        assert_ne!(marker_sprites(&compiled, &m, 1, Duration::ZERO, true)[1].set,
            marker_sprites(&compiled, &m, 1, Duration::from_millis(10), true)[1].set,
            "subpixel phases move before the integer origin advances");
        // The actual GPU retained-picture decision, not just marker math:
        // moving the arrow invalidates its world, a static/absent one does
        // not create further full redraws as the animation clock advances.
        let gpu = tile_render::Headless::new().unwrap();
        let mut renderer = tile_render::TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
        let mut world = compiled.world(1, 1, false, false);
        world.color = tile_render::ColorModel::Quantized { bits: 5 };
        renderer.set_world(&gpu.device, &world);
        let check = |renderer: &mut tile_render::TileRenderer, time, bobbing, markers: &[layout::Marker]| {
            renderer.set_markers(&gpu.device, &gpu.queue, &marker_sprites(&compiled, markers, 1, time, bobbing));
            gpu.render_views(renderer, &cameras, (320, 240), 0, [64, 72, 48, 255]);
        };
        check(&mut renderer, a, false, &m);
        check(&mut renderer, b, false, &m);
        assert!(renderer.reused());
        check(&mut renderer, a, true, &m);
        assert!(!renderer.reused());
        check(&mut renderer, b, true, &m);
        assert!(!renderer.reused());
        check(&mut renderer, a, true, &[]);
        check(&mut renderer, b, true, &[]);
        assert!(renderer.reused());
    }
}

/// The divider's glass (`layout::Frame::divider`, `layout::glass`): one
/// translucent strip a pixel across the gap each, tiled along it, in the
/// camera of the half of the gap it lies in; nothing without the art.
fn glass_sprites(compiled: &Compiled, strips: &[(u32, [i32; 4], bool, layout::Shade)]) -> Vec<tile_render::PlacedSprite> {
    let mut out = Vec::new();
    for &(camera, [x, y, w, h], vertical, shade) in strips {
        let Some((set, _)) = compiled.set(glass_sprite(shade, vertical)) else { continue };
        let clip = Some([x, y, x + w, y + h]);
        let len = if vertical { h } else { w };
        for k in (0..len).step_by(GLASS_TILE as usize) {
            let (px, py) = if vertical { (x, y + k) } else { (x + k, y) };
            out.push(tile_render::PlacedSprite { screen: true, view: Some(camera), clip, ..tile_render::PlacedSprite::new(set, px, py) });
        }
    }
    out
}

/// A dark band with a light inner half, sized by Layout in window pixels.
fn separator_sprites(pieces: &[(u32, [i32; 4])]) -> Vec<tile_render::PlacedSprite> {
    let mut out = Vec::new();
    for &(camera, [x, y, w, h]) in pieces {
        for (top, height, color) in [(y, h, 20), (y + h / 2, (h + 1) / 2, 205)] {
            out.push(tile_render::PlacedSprite { screen: true, view: Some(camera),
                solid: Some(tile_render::SolidRect { width: w as u32, height: height as u32, rgba: [color, color, color, 255] }),
                ..tile_render::PlacedSprite::new(tile_render::SpriteSetId::EMPTY, x, top) });
        }
    }
    out
}

/// How long a glass strip's art is along the divider.
const GLASS_TILE: i32 = 64;

/// The sprite set of a glass strip (side by side: a column; stacked: a
/// row), its top-left at the anchor.
pub fn glass_sprite(shade: layout::Shade, vertical: bool) -> &'static str {
    match (shade, vertical) {
        (layout::Shade::Light, true) => "Viewer@glass_light_v/0",
        (layout::Shade::Frost, true) => "Viewer@glass_frost_v/0",
        (layout::Shade::Shadow, true) => "Viewer@glass_shadow_v/0",
        (layout::Shade::Light, false) => "Viewer@glass_light_h/0",
        (layout::Shade::Frost, false) => "Viewer@glass_frost_h/0",
        (layout::Shade::Shadow, false) => "Viewer@glass_shadow_h/0",
    }
}

/// Light lines two pixels wide (the keyboard focus's outline), seen on
/// any background; nothing without the art.
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
    let (mut x0, mut y0, mut x1, mut y1) = if let Some(r) = s.solid {
        (s.x, s.y, s.x.saturating_add(r.width.min(i32::MAX as u32) as i32), s.y.saturating_add(r.height.min(i32::MAX as u32) as i32))
    } else {
        match s.glyph {
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
        }
    };
    if let Some([cx0, cy0, cx1, cy1]) = s.clip {
        (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// A placed screen sprite's bounds in the driver's pixels, before its
/// camera's scale and placement. Used to clip list pixels without moving
/// unrelated controls or the world beneath a production popup.
fn placed_rect(compiled: &Compiled, s: &tile_render::PlacedSprite) -> Option<[i32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = if let Some(r) = s.solid {
        (s.x, s.y, s.x.saturating_add(r.width.min(i32::MAX as u32) as i32), s.y.saturating_add(r.height.min(i32::MAX as u32) as i32))
    } else {
        match s.text {
            Some(g) => {
                let em = (g.em64 as i32 + 63) / 64;
                (s.x, s.y - em, s.x + g.advance.max(1), s.y + em / 3)
            }
            None => {
                let id = compiled.sets.frame(s.set, 0)?;
                let sprite = compiled.atlas.sprites.get(id.0 as usize)?;
                let (x0, y0) = (s.x + sprite.anchor_x, s.y + sprite.anchor_y);
                (x0, y0, x0 + sprite.width as i32, y0 + sprite.height as i32)
            }
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
/// Whether a system-pointer move that `routed` became takes the pointer
/// off the driver's interactables. Not while the mouse is captured, nor
/// while the snapping pointer is the pointer: the hidden system pointer's
/// motion is swallowed there, and the driver keeps the pointer on the
/// element snapped to (its hover look must stay).
fn motion_leaves_driver(captured: bool, snapping: bool, routed: &[Input]) -> bool {
    !captured && !snapping && !routed.iter().any(|i| matches!(i, Input::MouseMove { .. }))
}

fn interface_rects(compiled: &Compiled, scene: &Scene) -> Vec<[i32; 4]> {
    let parts = [scene.interface.roaming, scene.interface.popup, scene.interface.screen_view, scene.interface.capture_hint];
    let views = scene.views.iter().filter(|v| v.view.interface_only && !parts.contains(&Some(v.id)) && !scene.interface.is_popup_view(v.id));
    views.flat_map(|v| v.overlay.iter().filter(|s| s.glyph.is_none()).filter_map(|s| sprite_rect(compiled, s))).collect()
}

/// The active named panel's bounds, without the map's HUD behind it.
fn named_screen_content(compiled: &Compiled, scene: &Scene) -> Option<[i32; 4]> {
    scene.interface.screen.as_ref()?;
    if let Some(id) = scene.interface.screen_view {
        let v = scene.view(id)?.view;
        if v.viewport.0 > 0 && v.viewport.1 > 0 {
            return Some([v.origin.0, v.origin.1, v.viewport.0 as i32, v.viewport.1 as i32]);
        }
    }
    let city = scene.interface.screen.as_deref() == Some("City");
    let parts = [scene.interface.roaming, scene.interface.capture_hint, scene.interface.popup];
    let rects = scene.views.iter().filter(|v| v.view.interface_only && !parts.contains(&Some(v.id)) && !scene.interface.is_popup_view(v.id)
        && if city { v.view.camera_x == 0 && v.view.camera_y == 0 } else { v.view.menu })
        .flat_map(|v| v.overlay.iter().filter(|s| s.glyph.is_none()).filter_map(|s| sprite_rect(compiled, s)));
    let [l, t, r, b] = rects.map(|[x, y, w, h]| [x, y, x + w, y + h])
        .reduce(|a, r| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])])?;
    Some([l, t, r - l, b - t])
}

/// Writes run-time sprite pixels into the atlas; patches for sprites the
/// description does not have (yet) wait for one that has.
pub fn apply_patches<'p>(
    renderer: &mut TileRenderer,
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
    apply_patches(&mut renderer, &gpu.queue, compiled, &scene.patches);
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
    let frame = arranged_frame(compiled, scene, size, config);
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
    renderer.set_world(&gpu.device, &scene_world_for(compiled, scene, &frame.view_cameras));
    apply_patches(&mut renderer, &gpu.queue, compiled, &scene.patches);
    gpu.render_views(&mut renderer, &frame.cameras, size, scene.tick, config.clear)
}

/// Renders `scene` as a viewer window of `window` pixels shows it with
/// the user's `resolution` and `layout` settings: the canvas (the window
/// divided by the pixel scale), the driver's interface at the menu and
/// frame scales (`scaling::plan`, `scaling::arrange`). For tools that look
/// at how the interface is placed and sized.
pub fn render_planned(
    gpu: &tile_render::Headless,
    compiled: &Compiled,
    scene: &Scene,
    window: (u32, u32),
    resolution: &viewer_settings::Resolution,
    settings: &viewer_settings::Layout,
    config: &ViewerConfig,
) -> RgbaImage {
    let rect_of = |s: &scene::FreeSprite| sprite_rect(compiled, s);
    let plan = scaling::plan(window, scene.screen_size, resolution).for_scene(scene, rect_of);
    let mut layout = layout::Layout::default();
    layout.apply_settings(settings);
    layout.grid_step = compiled.step;
    layout.driver_screen = Some(plan.screen);
    layout.menu_scale = plan.menu.zoom;
    layout.hud_scale = plan.frame;
    layout.window_pixel_scale = plan.factor();
    layout.screen_content = named_screen_content(compiled, scene);
    let mut frame = layout.frame(scene, plan.canvas(), config.overhang, interface_rects(compiled, scene), Some(world_bounds(compiled, scene)));
    let arranged = scaling::arrange_panned(&plan, scene, &mut frame, &mut layout.menu_pan, rect_of);
    let mut renderer = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
    renderer.set_atlas(&gpu.device, &gpu.queue, &compiled.atlas);
    renderer.set_world(&gpu.device, &scene_world_arranged(compiled, scene, &frame.view_cameras, &arranged.overlay_cameras));
    apply_patches(&mut renderer, &gpu.queue, compiled, &scene.patches);
    gpu.render_views(&mut renderer, &frame.cameras, plan.canvas(), scene.tick, config.clear)
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
        desired_maximum_frame_latency: 2,
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

/// How a freshly opened viewer arranges `scene` in a window of `size` (the
/// user's settings left at their defaults).
pub fn arranged_frame(compiled: &Compiled, scene: &Scene, size: (u32, u32), config: &ViewerConfig) -> layout::Frame {
    let mut layout = layout::Layout::default();
    layout.grid_step = compiled.step;
    layout.frame(scene, size, config.overhang, interface_rects(compiled, scene), Some(world_bounds(compiled, scene)))
}

/// Opens the window and runs until it is closed, drawing what `display`
/// receives. Blocks the calling thread, which need not be the process's
/// main thread (the viewer usually runs inside someone else's process).
pub fn run(config: ViewerConfig, assets: Assets, display: Display) -> Result<(), ViewerError> {
    run_until(config, assets, display, Arc::new(std::sync::atomic::AtomicBool::new(false)))
}

/// Runs on the calling thread until closed or the host requests shutdown.
/// macOS callers must use the process main thread.
pub fn run_until(config: ViewerConfig, assets: Assets, display: Display, stop: Arc<std::sync::atomic::AtomicBool>) -> Result<(), ViewerError> {
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
    let cursor_proxy = proxy.clone();
    display.set_wake(move || {
        let _ = proxy.send_event(());
    });
    let mut app = new_app(config, assets, display, stop);
    app.cursor_builds.wake = Some(Arc::new(move || { let _ = cursor_proxy.send_event(()); }));
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// A viewer's state before its event loop runs, with the user's settings.
fn new_app(config: ViewerConfig, assets: Assets, display: Display, stop: Arc<std::sync::atomic::AtomicBool>) -> App {
    let mut assets = ArtInbox::new(assets);
    let (assets_version, whole) = assets.whole();
    let whole = whole.expect("a new description");
    let native_thread_init = config.native_thread_init;
    let mut app = App {
        stop,
        config,
        assets,
        assets_version,
        whole_version: assets_version,
        compiled: Arc::new(whole.without_pixels()),
        upload: Some(whole),
        display,
        shown_scene: Arc::new(Scene::default()),
        camera_arrival_reported: None,
        sprite_lifetimes: sprite_lifetimes::SpriteLifetimes::default(),
        unit_motions: unit_motions::UnitMotions::default(),
        pending_scene: None,
        future_metadata: None,
        drawable_scene: None,
        world_command_version: 0,
        pending_ui: None,
        next_frame_id: 1,
        worker_world_version: 0,
        input_revision: 0,
        preparing_inputs: None,
        pending_replay: None,
        gpu: None,
        drawn_seq: 0,
        patches: std::collections::BTreeMap::new(),
        input: input::InputState::default(),
        layout: layout::Layout::default(),
        drawn_views: Vec::new(),
        world: None,
        world_assets_version: 0,
        drawn_markers: Vec::new(),
        drawn_tick: 0,
        drawn_window: (0, 0),
        drawn_background: tile_render::BackgroundSampling::Dither,
        plan: None,
        drawn_overlay: Vec::new(),
        built_overlay: Vec::new(),
        overlay_camera: None,
        reported_size: (0, 0),
        reported_window: (0, 0, 0),
        sized_to_screen: None,
        fitted_on_screen: None,
        started: std::time::Instant::now(),
        drawn_frame: None,
        cached_picture: None,
        drawn_at: None,
        settings: viewer_settings::Settings::default(),
        error: None,
        snap: snap::SnapInput::default(),
        fps: fps::Overlay::default(),
        host_status: host_status::Indicator::default(),
        settings_preview: settings_preview::Preview::default(),
        snap_grabbed: false,
        cursor_art: cursor::Art::default(),
        os_cursor: system_cursor::SystemCursor::default(),
        pending_cursor_art: None,
        custom_cursors: std::collections::HashMap::new(),
        system_cursor_size: cursor::NOMINAL,
        cursor_builds: CursorBuilds { native_thread_init, ..Default::default() },
        pointer_presentation: system_cursor::Presentation::default(),
        capture_icon: false,
        pointer_inside: true,
        pointer_since: None,
        drawn_cursor: Vec::new(),
        latency: cursor::Latency::default(),
        latency_logged: Instant::now(),
        frames_presented: 0,
        diagnostics: profile::env_on("TILE_VIEWER_PROFILE").then(render_worker::AppDiagnostics::new),
        frame_log: frame_times().then(fps::FrameLog::default),
        beside_drawn: None,
        unlock_pending: None,
        machine: cursor::Machine::default(),
        system_pointer: (0, 0),
        cursor: scene::Cursor::Default,
        presented: false,
        woken_idle: false,
        cursor_cameras: None,
        frame_clock: FrameClock::default(),
        map_drawn: false,
        interface: None,
        profile: profile::env_on("TILE_VIEWER_PROFILE").then(|| profile::Profile::new("viewer", Duration::from_secs(5))),
        received_seq: 0,
        constraints: constraints::Constraints::default(),
        feedback: looks::Feedback::from_env(),
        drawn_looks: Vec::new(),
        base_free: Vec::new(),
        spans: Vec::new(),
        look_scene: Arc::new(looks::LookScene::default()),
        menu_scroll: smooth_menus::SmoothMenus::default(),
    };
    // The arrangement to start with: the user's settings.
    app.set_settings(load_settings());
    app
}

/// How a viewer started in a host's application ended: closed (`Ok`) or
/// failed. Called on the main thread, once.
#[cfg(target_os = "macos")]
pub type OnExit = Box<dyn FnOnce(Result<(), ViewerError>) + Send>;

/// Opens the window in the `NSApplication` a host already runs on the
/// process's main thread (Wine's, for a native library in a Wine program)
/// and returns at once; any thread may call it. The window's events and
/// frames run on the main thread, scheduled through its dispatch queue: the
/// host's run loop drives them. Waits up to `wait` for the host to start
/// its application. `on_exit` reports the end.
#[cfg(target_os = "macos")]
pub fn start_in_host_application(config: ViewerConfig, assets: Assets, display: Display, wait: Duration, on_exit: OnExit) {
    host_app::start(Box::new(move || {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        let mut builder = EventLoop::builder();
        // (The host's own windows may stay hidden: ours makes the
        // application a regular one with a Dock icon.)
        builder.with_attached_application(true).with_activation_policy(ActivationPolicy::Regular);
        let event_loop = builder.build()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        let proxy = event_loop.create_proxy();
        let cursor_proxy = proxy.clone();
        display.set_wake(move || {
            let _ = proxy.send_event(());
        });
        let mut app = new_app(config, assets, display, Arc::new(std::sync::atomic::AtomicBool::new(false)));
        app.cursor_builds.wake = Some(Arc::new(move || { let _ = cursor_proxy.send_event(()); }));
        Ok((event_loop, app))
    }), wait, on_exit);
}

#[cfg(target_os = "macos")]
mod host_app {
    use super::*;

    /// An [`App`] run by the host's application; reports its end.
    pub(super) struct Attached {
        pub app: App,
        pub on_exit: Option<OnExit>,
    }

    impl ApplicationHandler for Attached {
        fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
            self.app.new_events(event_loop, cause)
        }
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.app.resumed(event_loop)
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.app.window_event(event_loop, id, event)
        }
        fn device_event(&mut self, event_loop: &ActiveEventLoop, id: winit::event::DeviceId, event: winit::event::DeviceEvent) {
            self.app.device_event(event_loop, id, event)
        }
        fn user_event(&mut self, event_loop: &ActiveEventLoop, (): ()) {
            self.app.user_event(event_loop, ())
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.app.about_to_wait(event_loop)
        }
        fn exiting(&mut self, event_loop: &ActiveEventLoop) {
            self.app.exiting(event_loop);
            if let Some(on_exit) = self.on_exit.take() {
                on_exit(self.app.error.take().map_or(Ok(()), Err));
            }
        }
    }

    type Make = Box<dyn FnOnce() -> Result<(EventLoop<()>, App), ViewerError> + Send>;

    /// Runs `make` on the main thread once the host's application runs
    /// (checked every `RETRY`, for at most `wait`), then hands the viewer to
    /// the host's run loop.
    pub(super) fn start(make: Make, wait: Duration, on_exit: OnExit) {
        let deadline = Instant::now() + wait;
        dispatch2::DispatchQueue::main().exec_async(move || attempt(make, deadline, on_exit));
    }

    const RETRY: Duration = Duration::from_millis(50);

    fn attempt(make: Make, deadline: Instant, on_exit: OnExit) {
        if !winit::platform::macos::host_application_running() {
            if Instant::now() >= deadline {
                on_exit(Err(ViewerError::Host("no NSApplication runs on the main thread".into())));
                return;
            }
            let when = dispatch2::DispatchTime::NOW.time(RETRY.as_nanos() as i64);
            let _ = dispatch2::DispatchQueue::main().after(when, move || attempt(make, deadline, on_exit));
            return;
        }
        match make() {
            Ok((event_loop, app)) => {
                use winit::platform::macos::EventLoopExtMacOSAttached;
                log::info!("viewer: attached to the host's application on the main thread");
                // (On failure the handler, with `on_exit`, is dropped: its
                // owner's guards report the end.)
                if let Err(e) = event_loop.run_app_attached(Attached { app, on_exit: Some(on_exit) }) {
                    log::error!("viewer: {e}");
                }
            }
            Err(e) => on_exit(Err(e)),
        }
    }
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

/// Consuming an empty dirty set is a read-only operation on a published
/// world. Only real cell changes need ownership of its record storage.
fn take_world_dirty(world: &mut Arc<TileWorld>) -> tile_render::Dirty {
    if *world.dirty() == tile_render::Dirty::None { tile_render::Dirty::None }
    else { Arc::make_mut(world).take_dirty() }
}

fn worker_wake(event_loop: Option<&Arc<dyn Fn() + Send + Sync>>, redraw: impl Fn() + Send + Sync + 'static) -> Arc<dyn Fn() + Send + Sync> {
    // Production factories install the event-loop proxy before init. The
    // redraw fallback is for manually constructed/test Apps only.
    event_loop.cloned().unwrap_or_else(|| Arc::new(redraw))
}

fn frame_input(preparing: &mut Option<Vec<Input>>, display: &Display, input: Input) {
    if let Some(inputs) = preparing { inputs.push(input); } else { display.send_input(input); }
}

struct Gpu {
    window: Arc<Window>,
    config: wgpu::SurfaceConfiguration,
    worker: render_worker::Worker,
    ready: bool,
    config_revision: u64,
    software: bool,
    reconfigured: bool,
    present_modes: Vec<wgpu::PresentMode>,
    #[cfg(target_os = "macos")]
    bitmap: Option<presentation::CpuBitmap>,
}


struct App {
    stop: Arc<std::sync::atomic::AtomicBool>,
    config: ViewerConfig,
    assets: ArtInbox,
    /// The generation drawn (the whole description's, or a growth's of
    /// it), and that whole description's.
    assets_version: u64,
    whole_version: u64,
    /// Immutable committed metadata shared directly with the worker's
    /// publication. Pixel storage remains on the GPU.
    compiled: Arc<Compiled>,
    /// The first description's pixels, until the GPU is there to take them.
    upload: Option<Arc<Compiled>>,
    display: Display,
    /// Last scene adopted with compatible active art. Kept live for local
    /// cursor, hover, layout and FPS drawing while newer art is prepared.
    shown_scene: Arc<Scene>,
    camera_arrival_reported: Option<u64>,
    sprite_lifetimes: sprite_lifetimes::SpriteLifetimes,
    unit_motions: unit_motions::UnitMotions,
    /// One immutable future snapshot prevents later optional art generations
    /// from moving the adoption target. Display keeps coalescing newer work.
    pending_scene: Option<PendingScene>,
    /// Same-epoch command/layout metadata, independent of pixel upload.
    future_metadata: Option<scene_prefix::Metadata>,
    drawable_scene: Option<scene_prefix::DrawableScene>,
    world_command_version: u64,
    pending_ui: Option<ui_submission::PendingUi>,
    next_frame_id: u64,
    worker_world_version: u64,
    input_revision: u64,
    preparing_inputs: Option<Vec<Input>>,
    pending_replay: Option<(u64, u64, Instant, Duration)>,
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
    world: Option<Arc<TileWorld>>,
    /// Art generation paired with the renderer's currently built world.
    /// An intermediate atlas commit may precede its compatible scene.
    world_assets_version: u64,
    drawn_markers: Vec<tile_render::PlacedSprite>,
    /// The animation tick and window size drawn last.
    drawn_tick: u32,
    drawn_window: (u32, u32),
    /// The background sampling drawn last (`settings.resolution.background_sampling`).
    drawn_background: tile_render::BackgroundSampling,
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
    /// The arrangement drawn last (drawn again while a scene waits for its
    /// art).
    drawn_frame: Option<layout::Frame>,
    /// Mapping and copies of the last compatible picture. Its composed
    /// pixels can be replayed while a new atlas awaits its scene.
    cached_picture: Option<CachedPicture>,
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
    /// Whether the driver's host answers, as last shown (`host_status`).
    host_status: host_status::Indicator,
    settings_preview: settings_preview::Preview,
    /// The cursor art kept for the system pointer, the system cursors made
    /// of it (by set), what the system pointer shows, and whether it is the
    /// capture pointer (the layout's `PointerIcon::Capture`).
    cursor_art: cursor::Art,
    os_cursor: system_cursor::SystemCursor,
    /// A committed whole atlas's cursor pixels, activated only when its
    /// scene becomes drawable. Old custom cursors remain usable meanwhile.
    pending_cursor_art: Option<(u64, cursor::Art)>,
    custom_cursors: std::collections::HashMap<(cursor::Shown, cursor::Scale), winit::window::CustomCursor>,
    /// The operating system's cursor size (window pixels; the cursor size
    /// setting's default). Normal uncaptured pointers are drawn by the OS.
    system_cursor_size: u32,
    /// System cursors being made (off the event loop: the picture never
    /// waits for one).
    cursor_builds: CursorBuilds,
    pointer_presentation: system_cursor::Presentation,
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
    diagnostics: Option<render_worker::AppDiagnostics>,
    /// Frame times for the log (`TILE_VIEWER_FRAME_TIMES`), and the world
    /// last drawn beside a screen (`scene::Beside`, for the log).
    frame_log: Option<fps::FrameLog>,
    beside_drawn: Option<(scene::WorldShape, Option<[i32; 4]>)>,
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
    /// Animation/retry timing and the optional maximum presentation rate.
    frame_clock: FrameClock,
    /// Whether a scene with the world in a view was drawn yet.
    map_drawn: bool,
    /// What the scene's interface covers (`interface_rects`), for the scene
    /// version and art it was found for.
    interface: Option<((u64, u64), Vec<[i32; 4]>)>,
    /// Where the time goes (`TILE_VIEWER_PROFILE=1`), logged every 5 s.
    profile: Option<profile::Profile>,
    /// The scene version last received (scenes counted for the profile).
    received_seq: u64,
    /// The display-independent pointer controller.
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
    look_scene: Arc<looks::LookScene>,
    /// Pixel transitions and matching pointer maps of scrollable menu lists.
    menu_scroll: smooth_menus::SmoothMenus,
}

/// Adds the time since `t` to span `name` of the profile, if one is kept,
/// and restarts `t`.
fn mark(profile: &mut Option<profile::Profile>, name: &str, t: &mut Instant) {
    let now = Instant::now();
    if let Some(p) = profile {
        p.add(name, now - *t);
    }
    *t = now;
}

/// What happens to the system pointer once a frame went out after it was
/// moved (on Wayland a captured pointer is moved by a hint the window's
/// next frame applies when it is let go).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Unlock {
    /// The mouse is let go.
    Release,
    /// It shows at the world cursor, kept inside its view, moving it
    /// (`cursor::Machine::edge`).
    Edge,
}

/// The longest time between frames (the animation clock) where the
/// display's refresh rate is not known.
const DEFAULT_FRAME: Duration = Duration::from_millis(16);

/// The animation clock permits immediate input frames unless the user set
/// a maximum rate. Only a presented frame starts the cap's next interval;
/// unsuccessful draw attempts merely postpone the next animation retry.
struct FrameClock {
    interval: Duration,
    capped: bool,
    last_presented: Option<Instant>,
    next_attempt: Option<Instant>,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self { interval: DEFAULT_FRAME, capped: false, last_presented: None, next_attempt: None }
    }
}

impl FrameClock {
    fn configure(&mut self, interval: Duration, capped: bool) {
        if (self.interval, self.capped) == (interval, capped) { return; }
        self.interval = interval;
        self.capped = capped;
        // Recompute from the last presentation, so changing or disabling a
        // cap never retains the previous cap's deadline.
        self.next_attempt = self.last_presented.map(|at| at + interval);
    }

    fn allowed(&self, now: Instant) -> bool {
        !self.capped || self.last_presented.is_none_or(|at| now >= at + self.interval)
    }

    fn ready(&self, now: Instant, requested: bool) -> bool {
        self.allowed(now) && (requested || self.next_attempt.is_none_or(|at| now >= at))
    }

    /// Accepts either a compositor redraw or the viewer's own request.
    /// Rejected requests leave the presentation and retry clocks alone.
    fn request(&mut self, now: Instant) -> bool {
        if !self.allowed(now) {
            return false;
        }
        self.next_attempt = Some(now + self.interval);
        true
    }

    fn presented(&mut self, now: Instant) {
        self.last_presented = Some(now);
        // The surface already paces VSync. Completing an uncapped frame
        // must not add another refresh interval after its preparation and
        // presentation. Keep the deadline established by the draw attempt.
        if self.capped || self.next_attempt.is_none() {
            self.next_attempt = Some(now + self.interval);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_attempt.map(|retry| match (self.capped, self.last_presented) {
            (true, Some(at)) => retry.max(at + self.interval),
            _ => retry,
        })
    }

    fn wake_at(&self, now: Instant) -> Option<Instant> {
        if self.interval.is_zero() { None } else { Some(self.deadline().unwrap_or(now + self.interval)) }
    }
}

#[cfg(test)]
mod frame_clock_tests {
    use super::{Duration, FrameClock, Instant};

    #[test]
    fn update_and_redraw_storms_obey_the_same_cap() {
        let start = Instant::now();
        let interval = Duration::from_millis(4);
        let mut clock = FrameClock::default();
        clock.configure(interval, true);
        let mut presentations = Vec::new();
        for event in 0..=256 {
            let now = start + Duration::from_micros(event * 125);
            // Interleave driver updates and compositor/pointer redraws.
            let draw = if event % 3 == 0 {
                clock.ready(now, true) && clock.request(now)
            } else {
                clock.request(now)
            };
            if draw {
                presentations.push(now);
                clock.presented(now);
            }
        }
        assert_eq!(presentations.len(), 9);
        assert!(presentations.windows(2).all(|pair| pair[1] - pair[0] >= interval));
    }

    #[test]
    fn early_redraws_do_not_move_the_deadline() {
        let start = Instant::now();
        let interval = Duration::from_millis(10);
        let mut clock = FrameClock::default();
        clock.configure(interval, true);
        clock.presented(start);
        for millis in 1..10 {
            let now = start + Duration::from_millis(millis);
            assert!(!clock.ready(now, true));
            assert!(!clock.request(now));
            assert_eq!(clock.wake_at(now), Some(start + interval));
        }
        assert!(clock.ready(start + interval, false));
        assert!(clock.request(start + interval));
    }

    #[test]
    fn graphics_settings_pace_high_refresh_and_reconfigure_without_stale_deadlines() {
        let start = Instant::now();
        let mut clock = FrameClock::default();
        for fps in [30, 120, 240, 360, 480] {
            for mode in [wgpu::PresentMode::AutoVsync, wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox] {
                let interval = super::presentation_interval(fps, mode, Some(60_000));
                assert!(interval.as_secs_f64() >= 1.0 / f64::from(fps));
                clock.configure(interval, true);
                clock.presented(start);
                assert!(!clock.ready(start + interval - Duration::from_nanos(1), true));
                assert_eq!(clock.wake_at(start), Some(start + interval));
                assert!(clock.ready(start + interval, true));
            }
        }
        clock.configure(super::presentation_interval(0, wgpu::PresentMode::Immediate, None), false);
        assert!(clock.ready(start, true));
        assert_eq!(clock.wake_at(start), None);
        clock.configure(super::presentation_interval(0, wgpu::PresentMode::AutoVsync, Some(120_000)), false);
        assert!(clock.ready(start, true), "uncapped input stays immediate with VSync");
        assert!(!clock.ready(start, false));
        assert_eq!(clock.wake_at(start), Some(start + Duration::from_secs_f64(1.0 / 120.0)));
        assert_eq!(super::presentation_interval(0, wgpu::PresentMode::AutoVsync, Some(0)), super::DEFAULT_FRAME);
    }

    #[test]
    fn uncapped_vsync_completion_never_adds_a_second_refresh_interval() {
        let start = Instant::now();
        let refresh = Duration::from_millis(16);
        let mut clock = FrameClock::default();
        clock.configure(refresh, false);
        assert!(clock.request(start));
        clock.presented(start + Duration::from_millis(5));
        assert_eq!(clock.wake_at(start), Some(start + refresh));
        // An unrelated settings update must not postpone that deadline.
        clock.configure(refresh, false);
        assert_eq!(clock.wake_at(start), Some(start + refresh));
        assert!(clock.request(start + refresh));
        // If preparation/presentation crosses the deadline, immediately
        // request the next redraw and let the compositor pace it.
        let completed = start + Duration::from_millis(35);
        clock.presented(completed);
        assert!(clock.ready(completed, false));
        assert_eq!(clock.wake_at(completed), Some(start + refresh * 2));
        // Input may still draw before an animation deadline.
        assert!(clock.request(completed));
        clock.presented(completed + Duration::from_millis(1));
        assert!(clock.ready(completed + Duration::from_millis(2), true));
        assert!(!clock.ready(completed + Duration::from_millis(2), false));
        // Enabling an explicit cap restores presentation-based spacing.
        clock.configure(refresh, true);
        let capped = completed + refresh;
        assert!(clock.request(capped + Duration::from_millis(1)));
        clock.presented(capped + Duration::from_millis(4));
        assert_eq!(clock.wake_at(capped), Some(capped + Duration::from_millis(4) + refresh));
    }

    #[test]
    fn cap_starts_when_the_frame_is_presented() {
        let start = Instant::now();
        let interval = Duration::from_millis(10);
        let mut clock = FrameClock::default();
        clock.configure(interval, true);
        assert!(clock.request(start));
        let presented = start + Duration::from_millis(3);
        clock.presented(presented);
        assert!(!clock.request(start + interval));
        assert_eq!(clock.wake_at(start + interval), Some(presented + interval));
        assert!(clock.request(presented + interval));
    }

    #[test]
    fn unsuccessful_draws_only_delay_the_animation_retry() {
        let start = Instant::now();
        let interval = Duration::from_millis(10);
        let mut clock = FrameClock::default();
        clock.configure(interval, true);
        clock.presented(start);
        let failed = start + interval;
        assert!(clock.request(failed));
        // No presentation followed this request. Avoid a timer spin while
        // still accepting a new update/redraw before the retry timer.
        assert_eq!(clock.wake_at(failed), Some(failed + interval));
        let update = failed + Duration::from_millis(1);
        assert!(!clock.ready(update, false));
        assert!(clock.ready(update, true));
        assert!(clock.request(update));
        assert_eq!(clock.last_presented, Some(start));
    }

    #[test]
    fn cap_changes_recompute_the_deadline_from_the_last_presentation() {
        let start = Instant::now();
        let mut clock = FrameClock::default();
        clock.configure(Duration::from_millis(20), true);
        clock.presented(start);
        let now = start + Duration::from_millis(5);
        clock.configure(Duration::from_millis(10), true);
        assert_eq!(clock.wake_at(now), Some(start + Duration::from_millis(10)));
        assert!(!clock.request(now));
        clock.configure(Duration::from_millis(4), true);
        assert!(clock.request(now));
        clock.configure(Duration::from_millis(30), true);
        assert_eq!(clock.wake_at(now), Some(start + Duration::from_millis(30)));
        assert!(!clock.request(now));
    }

    #[test]
    fn disabling_the_cap_restores_immediate_input_and_uncapped_polling() {
        let start = Instant::now();
        let mut clock = FrameClock::default();
        clock.configure(Duration::from_secs(1), true);
        clock.presented(start);
        let now = start + Duration::from_millis(1);
        clock.configure(Duration::from_millis(16), false);
        assert!(!clock.ready(now, false));
        assert!(clock.ready(now, true));
        assert!(clock.request(now));
        clock.configure(Duration::ZERO, false);
        assert_eq!(clock.wake_at(now), None);
        assert!(clock.ready(now, false));
        assert!(clock.request(now));
    }
}

/// The present mode for `asked` among the surface's `modes`, and how many
/// frames may be in flight: two with vsync permit CPU preparation to
/// overlap GPU rendering; mailbox/immediate keep their existing throughput
/// allowance of three. Acquiring a drawable can still block in the backend.
fn present_mode(asked: viewer_settings::Present, modes: &[wgpu::PresentMode]) -> (wgpu::PresentMode, u32) {
    use viewer_settings::Present;
    let has = |m| modes.contains(&m);
    match asked {
        Present::Immediate if has(wgpu::PresentMode::Immediate) => (wgpu::PresentMode::Immediate, 3),
        Present::Immediate | Present::Mailbox if has(wgpu::PresentMode::Mailbox) => (wgpu::PresentMode::Mailbox, 3),
        _ => (wgpu::PresentMode::AutoVsync, 2),
    }
}

/// An explicit cap paces all redraws independently of the surface mode.
/// Without one, only animations follow the display refresh.
fn presentation_interval(max_fps: u32, mode: wgpu::PresentMode, refresh_millihertz: Option<u32>) -> Duration {
    match (max_fps, mode) {
        (n, _) if n > 0 => Duration::from_nanos(1_000_000_000u64.div_ceil(u64::from(n))),
        (_, wgpu::PresentMode::Fifo | wgpu::PresentMode::AutoVsync) => refresh_millihertz.filter(|&r| r > 0)
            .map_or(DEFAULT_FRAME, |mhz| Duration::from_secs_f64(1000.0 / mhz as f64)),
        _ => Duration::ZERO,
    }
}

#[cfg(test)]
mod present_mode_tests {
    use super::present_mode;
    use viewer_settings::Present;
    use wgpu::PresentMode;

    #[test]
    fn vsync_allows_overlap_without_changing_explicit_present_modes() {
        let modes = [PresentMode::Fifo, PresentMode::Mailbox, PresentMode::Immediate];
        assert_eq!(present_mode(Present::Vsync, &modes), (PresentMode::AutoVsync, 2));
        assert_eq!(present_mode(Present::Mailbox, &modes), (PresentMode::Mailbox, 3));
        assert_eq!(present_mode(Present::Immediate, &modes), (PresentMode::Immediate, 3));
        assert_eq!(present_mode(Present::Immediate, &[PresentMode::Fifo, PresentMode::Mailbox]), (PresentMode::Mailbox, 3));
        assert_eq!(present_mode(Present::Vsync, &[PresentMode::Fifo, PresentMode::Immediate]), (PresentMode::AutoVsync, 2));
        assert_eq!(present_mode(Present::Immediate, &[PresentMode::Fifo]), (PresentMode::AutoVsync, 2));
        assert_eq!(present_mode(Present::Mailbox, &[PresentMode::Fifo]), (PresentMode::AutoVsync, 2));
    }
}

/// How long the event loop waits for the driver when it was woken for
/// nothing (see `about_to_wait`).
const IDLE_WAIT: Duration = Duration::from_millis(1);
/// Explicit atlas transfer allowance per frame. Large pages and copies
/// are split into strips. Driver initialization may also clear a whole
/// texture layer; that extra GPU work is not part of this byte allowance.
const ATLAS_PART: u64 = 4 << 20;

enum PendingArtPayload {
    Whole { version: u64, compiled: Arc<Compiled> },
    Growth { base: u64, version: u64, growth: Arc<render_desc::Growth> },
    Reserve { base: u64 },
}

impl PendingArtPayload {
    fn base(&self) -> u64 {
        match self {
            Self::Whole { version, .. } => *version,
            Self::Growth { base, .. } | Self::Reserve { base } => *base,
        }
    }

    fn source(&self) -> tile_render::renderer::AtlasTransferSource<'_> {
        use tile_render::renderer::AtlasTransferSource;
        match self {
            Self::Whole { compiled, .. } => AtlasTransferSource::Whole(&compiled.atlas),
            Self::Growth { growth, .. } => AtlasTransferSource::Growth(&growth.atlas),
            Self::Reserve { .. } => AtlasTransferSource::Reserve,
        }
    }
}

struct PendingArt {
    payload: PendingArtPayload,
    reserve: (u32, u32),
    started: bool,
    failed: bool,
    since: Instant,
    frames: u32,
}

impl PendingArt {
    fn new(payload: PendingArtPayload, reserve: (u32, u32)) -> Self {
        Self { payload, reserve, started: false, failed: false, since: Instant::now(), frames: 0 }
    }
}

struct SceneAdoption {
    changes: scene::Changes,
    patches: Vec<scene::Patch>,
}

fn uses_art(assets: u64, whole: u64, through: u64) -> bool {
    assets == 0 || (whole..=through).contains(&assets)
}

/// Moves only compatible received scenes into the live drawing snapshot.
/// Failed adoption leaves both pending changes and future patch pixels in
/// Display. An unchanged sequence reuses the snapshot's existing storage.
fn adopt_shown_scene(shown: &mut Scene, display: &mut Display, whole: u64, through: u64) -> Option<SceneAdoption> {
    let latest = display.scene();
    if latest.seq == 0 || latest.seq == shown.seq || !uses_art(latest.assets, whole, through) {
        return None;
    }
    let changes = display.take_changes_from(shown.seq).0;
    let patches = display.take_patches();
    // Drain patch pixels first. Keep the already owned world arrays and
    // copy only the records identified by Display's coalesced changes.
    let _ = scene_snapshot::update(shown, display.scene(), &changes);
    Some(SceneAdoption { changes, patches })
}

/// One future scene, separate from Display's coalesced latest scene. Its
/// patch pixels stay isolated from the drawable atlas until adoption.
struct PendingScene {
    base: u64,
    scene: Scene,
}

fn retain_pending_scene(pending: &mut Option<PendingScene>, shown: &Scene, display: &Display, base: u64, whole: u64, through: u64) {
    if pending.as_ref().is_some_and(|p| p.base != base) {
        *pending = None;
    }
    let latest = display.scene();
    if pending.is_none() && latest.seq != 0 && latest.seq != shown.seq && latest.assets >= base
        && !uses_art(latest.assets, whole, through) {
        // Do not drain Display here. Its later deltas and newest patch per
        // sprite must also survive cancellation by a replacement whole.
        *pending = Some(PendingScene { base, scene: latest.clone() });
    }
}

fn scene_art_target(pending: &Option<PendingScene>, display: &Display) -> u64 {
    pending.as_ref().map_or_else(|| display.scene().assets, |p| p.scene.assets)
}

fn adopt_available_scene(shown: &mut Scene, pending: &mut Option<PendingScene>, display: &mut Display, whole: u64, through: u64) -> Option<SceneAdoption> {
    // Prefer the newest scene whenever it is already compatible.
    if let Some(adoption) = adopt_shown_scene(shown, display, whole, through) {
        *pending = None;
        return Some(adoption);
    }
    if !pending.as_ref().is_some_and(|p| uses_art(p.scene.assets, whole, through)) {
        return None;
    }
    let mut next = pending.take().expect("compatible snapshot").scene;
    let patches = std::mem::take(&mut next.patches);
    *shown = next;
    // Display's accumulated changes span both this snapshot and anything
    // newer. Leave them there for the next adoption; rebuild this one.
    Some(SceneAdoption { changes: scene::Changes { world: true, sprites: true, views: true, ..Default::default() }, patches })
}

fn adopt_available_shared_scene(shown: &mut Arc<Scene>, pending: &mut Option<PendingScene>, display: &mut Display, whole: u64, through: u64) -> Option<SceneAdoption> {
    let latest = display.scene();
    let latest_ready = latest.seq != 0 && latest.seq != shown.seq && uses_art(latest.assets, whole, through);
    let pending_ready = pending.as_ref().is_some_and(|p| uses_art(p.scene.assets, whole, through));
    if !latest_ready && !pending_ready { return None; }
    // Only a real adoption can mutate the owned snapshot. Pinned readers
    // retain their immutable arrays through Arc's copy-on-write fallback.
    adopt_available_scene(Arc::make_mut(shown), pending, display, whole, through)
}

#[cfg(test)]
mod shown_scene_tests {
    use super::{Display, PendingScene, Scene, adopt_available_scene, adopt_shown_scene, looks, retain_pending_scene, scene_art_target};
    use scene::{FreeSprite, Input, Interactable, InteractableKind, LayerCells, LookState, Looks, PointerState, Update, View, ViewLooks, WorldShape};
    use std::time::Duration;

    fn connected() -> (scene::Driver, Display, Scene) {
        let (driver, mut display) = scene::connect();
        let hover = LookState { pointer: PointerState::Hover, selected: false };
        assert!(driver.send(vec![
            Update::Assets(7),
            Update::World {
                shape: WorldShape { cols: 2, rows: 1, ..Default::default() },
                layers: vec![LayerCells { values: vec![4, 5], ..Default::default() }],
            },
            Update::View { id: 5, view: Some(View::default()) },
            Update::Overlay { view: 5, sprites: vec![FreeSprite { set: 10, ..Default::default() }] },
            Update::Interactables(vec![Interactable { region: None, alt_right_click: false, anchor: None, id: 1, rect: [10, 10, 20, 20], kind: InteractableKind::Button }]),
            Update::Looks(vec![Looks {
                id: 1,
                views: vec![ViewLooks { view: 5, sprites: (0, 1), states: vec![(hover, vec![FreeSprite { set: 11, ..Default::default() }])] }],
                ..Default::default()
            }]),
        ]));
        assert!(display.receive(Duration::ZERO));
        let mut shown = Scene::default();
        assert!(adopt_shown_scene(&mut shown, &mut display, 7, 7).unwrap().changes.world);
        (driver, display, shown)
    }

    #[test]
    fn future_art_keeps_the_old_scene_interactive_and_preserves_queued_pixels() {
        let (driver, mut display, mut shown) = connected();
        let old_seq = shown.seq;
        let storage = shown.layers[0].values.as_ptr();
        let patch = scene::Patch { sprite: "minimap".into(), width: 1, height: 1, rgba: vec![20, 30, 40, 255] };
        assert!(driver.send(vec![
            Update::Assets(9),
            Update::World {
                shape: WorldShape { cols: 3, rows: 1, ..Default::default() },
                layers: vec![LayerCells { values: vec![8, 9, 10], ..Default::default() }],
            },
            Update::Overlay { view: 5, sprites: vec![FreeSprite { set: 999, ..Default::default() }] },
            Update::Patches(vec![patch.clone()]),
        ]));
        assert!(display.receive(Duration::ZERO));
        let pending = display.changes().clone();
        for through in [7, 8, 8] {
            assert!(adopt_shown_scene(&mut shown, &mut display, 7, through).is_none());
            assert_eq!(shown.seq, old_seq);
            assert_eq!(shown.layers[0].values.as_ptr(), storage);
            assert_eq!(display.changes(), &pending);
            assert_eq!(display.scene().patches, [patch.clone()]);
        }
        // Local feedback still changes the retained overlay while the
        // future world's sprite IDs and patch pixels remain untouched.
        let metadata = looks::LookScene::of(&shown);
        let mut feedback = looks::Feedback::default();
        feedback.sent(&Input::MouseMove { x: 15, y: 15 }, &metadata, 0.0);
        let spans = [looks::Span { view: 5, start: 0, len: 1, cameras: vec![2] }];
        let hovered = looks::splice(&shown.views[0].overlay, &spans, &feedback.replacements(&metadata, 0.01), |sprite, _| *sprite);
        assert_eq!(hovered[0].set, 11);
        feedback.sent(&Input::MouseMove { x: 40, y: 40 }, &metadata, 0.02);
        let normal = looks::splice(&shown.views[0].overlay, &spans, &feedback.replacements(&metadata, 0.03), |sprite, _| *sprite);
        assert_eq!(normal[0].set, 10);
        assert_eq!(shown.layers[0].values, [4, 5]);

        let adopted = adopt_shown_scene(&mut shown, &mut display, 9, 9).unwrap();
        assert_eq!(adopted.changes, pending);
        assert_eq!(adopted.patches, [patch]);
        assert_eq!(shown.assets, 9);
        assert_eq!(shown.layers[0].values, [8, 9, 10]);
        assert_eq!(shown.views[0].overlay[0].set, 999);
        assert!(shown.patches.is_empty() && display.scene().patches.is_empty());
        assert!(display.changes().is_empty());
    }

    #[test]
    fn incremental_changes_accumulate_until_the_needed_growth_is_active() {
        let (driver, mut display, mut shown) = connected();
        assert!(driver.send(vec![Update::Assets(9), Update::Cells(vec![(0, 1, 20)])]));
        assert!(display.receive(Duration::ZERO));
        assert!(adopt_shown_scene(&mut shown, &mut display, 7, 8).is_none());
        assert!(driver.send(vec![Update::Cells(vec![(0, 0, 30)])]));
        assert!(display.receive(Duration::ZERO));
        let adopted = adopt_shown_scene(&mut shown, &mut display, 7, 9).unwrap();
        assert!(!adopted.changes.world);
        assert_eq!(adopted.changes.cells.len(), 2);
        assert!(adopted.changes.cells.contains(&(0, 0)) && adopted.changes.cells.contains(&(0, 1)));
        assert_eq!(shown.layers[0].values, [30, 20]);
        assert!(display.changes().is_empty());
    }

    fn hold(pending: &mut Option<PendingScene>, shown: &Scene, display: &Display, base: u64, through: u64) {
        retain_pending_scene(pending, shown, display, base, base, through);
    }

    #[test]
    fn future_snapshot_advances_while_optional_generations_keep_arriving() {
        let (driver, mut display, mut shown) = connected();
        let mut pending = None;
        for generation in 8..=40 {
            assert!(driver.send(vec![Update::Assets(generation), Update::Cells(vec![(0, 0, generation as u32)])]));
            assert!(display.receive(Duration::ZERO));
            hold(&mut pending, &shown, &display, 7, 7);
            // Exactly one future snapshot remains fixed while Display
            // coalesces all newer states into its own latest scene.
            assert_eq!(pending.as_ref().unwrap().scene.assets, 8);
            assert_eq!(scene_art_target(&pending, &display), 8);
            assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 7).is_none());
            assert_eq!(shown.assets, 7);
        }
        // The former latest-only adoption is still blocked here even
        // though the first complete snapshot's art is available.
        assert!(adopt_shown_scene(&mut shown, &mut display, 7, 8).is_none());
        let adoption = adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 8).unwrap();
        assert!(adoption.changes.world);
        assert_eq!(shown.assets, 8);
        assert_eq!(shown.layers[0].values[0], 8);
        assert_eq!(display.scene().assets, 40);
        assert_eq!(display.scene().layers[0].values[0], 40);
        assert!(pending.is_none());
        hold(&mut pending, &shown, &display, 7, 8);
        assert_eq!(scene_art_target(&pending, &display), 40);
        // The next fixed target can also become drawable without waiting
        // for the producer's newest optional generation.
        assert!(driver.send(vec![Update::Assets(41), Update::Cells(vec![(0, 0, 41)])]));
        assert!(display.receive(Duration::ZERO));
        hold(&mut pending, &shown, &display, 7, 39);
        assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 39).is_none());
        assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 40).is_some());
        assert_eq!(shown.assets, 40);
        assert_eq!(shown.layers[0].values[0], 40);
        assert_eq!(display.scene().assets, 41);
    }

    #[test]
    fn retained_future_patches_are_isolated_and_latest_deltas_survive() {
        let (driver, mut display, mut shown) = connected();
        let mut pending = None;
        let old = scene::Patch { sprite: "minimap".into(), width: 1, height: 1, rgba: vec![1, 2, 3, 255] };
        let new = scene::Patch { rgba: vec![4, 5, 6, 255], ..old.clone() };
        assert!(driver.send(vec![Update::Assets(9), Update::Cells(vec![(0, 0, 9)]), Update::Patches(vec![old.clone()])]));
        assert!(display.receive(Duration::ZERO));
        hold(&mut pending, &shown, &display, 7, 7);
        assert!(driver.send(vec![Update::Assets(10), Update::Cells(vec![(0, 1, 10)]), Update::Patches(vec![new.clone()])]));
        assert!(display.receive(Duration::ZERO));
        hold(&mut pending, &shown, &display, 7, 8);
        assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 8).is_none());
        assert!(shown.patches.is_empty());
        let adoption = adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 9).unwrap();
        assert_eq!(adoption.patches, [old]);
        assert!(shown.patches.is_empty());
        assert_eq!(shown.layers[0].values, [9, 5]);
        assert_eq!(display.scene().patches, [new.clone()]);
        assert_eq!(display.changes().cells.len(), 2);
        let adoption = adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 10).unwrap();
        assert_eq!(adoption.patches, [new]);
        assert_eq!(shown.layers[0].values, [9, 10]);
        assert!(display.changes().is_empty());
        assert!(display.scene().patches.is_empty());
    }

    #[test]
    fn compatible_latest_scene_replaces_the_future_snapshot() {
        let (driver, mut display, mut shown) = connected();
        let mut pending = None;
        assert!(driver.send(vec![Update::Assets(9), Update::Cells(vec![(0, 0, 9)])]));
        assert!(display.receive(Duration::ZERO));
        hold(&mut pending, &shown, &display, 7, 7);
        assert!(pending.is_some());
        // A newer scene can depend on a smaller prefix, for example after
        // closing a screen whose new art has not arrived yet.
        assert!(driver.send(vec![Update::Assets(8), Update::Cells(vec![(0, 0, 8)])]));
        assert!(display.receive(Duration::ZERO));
        assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 8).is_some());
        assert_eq!(shown.assets, 8);
        assert_eq!(shown.layers[0].values[0], 8);
        assert!(pending.is_none());
    }

    #[test]
    fn superseding_whole_discards_only_its_snapshot_without_losing_latest_patches() {
        let (driver, mut display, mut shown) = connected();
        let mut pending = None;
        let patch = scene::Patch { sprite: "minimap".into(), width: 1, height: 1, rgba: vec![7, 8, 9, 255] };
        assert!(driver.send(vec![Update::Assets(9), Update::Patches(vec![patch.clone()])]));
        assert!(display.receive(Duration::ZERO));
        hold(&mut pending, &shown, &display, 7, 7);
        assert!(pending.is_some());
        retain_pending_scene(&mut pending, &shown, &display, 20, 7, 7);
        assert!(pending.is_none());
        assert_eq!(display.scene().patches, [patch.clone()]);
        assert!(driver.send(vec![Update::Assets(20), Update::Cells(vec![(0, 0, 20)])]));
        assert!(display.receive(Duration::ZERO));
        retain_pending_scene(&mut pending, &shown, &display, 20, 7, 7);
        assert_eq!(scene_art_target(&pending, &display), 20);
        assert!(adopt_available_scene(&mut shown, &mut pending, &mut display, 7, 19).is_none());
        let adoption = adopt_available_scene(&mut shown, &mut pending, &mut display, 20, 20).unwrap();
        assert_eq!(adoption.patches, [patch]);
        assert_eq!(shown.assets, 20);
        assert!(pending.is_none());
    }

    #[test]
    fn unchanged_sequence_reuses_snapshot_storage() {
        let (_, mut display, mut shown) = connected();
        let cells = shown.layers[0].values.as_ptr();
        let views = shown.views.as_ptr();
        for _ in 0..10 {
            assert!(adopt_shown_scene(&mut shown, &mut display, 7, 7).is_none());
            assert_eq!(shown.layers[0].values.as_ptr(), cells);
            assert_eq!(shown.views.as_ptr(), views);
        }
    }
}

/// A replacement is prepared independently until its complete scene can
/// draw. Raw source pages and growths are retained for a bounded restart
/// when old + intermediate + resized arrays cannot all fit the peak limit.
struct PreparingArt {
    renderer: TileRenderer,
    base: u64,
    whole: Arc<Compiled>,
    compiled: Compiled,
    through: u64,
    whole_done: bool,
    growths: Vec<(u64, Arc<render_desc::Growth>)>,
    applied: usize,
    minimum_room: (u32, u32),
    reserve: (u32, u32),
    pending: Option<PendingArt>,
    failed: bool,
    obsolete: bool,
    since: Instant,
    frames: u32,
}

impl PreparingArt {
    fn new(active: &TileRenderer, base: u64, whole: Arc<Compiled>) -> Self {
        let compiled = whole.without_pixels();
        let minimum_room = (
            whole.atlas.pages.iter().filter(|p| p.width == whole.atlas.page_size).count() as u32,
            whole.atlas.index_pages.len() as u32,
        );
        Self {
            renderer: active.atlas_preparer(), base, whole, compiled, through: base,
            whole_done: false, growths: Vec::new(), applied: 0, minimum_room,
            reserve: (0, 0), pending: None, failed: false, obsolete: false,
            since: Instant::now(), frames: 0,
        }
    }

    fn cancel(&mut self, queue: &wgpu::Queue) {
        self.renderer.cancel_atlas_transfer(queue);
        self.pending = None;
        self.obsolete = true;
    }

    /// The preparer has never rendered. Once its transfer fences are done,
    /// its intermediate atlas can be released without a wait or another
    /// retained generation. Replay source references, without cloning pages.
    fn restart(&mut self, active: &TileRenderer) {
        debug_assert!(self.renderer.atlas_transfer_idle());
        self.renderer = active.atlas_preparer();
        self.compiled = self.whole.without_pixels();
        self.whole_done = false;
        self.through = self.base;
        self.applied = 0;
        self.pending = None;
    }

    fn advance(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, active_renderer: &TileRenderer, assets: &mut ArtInbox, scene_assets: u64) -> Result<(u64, bool), tile_render::renderer::AtlasTransferError> {
        use tile_render::renderer::AtlasTransferError;
        self.renderer.poll_atlas_transfers(device);
        if self.failed || self.obsolete { return Ok((0, false)) }
        // Collect only the prefix required by the scene, never optional
        // preload work beyond it. Sources stay ordered across restarts.
        let mut known = self.growths.last().map_or(self.base, |(version, _)| *version);
        while known < scene_assets {
            let Some((version, growth)) = assets.take_growth(self.base) else { break };
            self.minimum_room.0 = self.minimum_room.0.max(growth.atlas.pages);
            self.minimum_room.1 = self.minimum_room.1.max(growth.atlas.index_pages);
            known = version;
            self.growths.push((version, growth));
        }
        if let Some((pages, index)) = assets.take_reserve(self.base) {
            self.reserve.0 = self.reserve.0.max(pages);
            self.reserve.1 = self.reserve.1.max(index);
        }
        let limit = active_renderer.atlas_transfer_limit().get();
        let active = active_renderer.atlas_resident_bytes();
        let available = limit.saturating_sub(active);
        let remaining = std::num::NonZeroU64::new(available).ok_or(AtlasTransferError::BudgetExceeded { required: active.saturating_add(1), limit })?;
        self.renderer.set_atlas_transfer_limit(remaining);
        if self.pending.is_none() {
            let payload = if !self.whole_done {
                Some(PendingArtPayload::Whole { version: self.base, compiled: self.whole.clone() })
            } else {
                self.growths.get(self.applied).map(|(version, growth)| PendingArtPayload::Growth {
                    base: self.base, version: *version, growth: growth.clone(),
                })
            };
            self.pending = payload.map(|payload| PendingArt::new(payload, self.reserve));
        }
        let mut bytes = 0;
        if let Some(mut p) = self.pending.take() {
            if !p.started {
                let minimum = if matches!(p.payload, PendingArtPayload::Whole { .. }) { self.minimum_room } else { (0, 0) };
                match self.renderer.begin_atlas_transfer_with_capacity(device, p.payload.source(), minimum, p.reserve) {
                    Ok(()) => {
                        p.started = true;
                        self.reserve = (0, 0);
                    }
                    Err(AtlasTransferError::Busy) => {
                        self.pending = Some(p);
                        return Ok((0, false));
                    }
                    Err(AtlasTransferError::BudgetExceeded { .. }) if matches!(&p.payload, PendingArtPayload::Growth { growth, .. }
                        if growth.atlas.pages > self.renderer.atlas_room().0 || growth.atlas.index_pages > self.renderer.atlas_room().1) => {
                        // old drawable + intermediate + resized array did
                        // not fit. Rebuild directly into the final required
                        // capacity, keeping the drawable atlas throughout.
                        self.restart(active_renderer);
                        return Ok((0, false));
                    }
                    Err(e) => return Err(e),
                }
            }
            p.frames += 1;
            self.frames += 1;
            let progress = match self.renderer.advance_atlas_transfer(device, queue, p.payload.source(), ATLAS_PART) {
                Ok(progress) => progress,
                Err(AtlasTransferError::Invalidated) => {
                    self.renderer.cancel_atlas_transfer(queue);
                    p.started = false;
                    self.pending = Some(p);
                    return Ok((0, false));
                }
                Err(e) => return Err(e),
            };
            bytes = progress.bytes;
            if progress.ready && self.renderer.commit_atlas_transfer(device, queue) {
                match p.payload {
                    PendingArtPayload::Whole { .. } => self.whole_done = true,
                    PendingArtPayload::Growth { version, growth, .. } => {
                        self.compiled.apply(&growth);
                        self.applied += 1;
                        self.through = version;
                    }
                    PendingArtPayload::Reserve { .. } => unreachable!("a replacement does not need a separate reservation"),
                }
            } else {
                self.pending = Some(p);
            }
        }
        self.renderer.poll_atlas_transfers(device);
        let ready = self.whole_done && self.pending.is_none() && self.renderer.atlas_transfer_idle()
            && uses_art(scene_assets, self.base, self.through);
        Ok((bytes, ready))
    }
}

#[cfg(test)]
mod preparation_gpu_tests {
    use super::*;
    use tile_render::{AtlasGrowth, Blit, BlitPixels, Headless, atlas::sprite_at};

    struct Images;
    impl render_desc::ImageLoader for Images {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            Ok(vec![RgbaImage::from_fn(8, 8, |_, _| [240, 40, 8, 255])])
        }
    }

    fn compiled() -> Arc<Compiled> {
        Arc::new(render_desc::Description::parse(r#"
            grid = { layout = "orthogonal", step = [8, 8] }
            [images.base]
            source = "base"
            [sheets.s]
            image = "base"
            cell = [8, 8]
        "#).unwrap().compile(&Images, 16).unwrap())
    }

    fn growth(first: u32, x: u32) -> Arc<render_desc::Growth> {
        let mut growth = render_desc::Growth::default();
        growth.atlas = AtlasGrowth {
            first, pages: 2, index_pages: 0,
            sprites: vec![sprite_at((1, x, 0), (4, 4), (0, 0))],
            blits: vec![Blit { page: 1, x, y: 0, pixels: BlitPixels::Rgba(RgbaImage::from_fn(4, 4, |_, _| [8, 40, 240, 255])) }],
            ..Default::default()
        };
        growth.sprite_names = vec![format!("added-{first}")];
        Arc::new(growth)
    }

    fn drive_base(gpu: &Headless, active: &TileRenderer, assets: &mut ArtInbox, preparing: &mut PreparingArt, target: u64) {
        for _ in 0..64 {
            let (_, ready) = preparing.advance(&gpu.device, &gpu.queue, active, assets, target).unwrap();
            assert!(!ready, "the required growth has not been published");
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            preparing.renderer.poll_atlas_transfers(&gpu.device);
            if preparing.whole_done && preparing.renderer.atlas_transfer_idle() { return }
        }
        panic!("base did not finish");
    }

    #[test]
    #[ignore = "requires a GPU; exercised with the atlas transfer regressions"]
    fn delayed_required_growth_does_not_activate_an_intermediate_whole() {
        let gpu = Headless::new().unwrap();
        let whole = compiled();
        let mut active = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        active.set_atlas(&gpu.device, &gpu.queue, &whole.atlas);
        let old_room = active.atlas_room();
        let mut assets = ArtInbox::new(Assets::new(whole.clone()));
        assets.set(whole.clone(), 7);
        let mut preparing = PreparingArt::new(&active, 7, whole);
        drive_base(&gpu, &active, &mut assets, &mut preparing, 9);
        for _ in 0..4 {
            assert!(!preparing.advance(&gpu.device, &gpu.queue, &active, &mut assets, 9).unwrap().1);
            assert_eq!(active.atlas_room(), old_room);
            assert_eq!(preparing.through, 7);
        }
        assets.grow(growth(1, 0), 8);
        assets.grow(growth(2, 4), 9);
        let mut ready = false;
        for _ in 0..64 {
            ready = preparing.advance(&gpu.device, &gpu.queue, &active, &mut assets, 9).unwrap().1;
            assert_eq!(active.atlas_room(), old_room);
            assert!(active.atlas_resident_bytes() + preparing.renderer.atlas_resident_bytes() <= active.atlas_transfer_limit().get());
            if ready { break }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
        assert!(ready);
        assert_eq!(preparing.through, 9);
        assert_eq!(preparing.compiled.atlas.sprites.len(), 3);
        active.adopt_prepared_atlas(&gpu.device, &gpu.queue, &mut preparing.renderer).unwrap();
        assert!(active.atlas_room().0 >= 2);
    }

    #[test]
    #[ignore = "requires a GPU; exercised with the atlas transfer regressions"]
    fn tight_peak_limit_replays_directly_into_mandatory_final_capacity() {
        let gpu = Headless::new().unwrap();
        let whole = compiled();
        let mut active = TileRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
        active.set_atlas(&gpu.device, &gpu.queue, &whole.atlas);
        let old_resident = active.atlas_resident_bytes();
        let desc = std::mem::size_of_val(&whole.atlas.sprites[0]) as u64;
        let base_tables = desc + 256 * 4;
        let final_tables = 2 * desc + 256 * 4;
        let final_arrays = 16 * 16 * 2 * 4 + 1;
        let remainder = final_arrays + base_tables + final_tables + tile_render::renderer::ATLAS_TRANSFER_STAGING_RESERVE_BYTES;
        active.set_atlas_transfer_limit(std::num::NonZeroU64::new(old_resident + remainder).unwrap());
        let mut assets = ArtInbox::new(Assets::new(whole.clone()));
        assets.set(whole.clone(), 7);
        let mut preparing = PreparingArt::new(&active, 7, whole);
        drive_base(&gpu, &active, &mut assets, &mut preparing, 8);
        assets.grow(growth(1, 0), 8);
        assert!(!preparing.advance(&gpu.device, &gpu.queue, &active, &mut assets, 8).unwrap().1);
        assert!(!preparing.whole_done, "a resize that exceeds the combined peak must restart preparation");
        assert_eq!(preparing.renderer.atlas_resident_bytes(), 0);
        assert_eq!(preparing.growths.len(), 1, "keep the source for replay");
        let mut ready = false;
        for _ in 0..64 {
            ready = preparing.advance(&gpu.device, &gpu.queue, &active, &mut assets, 8).unwrap().1;
            assert_eq!(active.atlas_resident_bytes(), old_resident);
            assert!(old_resident + preparing.renderer.atlas_resident_bytes() <= active.atlas_transfer_limit().get());
            if ready { break }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
        assert!(ready);
        assert_eq!(preparing.through, 8);
        assert_eq!(preparing.renderer.atlas_room(), (2, 0));
        assert_eq!(preparing.compiled.atlas.sprites.len(), 2);
        active.adopt_prepared_atlas(&gpu.device, &gpu.queue, &mut preparing.renderer).unwrap();
        assert!(active.atlas_resident_bytes() <= active.atlas_transfer_limit().get());
    }
}

/// The last composed picture keeps its own canvas and magnifier sources
/// when the window resizes. No future scene IDs are needed to present it.
struct CachedPicture {
    plan: scaling::Plan,
    /// As drawn (`scaling::Drawn`): the canvas composed `pixel_box` times
    /// larger each way (1: as the plan's), the copies in its pixels.
    copies: Vec<tile_render::ScreenCopy>,
    pixel_box: u32,
}

impl CachedPicture {
    /// How the kept picture is shown on the window (see `scaling::drawn`).
    fn present(&self, plan: &scaling::Plan) -> tile_render::Present {
        if self.pixel_box > 1 { plan.present.at_target() } else { plan.present }
    }

    fn plan_for_target(&self, target: (u32, u32)) -> scaling::Plan {
        let mut plan = self.plan;
        let shift = |at: i32, old: u32, new: u32| {
            (i64::from(at) + (i64::from(new) - i64::from(old)).div_euclid(2))
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        plan.present.at = (
            shift(plan.present.at.0, plan.present.target.0, target.0),
            shift(plan.present.at.1, plan.present.target.1, target.1),
        );
        plan.present.target = target;
        plan
    }
}

#[cfg(test)]
mod cached_picture_tests {
    use super::{CachedPicture, scaling};
    use tile_render::{Present, ScreenCopy};

    fn picture() -> CachedPicture {
        let mut plan = scaling::plan((1280, 960), (640, 480), &Default::default());
        plan.present = Present { canvas: (640, 480), zoom: (2, 1), at: (0, 0), target: (1280, 960) };
        CachedPicture {
            plan,
            copies: vec![ScreenCopy { source: [100, 80, 120, 100], at: (300, 180), zoom: (2, 1), lens: None }],
            pixel_box: 1,
        }
    }

    #[test]
    fn a_picture_composed_at_the_window_resolution_is_replayed_one_to_one() {
        let mut picture = picture();
        assert_eq!(picture.present(&picture.plan), picture.plan.present);
        picture.pixel_box = 2;
        let plan = picture.plan_for_target((1300, 1000));
        let present = picture.present(&plan);
        assert_eq!((present.canvas, present.zoom, present.at, present.target), ((1280, 960), (1, 1), plan.present.at, (1300, 1000)));
    }

    #[test]
    fn resized_cached_picture_keeps_input_on_the_same_canvas_pixel() {
        let picture = picture();
        let before = picture.plan.to_screen((640.0, 480.0));
        for target in [(1600, 1000), (320, 200), (1280, 960)] {
            let plan = picture.plan_for_target(target);
            assert_eq!(plan.to_screen((target.0 as f64 / 2.0, target.1 as f64 / 2.0)), before);
            assert_eq!(plan.canvas(), picture.plan.canvas());
            assert_eq!(plan.screen, picture.plan.screen);
            assert_eq!(plan.present.zoom, picture.plan.present.zoom);
            assert_eq!(plan.present.target, target);
        }
    }

    #[test]
    fn repeated_resize_uses_the_original_picture_without_mapping_drift() {
        let picture = picture();
        let original = picture.plan;
        let small = picture.plan_for_target((319, 201));
        assert!(small.present.at.0 < 0 && small.present.at.1 < 0);
        assert_eq!(picture.plan_for_target(original.present.target), original);
        // Copies still address the retained canvas, including after the
        // target becomes smaller than a magnifier's destination.
        assert_eq!(picture.copies[0].scissor(small.canvas()), Some([300, 180, 240, 200]));
        assert_eq!(picture.plan, original);
    }
}

/// Whether frame times are logged (`TILE_VIEWER_FRAME_TIMES`, see `fps::FrameLog`).
fn frame_times() -> bool {
    std::env::var("TILE_VIEWER_FRAME_TIMES").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// The renderer's sampling for the user's menu background setting.
pub fn background_sampling(setting: viewer_settings::BackgroundSampling) -> tile_render::BackgroundSampling {
    match setting {
        viewer_settings::BackgroundSampling::Dither => tile_render::BackgroundSampling::Dither,
        viewer_settings::BackgroundSampling::Interpolate => tile_render::BackgroundSampling::Interpolate,
        viewer_settings::BackgroundSampling::Nearest => tile_render::BackgroundSampling::Nearest,
    }
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
        self.input_revision = self.input_revision.wrapping_add(1);
        self.layout.apply_settings(&settings.layout);
        self.layout.clean_map = settings.clean_map.clone();
        self.layout.focus_arrow = settings.accessibility.active_unit_arrow;
        self.layout.set_map_movement(settings.input.map_movement);
        self.layout.set_capture_sensitivity(settings.input.capture_sensitivity);
        if let Ok(spec) = std::env::var("TILE_VIEWER_LAYOUT") {
            if let Err(e) = self.layout.configure(&spec) {
                log::warn!("TILE_VIEWER_LAYOUT: {e}");
            }
        }
        // The counter counts while it shows, afresh each time it is shown.
        let old_details = self.settings.display.show_fps && self.settings.display.fps_detailed;
        let new_details = settings.display.show_fps && settings.display.fps_detailed;
        if settings.display.show_fps != self.settings.display.show_fps || old_details != new_details {
            self.fps = fps::Overlay::default();
        }
        if old_details != new_details {
            self.fps.timings.reset_from(self.next_frame_id);
        }
        self.settings = settings;
        self.configure_present();
        self.configure_timing();
    }

    fn configure_timing(&mut self) {
        let requested = tile_render::timing::requested() || self.profile.is_some()
            || (self.settings.display.show_fps && self.settings.display.fps_detailed);
        let Some(gpu) = &mut self.gpu else { return };
        gpu.worker.timing(requested);
    }

    /// The present mode and frame rate the settings ask for
    /// (`TILE_VIEWER_PRESENT`, `TILE_VIEWER_MAX_FPS` override them): the
    /// surface configured anew if its mode changes.
    fn configure_present(&mut self) {
        let asked = std::env::var("TILE_VIEWER_PRESENT").ok().and_then(|v| viewer_settings::Present::parse(&v)).unwrap_or(self.settings.display.present);
        let max_fps = std::env::var("TILE_VIEWER_MAX_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(self.settings.display.max_fps);
        let Some(gpu) = &mut self.gpu else { return };
        let (mode, latency) = present_mode(asked, &gpu.present_modes);
        // An explicit maximum limits every frame, including input and
        // scene updates. Otherwise the display's refresh only paces the
        // animation clock; input can still request a frame immediately.
        let refresh = gpu.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).filter(|&r| r > 0);
        let interval = presentation_interval(max_fps, mode, refresh);
        self.frame_clock.configure(interval, max_fps > 0);
        if (gpu.config.present_mode, gpu.config.desired_maximum_frame_latency) != (mode, latency) {
            gpu.config.present_mode = mode;
            gpu.config.desired_maximum_frame_latency = latency;
            gpu.config_revision = gpu.config_revision.wrapping_add(1);
            gpu.worker.configure(render_worker::Config { revision: gpu.config_revision, width: gpu.config.width, height: gpu.config.height, present_mode: mode, latency });
            gpu.reconfigured = true;
        }
        log::info!(
            "viewer: present {mode:?} (asked {asked:?}; the surface has {:?}), monitor refresh {refresh:?} mHz, explicit cap {max_fps} FPS, {latency} frame(s) in flight, frames {}",
            gpu.present_modes,
            if interval.is_zero() { "as fast as they are drawn".to_owned() }
            else if max_fps > 0 { format!("at most {max_fps} per second") }
            else { format!("animation every {:.2} ms, input immediately", interval.as_secs_f64() * 1000.0) }
        );
    }

    /// Input for the driver: also what the interaction looks follow (a
    /// frame is drawn at once when they may change).
    fn resolve_settings_preview(&mut self, result: Option<(Input, u32)>) {
        if let Some((input, scale)) = result {
            self.input.reset_wheel();
            let mut settings = self.settings.clone();
            settings.resolution.pixel_scale = scale;
            self.set_settings(settings);
            // Confirmation and rollback never wait for a render submission.
            self.display.send_input(input);
            if let Some(gpu) = &self.gpu { gpu.window.request_redraw(); }
        }
    }

    fn send(&mut self, input: Input) {
        let now = self.now();
        let input = if self.snap.snap_mode_active() { input } else {
            let Some(input) = self.menu_scroll.route(input, now) else {
                self.feedback.pointer_gone(now);
                return;
            };
            input
        };
        if self.feedback.sent(&input, &self.look_scene, now) {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
        if matches!(input, Input::MouseButton { .. }) {
            log::debug!("viewer: forwarding {input:?}");
        }
        frame_input(&mut self.preparing_inputs, &self.display, input);
    }

    /// The slider arrow keys step is drawn hovered (a frame at once when
    /// that changes).
    fn show_keyboard_slider(&mut self) {
        if self.settings_preview.active() { return; }
        let now = self.now();
        if self.feedback.keyboard_slider(self.snap.selected_slider(), now) {
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
    }

    /// Retarget only against pixels the user can see. A failed surface
    /// acquisition or a software frame still in flight must not report a
    /// future camera position to the game.
    fn refresh_presented_world_pointer(&mut self) {
        let active = self.gpu.as_ref().is_some_and(|gpu| gpu.window.has_focus())
            && (self.pointer_inside || self.layout.captured()) && !self.snap.snap_mode_active();
        for input in self.layout.refresh_world_pointer(active) {
            frame_input(&mut self.preparing_inputs, &self.display, input);
        }
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

    /// The cursor's size in effect (the setting, by default the system's),
    /// at the pixel scale: the system pointer and the drawn cursor alike.
    fn cursor_scale(&self) -> cursor::Scale {
        let present = self.plan.map_or(1, |p| p.present.zoom.0.max(1) / p.present.zoom.1.max(1)).max(1);
        cursor::Scale::new(self.settings.display.effective_cursor_size(self.system_cursor_size), present)
    }

    fn shown_scene_matches_art(&self) -> bool {
        self.shown_scene.seq != 0 && (uses_art(self.shown_scene.assets, self.whole_version, self.assets_version)
            || self.future_metadata.as_ref().is_some_and(|m| m.base == self.whole_version && m.through == self.shown_scene.assets))
    }

    fn activate_cursor_art(&mut self) {
        if !self.shown_scene_matches_art() {
            return;
        }
        if self.pending_cursor_art.as_ref().is_some_and(|(base, _)| *base == self.whole_version) {
            let (_, art) = self.pending_cursor_art.take().expect("checked above");
            self.cursor_art = art;
        }
    }

    /// Receive OS cursor metadata independently of GPU upload/presentation.
    fn refresh_system_cursor(&mut self) {
        let through = self.assets.latest();
        let Some(whole) = self.assets.0.whole.load() else { return };
        if self.os_cursor.refresh(self.display.scene(), whole.generation, through, whole.cursor_art.clone()) {
            self.custom_cursors.clear();
            self.cursor_builds.forget();
            self.pointer_presentation.shown = None;
        }
    }

    /// Shows the system pointer as it is to be now (`cursor`): hidden
    /// while the viewer draws the cursor, else the driver's cursor (made
    /// from its art), or the capture pointer.
    fn update_system_pointer(&mut self, event_loop: &ActiveEventLoop) {
        self.refresh_system_cursor();
        // (Button snapping says when it hides the pointer; whether it is
        // hidden is asked anew every time.)
        let _ = self.snap.take_hide_pointer();
        // System cursors made since (each asked for again).
        for (key, source) in self.cursor_builds.take_ready() {
            self.custom_cursors.insert(key, event_loop.create_custom_cursor(source));
            self.pointer_presentation.shown = None;
        }
        let status = host_status::judge(self.display.host(), Instant::now());
        let hidden = self.pointer_presentation.active() && self.pointer_hidden();
        let shown = self.os_cursor.shown_busy(hidden, self.capture_icon, self.display.scene().interface.targeting, self.layout.captured(), status, std::time::Duration::from_secs_f64(self.now().max(0.0)));
        let scale = self.cursor_scale();
        if self.gpu.is_none() { return; }
        let Some(restore) = self.pointer_presentation.apply(shown, scale) else { return };
        if restore {
            // Repeat the real grab request, including release. AppKit and
            // compositors may have revoked it while the window was away.
            self.grab((self.layout.captured() && !self.machine.edge()) || self.snap.snap_mode_active());
        }
        let Some(gpu) = &self.gpu else { return };
        if restore {
            // A same-value call also hits winit's macOS visibility cache.
            // Reproduce Alt's visibility transition before the final state.
            gpu.window.set_cursor_visible(shown == cursor::Shown::Hidden);
        }
        log::debug!("viewer: system pointer {shown:?}");
        match shown {
            cursor::Shown::Hidden => gpu.window.set_cursor_visible(false),
            cursor::Shown::Icon(icon) => {
                gpu.window.set_cursor(icon);
                gpu.window.set_cursor_visible(true);
            }
            cursor::Shown::Custom(_) | cursor::Shown::Badged { .. } => {
                // At the cursor size, pixel for pixel what the viewer draws
                // of it on the canvas at the pixel scale (a custom cursor's
                // pixels are the window's physical ones on every system).
                match self.custom_cursors.get(&(shown, scale)) {
                    Some(c) => gpu.window.set_cursor(c.clone()),
                    None if self.os_cursor.art().has_image(shown) => {
                        // Made on a thread of its own; until it is, the
                        // pointer stays as it is (asked again next frame).
                        // An animation's pictures are all made at once and
                        // kept: each later step only switches handles.
                        for picture in self.os_cursor.art().animation_of(shown) {
                            if !self.custom_cursors.contains_key(&(picture, scale)) {
                                self.cursor_builds.start((picture, scale), self.os_cursor.art_handle());
                            }
                        }
                        self.pointer_presentation.shown = None;
                    }
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
        for input in self.layout.take_unit_order_events() {
            self.send(input);
        }
        let grab = self.layout.take_grab();
        if grab.is_some() {
            self.machine.cancel_edge();
            self.layout.set_pointer_at_edge(false);
            self.unlock_pending = None;
        }
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
        if let (Some(p), Some(_)) = (warp, &self.gpu) {
            let (x, y) = self.plan.map_or(p, |plan| plan.to_window(p));
            match self.constraints.move_pointer((x, y)) {
                Ok(constraints::Move::Pending) => {
                    log::debug!("viewer: moving the pointer to {x}, {y} (lock and hint)");
                    // Input follows it after the compositor activates the lock.
                }
                Ok(constraints::Move::Immediate) => self.pointer_placed((x, y)),
                Err(e) => {
                    log::debug!("viewer: moving the pointer to {x}, {y}: {e}");
                    self.layout.pointer_cannot_be_moved();
                    self.pointer_placement_failed();
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
        self.layout.set_pointer_at_edge(self.machine.edge());
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
    /// active view only (the edge mode); the pointer shows then.
    fn unlock_after_frame(&mut self) {
        if !std::mem::take(&mut self.presented) {
            return;
        }
        match self.unlock_pending.take() {
            Some(Unlock::Release) => self.grab(false),
            Some(Unlock::Edge) => {
                // winit's region covers the whole window and prevents our
                // Wayland walls from taking ownership of the pointer.
                self.grab(false);
                // Confine the pointer to its line (or area) at once, in the
                // same request batch as the release: the pointer is at the
                // drawn cursor now, and any motion before a later frame
                // would take it off a one-pixel line before the system
                // holds it there.
                self.layout.set_pointer_at_edge(self.machine.edge());
                if let Some(gpu) = &self.gpu {
                    match self.constraints.update(&gpu.window, &mut self.layout, self.plan.map(|p| p.present), self.pointer_inside) {
                        Some(constraints::Placement::Hinted(at)) => self.pointer_placed(at),
                        Some(constraints::Placement::Cancelled) => self.pointer_placement_failed(),
                        None => {}
                    }
                }
            }
            None => {}
        }
    }

    /// The pointer moved: if the viewer draws the cursor, request a frame
    /// immediately. An explicit maximum rate still applies to its draw.
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
        if let Some((_, usable)) = monitor.map(|m| m.work_area()).filter(|(_, size)| size.width > 0 && size.height > 0) {
            size = within_screen(usable, usable);
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
        if let Some(icon) = &self.config.application_icon {
            if let Err(error) = icon.apply(&window) {
                log::warn!("application icon: {error}");
            }
        }
        self.constraints.attach(&window);
        self.system_cursor_size = pointer_control::system_cursor_size(&window);
        log::info!("viewer: the system's cursor size is {} window pixels", self.system_cursor_size);
        let size = window.inner_size();
        let bitmap = std::env::var("TILE_VIEWER_SOFTWARE_PRESENT").is_ok_and(|v| !v.is_empty() && v != "0");
        #[cfg(not(target_os = "macos"))]
        if bitmap { return Err(ViewerError::SoftwarePresent("TILE_VIEWER_SOFTWARE_PRESENT is only supported on macOS".into())); }
        let initial = self.upload.take().expect("initial art retained until worker startup");
        self.cursor_art = cursor::Art::of(&initial);
        let wake_window = window.clone();
        let worker = render_worker::Worker::spawn(render_worker::Init {
            window: window.clone(),
            native_thread_init: self.config.native_thread_init, initial, base: self.whole_version, through: self.assets_version,
            size: (size.width.max(1), size.height.max(1)), profile: self.profile.is_some(), bitmap,
            wake_main: worker_wake(self.cursor_builds.wake.as_ref(), move || wake_window.request_redraw()),
        });
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: wgpu::TextureFormat::Bgra8Unorm, width: size.width.max(1), height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync, desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto, view_formats: vec![], color_space: wgpu::SurfaceColorSpace::Auto,
        };
        #[cfg(target_os = "macos")]
        let bitmap_presenter = if bitmap { Some(presentation::CpuBitmap::new(window.clone())?) } else { None };
        self.gpu = Some(Gpu { window, config, worker, ready: false, config_revision: 0, software: false, reconfigured: true,
            present_modes: vec![wgpu::PresentMode::Fifo], #[cfg(target_os = "macos")] bitmap: bitmap_presenter });
        self.configure_present();
        Ok(())
    }

    /// Follows the window's size, the driver's screen and the resolution
    /// settings ([`scaling::plan`]): tells the driver the size of its screen
    /// as the viewer arranges it when that changes, and maps the pointer
    /// accordingly.
    fn update_plan(&mut self) {
        let Some(gpu) = &self.gpu else { return };
        let window = (gpu.config.width, gpu.config.height);
        let waiting = !self.shown_scene_matches_art();
        let screen = if waiting {
            self.cached_picture.as_ref().map_or_else(
                || self.plan.map_or((0, 0), |p| p.screen),
                |picture| picture.plan.screen,
            )
        } else {
            self.shown_scene.screen_size
        };
        // A window: as large as the driver's screen at the desktop's scale,
        // if the compositor lets it.
        let scale = gpu.window.scale_factor();
        let key = (screen, (scale * 1000.0).round() as u64);
        if !self.config.fullscreen && self.sized_to_screen != Some(key) && screen.0 > 0 && screen.1 > 0 {
            self.sized_to_screen = Some(key);
            let mut want = winit::dpi::PhysicalSize::new((screen.0 as f64 * scale).round() as u32, (screen.1 as f64 * scale).round() as u32);
            // Keep the title bar and the whole client area inside the
            // usable desktop. The platform backend supplies reserved areas.
            let area = gpu.window.current_monitor().map(|m| m.work_area())
                .filter(|(_, size)| size.width > 0 && size.height > 0);
            let frame = window_frame(gpu.window.inner_size(), gpu.window.outer_size());
            if let Some(area) = area {
                want = within_screen(want, area.1);
                want = window_placement(want, frame, area).0;
            }
            log::info!("viewer: asking for a {}×{} window (the driver's {}×{} screen at scale {scale})", want.width, want.height, screen.0, screen.1);
            let _ = gpu.window.request_inner_size(want);
            // Wayland's compositor places windows itself.
            if let Some(area) = area {
                gpu.window.set_outer_position(window_placement(want, frame, area).1);
            }
        }
        // A compositor may choose a different initial client size. Check
        // the actual decorated bounds against the same usable desktop.
        if !self.config.fullscreen && self.fitted_on_screen != Some((window, key)) {
            self.fitted_on_screen = Some((window, key));
            if let (Ok(at), Some(m)) = (gpu.window.outer_position(), gpu.window.current_monitor()) {
                let (outer, area) = (gpu.window.outer_size(), m.work_area());
                if area.1.width > 0 && area.1.height > 0 && !window_in_work_area(at, outer, area) {
                    let inner = gpu.window.inner_size();
                    let (fits, position) = window_placement(inner, window_frame(inner, outer), area);
                    log::info!("viewer: fitting decorated window inside usable desktop {:?}: {}×{} at {position:?}", area, fits.width, fits.height);
                    if fits != inner { let _ = gpu.window.request_inner_size(fits); }
                    gpu.window.set_outer_position(position);
                }
            }
        }
        // The desktop's scale (for the driver; not the pixel scale): the window's, or a native
        // viewer's launcher's (`CIV3R_SCALE`, which sizes the game too; a
        // viewer in a Wine program has its window in Wine's own pixels,
        // which Wine scales for the desktop itself).
        let launcher = std::env::var("CIV3R_SCALE").ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 1.0);
        let desktop = if cfg!(windows) { scale } else { launcher.unwrap_or(scale) };
        let native = (window.0, window.1, (desktop * 1000.0).round() as u32);
        if native != self.reported_window {
            self.reported_window = native;
            frame_input(&mut self.preparing_inputs, &self.display, Input::Window { width: native.0, height: native.1, scale_milli: native.2 });
        }
        let plan = if waiting {
            self.cached_picture.as_ref().map(|picture| picture.plan_for_target(window))
        } else {
            None
        }.unwrap_or_else(|| scaling::plan(window, screen, &self.settings.resolution)
            .for_scene(&self.shown_scene, |s| sprite_rect(self.future_metadata.as_ref().filter(|m| m.base == self.whole_version && m.through == self.shown_scene.assets).map_or(self.compiled.as_ref(), |m| m.layout.as_ref()), s)));
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
                self.settings.resolution.describe_pixel_scale(),
                plan.screen.0,
                plan.screen.1,
                if plan.menu.zoom != (1, 1) || plan.frame != (1, 1) || plan.world != (1, 1) {
                    format!("; menus {}/{} (fitted content {:?}), frame {}/{}, world {}/{}", plan.menu.zoom.0, plan.menu.zoom.1, plan.menu_bounds, plan.frame.0, plan.frame.1, plan.world.0, plan.world.1)
                } else {
                    String::new()
                }
            );
            // (The system pointer's art at the new pixel scale.)
            if self.plan.map(|p| p.present.zoom) != Some(plan.present.zoom) {
                self.pointer_presentation.shown = None;
            }
            self.plan = Some(plan);
            self.layout.world_scale = plan.world;
            if let Some(moved) = self.input.set_plan(plan) {
                for input in self.layout.route(moved) {
                    self.send(input);
                }
            }
        }
        if plan.screen != self.reported_size {
            self.reported_size = plan.screen;
            frame_input(&mut self.preparing_inputs, &self.display, Input::Resized { width: plan.screen.0, height: plan.screen.1 });
        }
    }

    /// Captures (holds) or releases the mouse (the pointer is hidden and
    /// shown by `update_system_pointer`).
    fn grab(&mut self, lock: bool) {
        self.input_revision = self.input_revision.wrapping_add(1);
        if self.gpu.is_some() { self.constraints.grab(lock); }
    }

    fn draw(&mut self) {
        // (Measuring: frames that take long.)
        let started = Instant::now();
        // Compositor redraws and pointer requests must obey the same cap
        // as scene updates. Receiving input continues while they wait.
        if !self.frame_clock.request(started) {
            return;
        }
        if let Some(p) = &mut self.profile {
            p.round_start();
        }
        self.draw_frame(started);
        let took = started.elapsed();
        if took >= Duration::from_millis(40) && self.map_drawn {
            log::info!("viewer: a frame took {} ms", took.as_millis());
        }
        if let Some(p) = &mut self.profile {
            p.add("draw (all)", took);
            p.round_end();
            if let Some(report) = p.report() {
                log::info!("{report}");
            }
        }
    }

    /// Keeps the drawable atlas until a replacement and every growth its
    /// scene needs are ready. Only matching scenes activate prepared art.
    fn refresh_future_metadata(&mut self) {
        let whole = self.assets.receive();
        let target = self.display.scene().assets;
        // Never reinterpret numeric IDs across a whole replacement.
        if whole.generation != self.whole_version || target <= self.assets_version {
            if self.future_metadata.as_ref().is_some_and(|m| m.base != self.whole_version || m.through <= self.assets_version) {
                self.future_metadata = None;
            }
            return;
        }
        if self.future_metadata.as_ref().is_some_and(|m| m.base == self.whole_version && m.through == target) { return; }
        let queued = self.assets.growths.iter().map(|(version, growth)| (*version, growth.as_ref()));
        let work = scene_prefix::refresh_metadata(&mut self.future_metadata, self.whole_version, &self.compiled, self.assets_version, target, queued);
        if let Some(profile) = &mut self.profile {
            profile.count("future metadata copies", work.table_copies);
            profile.count("future growths applied", work.growths_applied);
        }
    }

    fn upload_art(&mut self) -> bool {
        self.receive_worker();
        let whole = self.assets.receive();
        let reserve = self.assets.reserve.filter(|(base, _, _)| *base == whole.generation).map_or((0, 0), |(_, w, h)| (w, h));
        if let Some(gpu) = &self.gpu {
            gpu.worker.art(render_worker::ArtSnapshot {
                base: whole.generation, whole: whole.compiled.clone(),
                growths: self.assets.growths.iter().map(|(version, growth)| (*version, growth.clone())).collect(),
                reserve, target: scene_art_target(&self.pending_scene, &self.display),
            });
        }
        false
    }

    fn receive_worker(&mut self) {
        self.report_latency();
        let events = self.gpu.as_ref().map_or_else(Vec::new, |gpu| gpu.worker.drain());
        for event in events {
            match event {
                render_worker::Event::SurfaceNeeded { instance } => {
                    if let Some(gpu) = &self.gpu {
                        gpu.worker.surface(instance.create_surface(gpu.window.clone()).map_err(|error| error.to_string()));
                    }
                }
                render_worker::Event::Ready { config, software, present_modes, base, through } => {
                    if let Some(gpu) = &mut self.gpu {
                        if gpu.config_revision == 0 { gpu.config = config; }
                        else { gpu.config.format = config.format; gpu.config.alpha_mode = config.alpha_mode; }
                        gpu.software = software;
                        gpu.present_modes = present_modes;
                        gpu.ready = true;
                        gpu.reconfigured = true;
                        gpu.window.set_visible(true);
                    }
                    self.whole_version = base;
                    self.assets_version = through;
                    self.assets.uploaded(base, through);
                    self.fps.timings.reset_from(self.next_frame_id);
                    self.configure_present();
                    self.configure_timing();
                }
                render_worker::Event::ArtCommitted { base, through, compiled, cursor_art } => {
                    let published_base = self.assets.receive().generation;
                    if base != published_base || base < self.whole_version || (base == self.whole_version && through < self.assets_version) { continue; }
                    self.compiled = compiled;
                    if let Some(profile) = &mut self.profile { profile.count("committed metadata snapshots", 1); }
                    self.whole_version = base;
                    self.assets_version = through;
                    self.pending_cursor_art = Some((base, (*cursor_art).clone()));
                    self.assets.uploaded(base, through);
                    // The worker now owns these pixels. Keep metadata only
                    // in main's inbox so future bounds never wait on uploads.
                    self.assets.growths.retain(|(version, _)| *version > through);
                }
                render_worker::Event::Presented { id, worker_prepare, .. } => self.commit_ui(id, worker_prepare),
                render_worker::Event::Bitmap { id, width, height, pixels, worker_prepare, .. } => {
                    // A reply queued before resize cannot be retracted by
                    // the worker. Validate before any OS bitmap mutation.
                    if !self.reply_matches_configuration(id) { self.commit_ui(id, worker_prepare); continue; }
                    #[cfg(target_os = "macos")]
                    {
                        let result = self.gpu.as_mut().and_then(|gpu| gpu.bitmap.as_mut()).ok_or_else(|| ViewerError::SoftwarePresent("bitmap presenter is absent".into()))
                            .and_then(|presenter| presenter.present(width, height, &pixels));
                        if let Err(error) = result { self.error = Some(error); continue; }
                        self.commit_ui(id, worker_prepare);
                    }
                    #[cfg(not(target_os = "macos"))]
                    { let _ = (id, width, height, pixels, worker_prepare); self.error = Some(ViewerError::SoftwarePresent("unexpected bitmap frame".into())); }
                }
                render_worker::Event::Retry { id } => {
                    if let Some(d) = &mut self.diagnostics { d.retried += 1; }
                    self.latency.pending.remove(&id);
                    if self.pending_ui.as_ref().is_some_and(|pending| pending.id == id) {
                        self.pending_ui = None;
                        self.world = None;
                        self.drawn_seq = 0;
                        self.world_command_version = 0;
                    }
                    if self.pending_replay.is_some_and(|(pending, _, _, _)| pending == id) { self.pending_replay = None; }
                }
                render_worker::Event::GpuCompleted { id, at } => {
                    self.latency.completed(id, at);
                }
                render_worker::Event::Timings(frames) => {
                    if self.settings.display.show_fps && self.settings.display.fps_detailed {
                        let seconds = self.started.elapsed().as_secs_f64();
                        for frame in frames { self.fps.timings.gpu_frame(seconds, &frame); }
                    }
                }
                render_worker::Event::Failed(error) => {
                    if let Some(request) = self.display.scene().camera_arrival.or(self.shown_scene.camera_arrival) {
                        self.report_camera_receipt(scene::Input::CameraUnavailable { serial: request.serial });
                    }
                    self.pending_ui = None;
                    self.pending_replay = None;
                    self.error = Some(ViewerError::RenderWorker(error));
                }
            }
        }
    }

    fn report_latency(&mut self) {
        if !log_latency() || self.latency_logged.elapsed() < Duration::from_secs(10) { return; }
        let mut done = Vec::new();
        while let Some(sample) = self.latency.done.try_recv() { done.push(sample); }
        if self.latency.start.is_empty() && done.is_empty() { return; }
        log::info!("viewer: cursor latency ms (p50, p95, max, n), motion to frame started: {:?}; to frame presented: {:?}; to GPU completion: {:?}", cursor::Latency::summary(&mut self.latency.start), cursor::Latency::summary(&mut self.latency.present), cursor::Latency::summary(&mut done));
        self.latency.start.clear();
        self.latency.present.clear();
        self.latency_logged = Instant::now();
    }

    fn reply_matches_configuration(&self, id: u64) -> bool {
        let Some(gpu) = &self.gpu else { return false; };
        self.pending_ui.as_ref().is_some_and(|pending| pending.id == id && pending.config_revision == gpu.config_revision)
            || self.pending_replay.is_some_and(|(pending, revision, _, _)| pending == id && revision == gpu.config_revision)
    }

    fn report_camera_receipt(&mut self, input: scene::Input) {
        let serial = match input { scene::Input::CameraArrived { serial } | scene::Input::CameraUnavailable { serial } => serial, _ => return };
        if self.camera_arrival_reported != Some(serial) {
            self.camera_arrival_reported = Some(serial);
            self.display.send_input(input);
        }
    }

    fn commit_ui(&mut self, id: u64, worker_prepare: Duration) {
        if self.pending_replay.is_some_and(|(pending, _, _, _)| pending == id) {
            let (_, revision, started, main_prepare) = self.pending_replay.take().expect("matched above");
            if self.gpu.as_ref().is_none_or(|gpu| gpu.config_revision != revision) { return; }
            self.record_presented(id, started, (main_prepare + worker_prepare).as_secs_f64() * 1000.0);
            return;
        }
        let pending = match ui_submission::complete(&mut self.pending_ui, id, self.gpu.as_ref().map(|gpu| gpu.config_revision)) {
            ui_submission::Completion::Unrelated => return,
            ui_submission::Completion::Obsolete => {
                if let Some(d) = &mut self.diagnostics { d.obsolete += 1; }
                self.world = None;
                self.drawn_seq = 0;
                self.world_command_version = 0;
                return;
            }
            ui_submission::Completion::Presented(pending) => pending,
        };
        let camera_receipt = pending.mapping.camera_arrival_receipt(&pending.frame);
        let unchanged_input = pending.input_revision == self.input_revision;
        let remapped_pointer = pending.mapping.activate(self, unchanged_input);
        self.worker_world_version = id;
        self.cursor_cameras = Some(pending.frame.cameras.clone());
        self.drawn_frame = Some(pending.frame);
        self.cached_picture = Some(pending.picture);
        for input in pending.inputs {
            if (!unchanged_input || remapped_pointer.is_some()) && matches!(input, Input::MouseMove { .. }) { continue; }
            frame_input(&mut self.preparing_inputs, &self.display, input);
        }
        if let Some(input) = remapped_pointer { frame_input(&mut self.preparing_inputs, &self.display, input); }
        self.feedback.presented(self.started.elapsed().as_secs_f64());
        if let Some(gpu) = &mut self.gpu { gpu.reconfigured = false; }
        self.record_presented(id, pending.started, (pending.main_prepare + worker_prepare).as_secs_f64() * 1000.0);
        self.refresh_presented_world_pointer();
        if let Some(receipt) = camera_receipt {
            if self.camera_arrival_reported != self.shown_scene.camera_arrival.map(|r| r.serial) { log::debug!("camera arrival: accepted frame {id}, {receipt:?}"); }
            self.report_camera_receipt(receipt);
        }
        let scene = self.shown_scene.as_ref();
        if !self.map_drawn && scene.shape.cols > 0 && scene.views.iter().any(|view| !view.view.interface_only) {
            self.map_drawn = true;
            log::info!("viewer: first map drawn (scene {})", scene.seq);
        }
        startup_ready::presented(scene, true, scene.views.iter().filter(|view| view.view.interface_only)
            .flat_map(|view| &view.overlay).any(|sprite| sprite_rect(&self.compiled, sprite).is_some_and(|[_, _, width, height]| width > 0 && height > 0)));
    }

    /// Counts actual presentations, including a replay whose cursor and
    /// counter pixels remain the previous snapshot.
    fn record_presented(&mut self, id: u64, started: Instant, cpu_build_ms: f64) {
        let now = Instant::now();
        self.frame_clock.presented(now);
        let seconds = now.duration_since(self.started).as_secs_f64();
        if self.settings.display.show_fps {
            self.fps.counter.presented(seconds);
            if self.settings.display.fps_detailed { self.fps.timings.cpu_frame(id, seconds, cpu_build_ms); }
        }
        self.frames_presented += 1;
        if let Some(d) = &mut self.diagnostics {
            d.presented += 1;
            let logical = self.cached_picture.as_ref().map(|p| (p.plan.present.canvas, p.pixel_box)).unwrap_or(((0,0),1));
            d.report_if_due(now.duration_since(self.started), id, logical.0, logical.1);
        }
        if let Some(since) = self.latency.pending.get(&id) {
            self.latency.present.push(now.saturating_duration_since(*since).as_secs_f64() * 1000.0);
            self.latency.last = Some(now);
        }
        if let Some(line) = self.frame_log.as_mut().and_then(|l| l.presented(seconds, now.duration_since(started).as_secs_f64() * 1000.0)) {
            log::info!("viewer: {line}");
        }
        self.presented = true;
    }

    fn draw_cached_picture(&mut self, started: Instant) {
        let Some(picture) = &self.cached_picture else { return };
        let Some(gpu) = &mut self.gpu else { return };
        if !gpu.ready || self.pending_ui.is_some() || self.pending_replay.is_some() { return; }
        let plan = picture.plan_for_target((gpu.config.width, gpu.config.height));
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        // (The worker notifies the window only when it presents: a frame it
        // retries must not hold back redraws, `render_worker::notify_present`.)
        let accepted = gpu.worker.frame(render_worker::Frame {
            id, config_revision: gpu.config_revision, base: self.whole_version, through: self.assets_version, world: None,
            world_version: self.worker_world_version, previous_world_version: self.worker_world_version,
            replace_world: false, dirty: tile_render::Dirty::All, free_changed: false,
            markers: Vec::new(), cameras: Vec::new(), copies: picture.copies.clone(), overlay_from: 0,
            tick: self.shown_scene.tick, host_tick_ms: self.started.elapsed().as_millis() as u32, clear: self.config.clear, present: picture.present(&plan),
            patches: Vec::new(), background: tile_render::BackgroundSampling::Dither, main_prepare: started.elapsed(),
        });
        if let Some(d) = &mut self.diagnostics {
            if accepted { d.accepted_replay += 1; } else { d.rejected += 1; }
        }
        if !accepted {
            self.error = Some(ViewerError::RenderWorker("rejected a cached frame".into()));
            return;
        }
        self.pending_replay = Some((id, gpu.config_revision, started, started.elapsed()));
        // A cached replay preserves the active input mapping.
    }

    fn draw_frame(&mut self, started: Instant) {
        self.receive_worker();
        self.upload_art();
        if self.pending_ui.is_some() || self.pending_replay.is_some() || self.gpu.as_ref().is_none_or(|gpu| !gpu.ready) { return; }
        let before = ui_submission::Mapping::capture(self);
        self.preparing_inputs = Some(Vec::new());
        self.prepare_frame(started);
        before.restore(self);
        self.preparing_inputs = None;
    }

    fn prepare_frame(&mut self, started: Instant) {
        let mut t = started;
        // Same-epoch layout tables can make the latest command group ready
        // immediately. Do not first clone it into a pending whole-world
        // snapshot that adoption would discard again in this same frame.
        self.refresh_future_metadata();
        let commands_through = self.future_metadata.as_ref().filter(|m| m.base == self.whole_version && m.through == self.display.scene().assets).map_or(self.assets_version, |m| m.through.max(self.assets_version));
        retain_pending_scene(&mut self.pending_scene, &self.shown_scene, &self.display, self.assets.whole().0, self.whole_version, commands_through);
        let art_committed = self.upload_art();
        let new_assets = art_committed || self.world_assets_version != self.assets_version;
        mark(&mut self.profile, "art transfers", &mut t);
        self.refresh_future_metadata();
        let commands_through = self.future_metadata.as_ref().filter(|m| m.base == self.whole_version && m.through == self.display.scene().assets).map_or(self.assets_version, |m| m.through.max(self.assets_version));
        let previous_scene_sequence = self.shown_scene.seq;
        let adoption = adopt_available_shared_scene(&mut self.shown_scene, &mut self.pending_scene, &mut self.display, self.whole_version, commands_through);
        let mut changes = match adoption {
            Some(adoption) => {
                for patch in adoption.patches {
                    self.patches.insert(patch.sprite.clone(), patch);
                }
                adoption.changes
            }
            None => scene::Changes::default(),
        };
        changes.sprites |= self.sprite_lifetimes.update(&self.shown_scene, started);
        let matches = self.shown_scene_matches_art();
        if !matches {
            // Normally the retained live scene keeps drawing while art is
            // prepared. Replay pixels only when no compatible scene exists.
            self.draw_cached_picture(started);
            return;
        }
        self.activate_cursor_art();
        self.update_plan();
        let Some(plan) = self.plan else { return };
        let software = self.gpu.as_ref().expect("checked above").software;
        let window = plan.canvas();
        let metadata = self.future_metadata.as_ref().filter(|m| m.base == self.whole_version && m.through == self.shown_scene.assets);
        let future_layout = metadata.map(|m| m.layout.clone());
        let scene = self.shown_scene.as_ref();
        let layout_compiled = future_layout.as_deref().unwrap_or(&self.compiled);
        let arranged = {
            let now = std::time::Instant::now();
            if let Some(at) = self.drawn_at.replace(now) {
                self.layout.advance(now - at);
            }
            let bounds = world_bounds(layout_compiled, scene);
            self.layout.grid_step = self.compiled.step;
            self.layout.driver_screen = Some(plan.screen);
            self.layout.menu_scale = plan.menu.zoom;
            self.layout.hud_scale = plan.frame;
            self.layout.window_pixel_scale = plan.factor();
            // (What the interface covers changes with the scene only.)
            let key = (scene.seq, self.assets_version);
            if self.interface.as_ref().is_none_or(|(k, _)| *k != key) {
                self.interface = Some((key, interface_rects(layout_compiled, scene)));
            }
            let rects = self.interface.as_ref().map(|(_, r)| r.clone()).unwrap_or_default();
            self.layout.screen_content = named_screen_content(layout_compiled, scene);
            let mut arranged = self.layout.frame(scene, window, self.config.overhang, rects, Some(bounds));
            let compiled = layout_compiled;
            let shown = scaling::arrange_panned(&plan, scene, &mut arranged, &mut self.layout.menu_pan, |s| sprite_rect(compiled, s));
            self.layout.set_interface_shown(shown.shown);
            // A middle drag suppresses hover while the menu moves. Restore
            // it through the map of the pixels this frame actually shows.
            if self.layout.menu_pan.take_hover_refresh() {
                if let Some((x, y)) = self.layout.route_window(self.layout.pointer_in_window()) {
                    let now = self.started.elapsed().as_secs_f64();
                    if let Some(input) = self.menu_scroll.route(Input::MouseMove { x, y }, now) {
                        self.feedback.sent(&input, &self.look_scene, now);
                        frame_input(&mut self.preparing_inputs, &self.display, input);
                    } else {
                        self.feedback.pointer_gone(now);
                    }
                }
            }
            self.drawn_overlay = shown.overlay_cameras;
            self.overlay_camera = shown.overlay;
            self.drawn_frame = Some(arranged.clone());
            arranged
        };
        mark(&mut self.profile, "layout", &mut t);
        // Views following something move without input: tell the driver.
        // (In this frame's world pixels: the driver takes them to its zoom
        // if that changed meanwhile, e.g. the game's zoom key.)
        let downscale = self.layout.reported_downscale();
        if let Some((x, y, w, h)) = self.layout.take_main_view_change() {
            log::debug!("viewer: own A now shows ({x}, {y}, {w}, {h}) at 1/{downscale}");
            frame_input(&mut self.preparing_inputs, &self.display, Input::MainView { x, y, w, h, downscale });
        }
        if let Some(views) = self.layout.take_views_change() {
            log::debug!("viewer: own views now show {views:?} at 1/{downscale}");
            frame_input(&mut self.preparing_inputs, &self.display, Input::Views { views, downscale });
        }
        if future_layout.is_none() { self.drawable_scene = None; }
        let render_scene = future_layout.as_ref().map(|future| {
            let (drawable, work) = scene_prefix::refresh_drawable_scene(&mut self.drawable_scene, self.assets_version, &self.compiled, future, scene, previous_scene_sequence, &changes);
            if let Some(profile) = &mut self.profile {
                profile.count("prefix full cell copies", work.full_cell_copies);
                profile.count("prefix changed cell copies", work.changed_cell_copies);
                profile.count("prefix cell mapping checks", work.cell_mapping_checks);
            }
            drawable
        });
        let scene = render_scene.as_deref().unwrap_or(scene);
        // Cells and free sprites have independent update paths. Local
        // interaction looks replace only the free sprites they affect.
        let views_moved = arranged.view_cameras != self.drawn_views || self.drawn_overlay != self.built_overlay;
        let full = new_assets || changes.world || self.world.is_none() || self.world_command_version != scene.assets;
        // (A view's own layers: slots the scene added, removed or changed.)
        let slots_changed = !full && (changes.slots || !changes.slot_cells.is_empty());
        let cells_changed = !full && (!changes.cells.is_empty() || slots_changed);
        let free_changed = !full && (changes.sprites || changes.views || views_moved);
        if full {
            // Never clone an old pinned world merely to discard it here.
            self.world = Some(Arc::new(scene_world_arranged(&self.compiled, scene, &arranged.view_cameras, &self.drawn_overlay)));
            mark(&mut self.profile, "world build", &mut t);
        } else if cells_changed || free_changed {
            let world = Arc::make_mut(self.world.as_mut().expect("existing world"));
            if cells_changed {
                scene_cells_changed(&self.compiled, scene, world, &changes.cells);
                if changes.slots {
                    scene_slots(&self.compiled, scene, world);
                } else {
                    scene_slot_cells_changed(&self.compiled, scene, world, &changes.slot_cells);
                }
                mark(&mut self.profile, "cells changed", &mut t);
            }
            if free_changed {
                world.free = scene_free_arranged(&self.compiled, scene, &arranged.view_cameras, &self.drawn_overlay);
                mark(&mut self.profile, "free sprites build", &mut t);
            }
        }
        if full || free_changed {
            for (index, sprite) in Arc::make_mut(self.world.as_mut().expect("built above")).free.iter_mut().enumerate() {
                sprite.clip = self.sprite_lifetimes.clip(index, sprite.clip);
            }
            self.base_free = self.world.as_ref().expect("built above").free.clone();
            self.spans = overlay_spans_arranged(scene, &arranged.view_cameras, &self.drawn_overlay);
            self.built_overlay = self.drawn_overlay.clone();
        }
        let looks_metadata_changed = (full || scene.seq != self.drawn_seq) && looks::LookScene::refresh_shared(&mut self.look_scene, scene);
        self.world_assets_version = self.assets_version;
        self.world_command_version = scene.assets;
        if let Some(p) = &mut self.profile {
            p.count("cells changed", changes.cells.len());
        }
        let now = self.started.elapsed().as_secs_f64();
        let replacements = self.feedback.replacements(&self.look_scene, now);
        let menu_was_scrolling = self.menu_scroll.running();
        let mut looks_changed = full || free_changed || looks_metadata_changed || self.feedback.drawn() != &self.drawn_looks[..]
            || self.menu_scroll.running();
        if looks_changed {
            let compiled = &self.compiled;
            Arc::make_mut(self.world.as_mut().expect("built above")).free = looks::splice(&self.base_free, &self.spans, &replacements, |s, camera| placed_sprite(compiled, s, Some(camera), None));
        }
        self.drawn_looks = self.feedback.drawn().to_vec();
        drop(replacements);
        let menu_pointer = (self.pointer_inside && !self.layout.captured() && !self.snap.snap_mode_active())
            .then(|| self.layout.route_window(self.layout.pointer_in_window())).flatten();
        if let Some(scrolled) = self.menu_scroll.draw(scene, &self.world.as_ref().expect("built above").free, &self.spans,
            now, self.settings.input.smooth_menu_scrolling && !self.snap.snap_mode_active(), self.whole_version, menu_pointer, |s| placed_rect(&self.compiled, s)) {
            Arc::make_mut(self.world.as_mut().expect("built above")).free = scrolled;
            looks_changed = true;
        }
        if (menu_was_scrolling || self.menu_scroll.running()) && self.pointer_inside && !self.layout.captured() && !self.snap.snap_mode_active() {
            if let Some(point) = self.layout.route_window(self.layout.pointer_in_window()) {
                match self.menu_scroll.refresh_pointer(point, now) {
                    smooth_menus::PointerRefresh::Move { x, y } => {
                        let input = Input::MouseMove { x, y };
                        self.feedback.sent(&input, &self.look_scene, now);
                        frame_input(&mut self.preparing_inputs, &self.display, input);
                    }
                    smooth_menus::PointerRefresh::Gone => self.feedback.pointer_gone(now),
                    smooth_menus::PointerRefresh::Unchanged => {}
                }
            }
        }
        // Units sliding to the next tile (`unit_motions`), where this
        // frame's time has them: the driver tells each slide once.
        if let Some(offsets) = self.unit_motions.offsets(scene, started, full || free_changed || looks_changed, |serial| self.display.motion_arrival(serial)) {
            if !offsets.is_empty() {
                unit_motions::apply(&mut Arc::make_mut(self.world.as_mut().expect("built above")).free, &scene.sprites, &offsets);
                looks_changed = true;
                // (A frame-timing trace of the slides: `TILE_VIEWER_MOTION_TRACE=1`.)
                static TRACE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| profile::env_on("TILE_VIEWER_MOTION_TRACE"));
                if *TRACE {
                    log::info!("viewer: motion at {:.1} ms: {offsets:?}", started.duration_since(self.started).as_secs_f64() * 1000.0);
                }
            }
        }
        // The viewer's markers on top of the world, for this frame only.
        let bobbing = arrow_bobbing_visible(self.settings.accessibility.arrow_bobbing, &arranged.markers, &arranged.cameras, world_downscale(scene));
        let mut markers = marker_sprites(&self.compiled, &arranged.markers, world_downscale(scene), self.started.elapsed(), bobbing);
        markers.extend(glass_sprites(&self.compiled, &arranged.divider));
        markers.extend(separator_sprites(&arranged.separators));
        // The cursor where the viewer draws it, in a camera of its own on
        // top (only when there is one: the frame is as before otherwise).
        let captured = self.layout.captured();
        let facts = cursor::Facts {
            captured,
            snap_mode: self.snap.snap_mode_active(),
            unlocking: self.unlock_pending.is_some(),
            world_cursor: arranged.markers.iter().filter_map(|m| if let layout::Marker::Cursor(p) = m { Some(*p) } else { None }).collect(),
            cursor_view: self.layout.world_cursor_view(),
            cursor_axes: self.layout.world_cursor_axes().unwrap_or((false, false)),
            pointer: if captured { self.system_pointer } else { self.layout.pointer_in_window() },
            pointer_inside: self.pointer_inside,
            pointer_world: self.layout.pointer_world(),
            interface_open: cursor::interface_open(scene),
            now: self.started.elapsed().as_secs_f64(),
        };
        let steps = self.machine.step(&facts);
        if steps.enter_edge.is_some() { self.layout.set_pointer_at_edge(false); }
        self.layout.set_pointer_at_edge(self.machine.edge());
        // A captured virtual cursor may show at its target tile. The normal
        // uncaptured pointer stays OS-rendered at its physical position;
        // logical tile targeting does not move or hide it.
        let snapping = captured && cursor::snaps_to_tiles(&scene.interface, &facts);
        let marks = if snapping {
            let n = world_downscale(scene);
            let step = (self.compiled.step.0 / n, self.compiled.step.1 / n);
            let staggered = self.compiled.layout == tile_render::Layout::Staggered;
            cursor::snap_to_tiles(steps.marks, &facts, |p| cursor::tile_centre(p, step, staggered))
        } else {
            steps.marks
        };
        // With edge scrolling the other panes do not mirror the pointer (it
        // would look like a second one); the zoom box still does.
        let marks = if self.layout.map_movement() == viewer_settings::MapMovement::EdgeScroll {
            cursor::Marks { mirror_only_in: Some(self.layout.zoom_area().into_iter().collect()), ..marks }
        } else {
            marks
        };
        if let Some(q) = steps.enter_edge {
            // A change from a line to a corner, or to another line, may
            // need a new Wayland position hint. Own the lock before it.
            self.constraints.grab(true);
            // The view following the world cursor is held at the world's
            // edge: the system pointer goes where the drawn cursor is and
            // shows there once this frame is out (`unlock_after_frame`).
            let w = plan.to_window(q);
            match self.constraints.move_pointer(w) {
                Ok(_) => {
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
            let locked = self.constraints.grab(true);
            log::info!("viewer: world cursor centred again: drawn, the system pointer captured (locked: {locked})");
        }
        let mut cameras = arranged.cameras.clone();
        // All scaled UI background sprites use it, including magnified art.
        let background = background_sampling(self.settings.resolution.background_sampling);
        for camera in &mut cameras {
            camera.background_sampling = background;
        }
        self.cursor = scene.cursor;
        // The cursor at its size: where it mirrors the pointer in another
        // view, in the cameras kept for it under the driver's interface;
        // the pointer itself on top.
        let overlay = cameras.len() as u32;
        // (As `cursor_scale`, with the GPU borrowed.)
        let cursor_scale = cursor::Scale::new(self.settings.display.effective_cursor_size(self.system_cursor_size), (plan.present.zoom.0.max(1) / plan.present.zoom.1.max(1)).max(1));
        let responsiveness = host_status::judge(self.display.host(), Instant::now());
        let busy_status = self.os_cursor.status(responsiveness);
        let cursor_art = cursor::sprite_set(&self.compiled, self.cursor_art.shown(self.cursor)).and_then(|set| cursor::SetArt::of(&self.compiled, set));
        let mut composed = match cursor_art {
            Some(art) if cameras.len() < tile_render::MAX_VIEWS => {
                let bounds = world_bounds(&self.compiled, scene);
                let period = (if bounds.wrap.0 { bounds.size.0 } else { 0 }, if bounds.wrap.1 { bounds.size.1 } else { 0 });
                cursor::compose_with_busy(&self.compiled, &self.cursor_art, art, &marks, &cameras, &arranged.cursor_slots, cursor_scale, window, overlay, period, busy_status)
            }
            _ => cursor::Composed::default(),
        };
        for (slot, camera) in &composed.slots {
            cameras[*slot as usize] = *camera;
        }
        // The snapping pointer and its ghost (while snapping, the only
        // cursor), in the same camera on top, in window pixels.
        // (At the cursor's size, as the cursor: its cameras after the
        // cursor's own.) Once it parked, only until the system pointer
        // shows there (as `pointer_hidden`): never both.
        let system_hidden = (captured && !self.machine.edge()) || facts.snap_mode || self.unlock_pending.is_some() || self.constraints.moving() || snapping;
        let snap = if cameras.len() < tile_render::MAX_VIEWS && system_hidden {
            self.snap.compose_with_busy(&self.compiled, &self.cursor_art, overlay, overlay + 1 + composed.top.len() as u32, cursor_scale, window, busy_status)
        } else {
            cursor::Composed::default()
        };
        composed.top.extend(snap.top);
        // While the system shows the pointer, its busy badge is part of the
        // system cursor's own image (`update_system_pointer`), never a
        // sprite here: a sprite would trail the pointer by a frame or more.
        // (A standard system icon, whose pixels the viewer does not have,
        // keeps its shape without a badge.)
        let cursor_sprites = composed.sprites;
        let mut snap_sprites = snap.sprites;
        if overlay < tile_render::MAX_VIEWS as u32 {
            if let Some([x, y, w, h]) = self.snap.keyboard_focus_rect(scene, &self.layout) {
                snap_sprites.extend(divider_sprites(&self.compiled, &[
                    (overlay, [x, y, w, 2], false),
                    (overlay, [x, y + h - 2, w, 2], false),
                    (overlay, [x, y, 2, h], true),
                    (overlay, [x + w - 2, y, 2, h], true),
                ]));
            }
        }
        if cursor_sprites.iter().any(|s| s.view == Some(overlay)) || !snap_sprites.is_empty() || !composed.top.is_empty() {
            cameras.push(cursor::overlay_camera(window));
            cameras.extend(composed.top);
        }
        // The frame counter over everything (the cursor too), by a camera of
        // its own showing the whole window 1:1 (whatever the split and
        // scales).
        let show_fps = self.settings.display.show_fps && cameras.len() < tile_render::MAX_VIEWS;
        let seconds = self.started.elapsed().as_secs_f64();
        if show_fps {
            let camera = cameras.len() as u32;
            markers.extend_from_slice(self.fps.sprites(&self.compiled, self.assets_version, seconds, window, camera, software, self.settings.display.fps_detailed));
            cameras.push(Camera { cells: false, ..Camera::new(window) });
        }
        // The host not answering (a busy or stuck game), over everything,
        // next to the driver's top-left buttons (`host_status`).
        let host = self.display.host();
        let stale = host_status::stale(host, Instant::now());
        self.host_status.note(responsiveness, stale);
        if responsiveness != host_status::Responsiveness::Answering && cameras.len() < tile_render::MAX_VIEWS {
            let anchor = host_status::anchor(&self.compiled, scene, &arranged.cameras, &arranged.view_cameras);
            if let Some((camera, sprites)) = host_status::compose(&self.compiled, responsiveness, anchor, window, seconds, cameras.len() as u32) {
                markers.extend(sprites);
                cameras.push(camera);
            }
        }
        markers.extend_from_slice(&cursor_sprites);
        markers.extend(snap_sprites);
        mark(&mut self.profile, "markers+cursor", &mut t);
        let world = self.world.as_ref().expect("built above");
        if let Some(p) = &mut self.profile {
            p.count("free sprites", world.free.len());
            p.count("markers", markers.len());
            p.count("cameras", cameras.len());
        }
        let dirty = take_world_dirty(self.world.as_mut().expect("built above"));
        let id = self.next_frame_id;
        if let Some(since) = self.pointer_since.take().filter(|_| log_latency() && (self.drawn_cursor != cursor_sprites || self.drawn_views != arranged.view_cameras)) {
            self.latency.begin(id, since, started);
        }
        self.next_frame_id += 1;
        let mut drawn = scaling::drawn(&cameras, &arranged.copies, plan.present, |zoom| self.layout.zoomed_art(zoom));
        // The pixel scale's confirmation: drawn onto the window at its
        // resolution after the frame composed as ever (its panel's pixels
        // only), not by composing the whole frame at the window's.
        let preview_seconds = self.settings_preview.seconds(Instant::now());
        self.settings_preview.drew(preview_seconds);
        if let Some(seconds) = preview_seconds {
            if drawn.cameras.len() < tile_render::MAX_VIEWS {
                let (camera, sprites) = settings_preview::compose_presented(&self.compiled, drawn.present, seconds, drawn.cameras.len() as u32);
                markers.extend(sprites);
                drawn.cameras.push(camera);
            }
        }
        let mut packet = render_worker::Frame {
            id, config_revision: self.gpu.as_ref().expect("initialized").config_revision, base: self.whole_version, through: self.assets_version,
            world: self.world.clone(), world_version: id, previous_world_version: self.worker_world_version,
            replace_world: new_assets || full, dirty, free_changed: free_changed || looks_changed,
            markers: markers.clone(), cameras: drawn.cameras, copies: drawn.copies.clone(),
            overlay_from: arranged.cameras.len(), tick: scene.tick, host_tick_ms: self.started.elapsed().as_millis() as u32, clear: self.config.clear, present: drawn.present,
            patches: self.patches.values().cloned().collect(), background, main_prepare: started.elapsed(),
        };
        self.drawn_markers = markers;
        self.drawn_cursor = cursor_sprites;
        self.drawn_views = arranged.view_cameras.clone();
        self.drawn_seq = scene.seq;
        self.drawn_tick = scene.tick;
        self.drawn_window = window;
        self.drawn_background = background;
        let mapping = ui_submission::Mapping::capture(self);
        let picture = CachedPicture { plan, copies: drawn.copies, pixel_box: drawn.pixel_box };
        let inputs = self.preparing_inputs.take().unwrap_or_default();
        self.pending_ui = Some(ui_submission::PendingUi { id, config_revision: packet.config_revision, input_revision: self.input_revision, mapping, started, main_prepare: packet.main_prepare, frame: arranged, picture, inputs });
        packet.main_prepare = started.elapsed();
        if let Some(pending) = &mut self.pending_ui { pending.main_prepare = packet.main_prepare; }
        let gpu = self.gpu.as_ref().expect("initialized above");
        // (Notified by the worker only when it presents this frame.)
        let accepted = gpu.worker.frame(packet);
        if let Some(d) = &mut self.diagnostics {
            if accepted { d.accepted_world += 1; } else { d.rejected += 1; }
        }
        if !accepted {
            self.pending_ui = None;
            self.error = Some(ViewerError::RenderWorker("rejected a complete frame".into()));
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
        if !matches!(event, WindowEvent::RedrawRequested) { self.input_revision = self.input_revision.wrapping_add(1); }
        match &event {
            WindowEvent::CloseRequested => {
                frame_input(&mut self.preparing_inputs, &self.display, Input::Quit);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.config_revision = gpu.config_revision.wrapping_add(1);
                    gpu.worker.configure(render_worker::Config { revision: gpu.config_revision, width: gpu.config.width, height: gpu.config.height,
                        present_mode: gpu.config.present_mode, latency: gpu.config.desired_maximum_frame_latency });
                    gpu.reconfigured = true;
                    log::debug!("viewer: window {}×{} pixels at scale {}", size.width, size.height, gpu.window.scale_factor());
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                log::info!("viewer: the window's scale is now {scale_factor}");
                if let Some(gpu) = &self.gpu {
                    self.system_cursor_size = pointer_control::system_cursor_size(&gpu.window);
                }
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
                if !self.settings_preview.active() && self.machine.edge() && self.unlock_pending.is_none() {
                    for input in self.layout.pointer_to(self.system_pointer) {
                        self.send(input);
                    }
                }
                if !self.settings_preview.active() { self.pointer_moved(); }
            }
            WindowEvent::CursorEntered { .. } => self.pointer_inside = true,
            WindowEvent::CursorLeft { .. } => self.pointer_inside = false,
            _ => {}
        }
        self.pointer_presentation.observe(&event);
        if matches!(event, WindowEvent::Focused(false) | WindowEvent::Occluded(true)) {
            self.menu_scroll.focus_lost();
            for input in self.snap.focus_lost(&mut self.layout) {
                self.send(input);
            }
            self.layout.released();
            self.unlock_pending = None;
            self.machine.cancel_edge();
            self.snap_grabbed = false;
            self.grab(false);
        }
        // The pointer is at no edge of the window once it left (or the
        // window is no longer the one the user works in).
        if matches!(event, WindowEvent::Focused(false) | WindowEvent::Occluded(true) | WindowEvent::CursorLeft { .. }) {
            for input in self.layout.pointer_gone() {
                self.send(input);
            }
            self.feedback.pointer_gone(self.started.elapsed().as_secs_f64());
        }
        let (now, rates) = (self.now(), snap::Rates::of(&self.settings.input));
        self.input.set_wheel_scale(self.gpu.as_ref().map_or(1.0, |gpu| gpu.window.scale_factor()));
        self.input.sync_wheel_context(self.display.scene(), self.layout.route_window(self.layout.pointer_in_window()), self.snap.snap_mode_active() || self.layout.captured());
        for input in self.input.translate(&event) {
            let pointer = self.input.window_pointer();
            let window = self.gpu.as_ref().map_or((1, 1), |g| (g.config.width, g.config.height));
            let (consumed, result) = self.settings_preview.input(&input, (pointer.0 as i32, pointer.1 as i32), window);
            self.resolve_settings_preview(result);
            if consumed { self.input.reset_wheel(); continue; }
            if self.settings_preview.active() && matches!(input, Input::Key { vk: 16..=18, .. } | Input::WorldKey { vk: 16..=18, .. }) {
                self.display.send_input(input);
                continue;
            }
            // (Through button snapping, which passes on what is not its.)
            let routed = self.snap.route(input.clone(), &mut self.layout, &self.shown_scene, now, rates);
            // The pointer moved where the driver does not get it, a release
            // the driver does not get: no longer over its interactables.
            match input {
                Input::MouseMove { .. } if motion_leaves_driver(self.layout.captured(), self.snap.active(), &routed) => self.feedback.pointer_gone(now),
                Input::MouseButton { button: scene::Button::Left, down: false, .. } if !routed.iter().any(|i| matches!(i, Input::MouseButton { button: scene::Button::Left, down: false, .. })) => self.feedback.released(now),
                _ => {}
            }
            for input in routed {
                self.send(input);
            }
        }
        self.show_keyboard_slider();
        // (After a redraw too: what the pointer is over may have changed.)
        self.apply_layout_requests(event_loop);
        if let Some(zooms) = self.layout.take_zooms_change() {
            frame_input(&mut self.preparing_inputs, &self.display, Input::Zooms(zooms));
        }
        // MainView/Views are consumed only while preparing a complete
        // frame, then delivered with its matching Presented reply.
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _id: winit::event::DeviceId, event: winit::event::DeviceEvent) {
        self.woken_idle = false;
        self.input_revision = self.input_revision.wrapping_add(1);
        // While the mouse is captured its motion moves the world cursor.
        // (While snapping, it moves the snapping pointer's hidden pointer.)
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            // (At the world's edge the system pointer moves it instead; in
            // the driver's pixels.)
            let edge = self.machine.edge() && self.unlock_pending.is_none();
            let s = self.plan.map_or(1.0, |p| p.factor());
            let delta = (delta.0 / s, delta.1 / s);
            let inputs = self.snap.motion(delta).unwrap_or_else(|| if edge { self.layout.edge_motion(delta) } else { self.layout.motion(delta) });
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
        self.receive_worker();
        self.update_system_pointer(event_loop);
        if self.error.is_some() || self.stop.load(std::sync::atomic::Ordering::Acquire) {
            event_loop.exit();
            return;
        }
        // Receive updates and input immediately. Without an explicit cap,
        // new updates also draw immediately; otherwise the event loop waits
        // for the next allowed frame while continuing to receive events.
        // Keep Wine's idle-wake workaround only for uncapped operation.
        let idle = std::mem::take(&mut self.woken_idle) && !self.frame_clock.capped
            && self.frame_clock.deadline().is_some_and(|t| t > Instant::now());
        let received = Instant::now();
        let mut changed = self.display.receive(if idle { IDLE_WAIT } else { Duration::ZERO });
        if let Some(p) = &mut self.profile {
            // (Waiting for nothing, when idle, is not the work of receiving.)
            if changed {
                p.add("receive (outside draw)", received.elapsed());
            }
            let seq = self.display.scene().seq;
            p.event("scenes", seq.saturating_sub(self.received_seq));
            self.received_seq = seq;
        }
        let lease = self.display.scene().settings_preview;
        let was_previewing = self.settings_preview.active();
        self.settings_preview.observe(lease, Instant::now());
        if was_previewing != self.settings_preview.active() { self.input.reset_wheel(); }
        if !was_previewing && self.settings_preview.active() {
            self.settings_preview.seed_held_keys(self.input.held_virtual_keys());
            for input in self.snap.focus_lost(&mut self.layout) { self.display.send_input(input); }
            self.layout.released();
        }
        if let Some(mut settings) = self.display.take_settings() {
            self.settings_preview.project(lease, &mut settings);
            if settings != self.settings {
                log::info!("viewer: new settings from the driver");
                self.set_settings(settings);
            }
        }
        let expired = self.settings_preview.tick(Instant::now());
        // (Only when what the confirmation shows changes: once a second.)
        changed |= self.settings_preview.needs_frame(Instant::now()) || expired.is_some();
        self.resolve_settings_preview(expired);
        let (now, rates) = (self.now(), snap::Rates::of(&self.settings.input));
        // The Alt mode by itself in menus, only for the window in front.
        let focused = self.pointer_presentation.active() && self.gpu.as_ref().is_some_and(|gpu| gpu.window.has_focus());
        self.snap.set_alt_in_menus(self.settings.input.alt_in_menus && focused);
        let snapped_inputs = if self.settings_preview.active() { Vec::new() } else {
            self.snap.tick(&mut self.layout, &self.shown_scene, now, rates)
        };
        if !snapped_inputs.is_empty() { self.input_revision = self.input_revision.wrapping_add(1); }
        for input in snapped_inputs { self.send(input); }
        self.show_keyboard_slider();
        // (Snapping may have put the pointer somewhere, or let go of it.)
        self.apply_layout_requests(event_loop);
        let now = Instant::now();
        let ready = self.frame_clock.ready(now, changed);
        if ready {
            // Pace retries even before the first scene arrives or while a
            // compositor redraw waits. Neither counts as a presentation.
            self.frame_clock.request(now);
        }
        if let Some(gpu) = &self.gpu {
            if (self.shown_scene.seq != 0 || self.display.scene().seq != 0) && ready {
                // Without vsync a frame is drawn now: a redraw request waits
                // for the compositor's next refresh (on Wayland), which
                // would cap the frame rate at the display's.
                if matches!(gpu.config.present_mode, wgpu::PresentMode::Mailbox | wgpu::PresentMode::Immediate) {
                    self.draw();
                    self.unlock_after_frame();
                    self.update_system_pointer(event_loop);
                } else {
                    gpu.window.request_redraw();
                }
            }
        }
        let waiting_worker = self.pending_ui.is_some() || self.pending_replay.is_some()
            || self.gpu.as_ref().is_some_and(|gpu| !gpu.ready);
        let flow = if waiting_worker {
            // Reply proxies and OS input wake immediately. A bounded idle
            // deadline keeps stop/timer checks alive without spinning a CPU
            // core while the GPU actor is busy or initializing.
            ControlFlow::WaitUntil(Instant::now() + DEFAULT_FRAME)
        } else {
            match self.frame_clock.wake_at(Instant::now()) {
                Some(at) => ControlFlow::WaitUntil(at),
                None => ControlFlow::Poll,
            }
        };
        let cursor_wake = self.os_cursor.next_animation_wake(self.pointer_hidden(), host_status::judge(self.display.host(), Instant::now()), Duration::from_secs_f64(self.now().max(0.0)));
        let flow = match (flow, cursor_wake) {
            (ControlFlow::WaitUntil(at), Some(delay)) => ControlFlow::WaitUntil(at.min(Instant::now() + delay)),
            (ControlFlow::Wait, Some(delay)) => ControlFlow::WaitUntil(Instant::now() + delay),
            (other, _) => other,
        };
        let flow = match (flow, self.settings_preview.wake(Instant::now())) {
            (ControlFlow::WaitUntil(at), Some(preview)) => ControlFlow::WaitUntil(at.min(preview)),
            (ControlFlow::Wait, Some(preview)) => ControlFlow::WaitUntil(preview),
            (flow, _) => flow,
        };
        event_loop.set_control_flow(flow);
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

fn window_frame(inner: winit::dpi::PhysicalSize<u32>, outer: winit::dpi::PhysicalSize<u32>) -> winit::dpi::PhysicalSize<u32> {
    winit::dpi::PhysicalSize::new(outer.width.saturating_sub(inner.width), outer.height.saturating_sub(inner.height))
}

/// Fit and center the decorated window, counting its title bar exactly once.
fn window_placement(
    inner: winit::dpi::PhysicalSize<u32>,
    frame: winit::dpi::PhysicalSize<u32>,
    area: (winit::dpi::PhysicalPosition<i32>, winit::dpi::PhysicalSize<u32>),
) -> (winit::dpi::PhysicalSize<u32>, winit::dpi::PhysicalPosition<i32>) {
    let inner = winit::dpi::PhysicalSize::new(
        inner.width.min(area.1.width.saturating_sub(frame.width)).max(1),
        inner.height.min(area.1.height.saturating_sub(frame.height)).max(1),
    );
    let axis = |start: i32, span: u32, content: u32, border: u32| {
        (start as i64 + (span as i64 - content as i64 - border as i64).max(0) / 2)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32
    };
    let at = winit::dpi::PhysicalPosition::new(
        axis(area.0.x, area.1.width, inner.width, frame.width),
        axis(area.0.y, area.1.height, inner.height, frame.height),
    );
    (inner, at)
}

fn window_in_work_area(
    at: winit::dpi::PhysicalPosition<i32>,
    size: winit::dpi::PhysicalSize<u32>,
    area: (winit::dpi::PhysicalPosition<i32>, winit::dpi::PhysicalSize<u32>),
) -> bool {
    at.x >= area.0.x && at.y >= area.0.y
        && at.x as i64 + size.width as i64 <= area.0.x as i64 + area.1.width as i64
        && at.y as i64 + size.height as i64 <= area.0.y as i64 + area.1.height as i64
}

#[cfg(test)]
mod window_work_area_tests {
    use super::*;
    use winit::dpi::{PhysicalPosition as Position, PhysicalSize as Size};

    #[test]
    fn dock_and_title_bar_are_both_outside_the_usable_client() {
        let area = (Position::new(0, 25), Size::new(1280, 706));
        let inner = Size::new(1152, 675);
        let frame = Size::new(0, 28);
        assert!(!window_in_work_area(Position::new(64, 40), Size::new(1152, 703), area));
        let (fits, at) = window_placement(inner, frame, area);
        assert_eq!(fits, inner);
        assert_eq!(at, Position::new(64, 26));
        assert!(window_in_work_area(at, Size::new(fits.width, fits.height + frame.height), area));
        let (fits, at) = window_placement(Size::new(1280, 800), frame, area);
        assert_eq!(fits, Size::new(1280, 678));
        assert_eq!(at, Position::new(0, 25));
    }

    #[test]
    fn side_dock_negative_monitor_origin_and_retina_use_physical_bounds() {
        let area = (Position::new(-1860, -1040), Size::new(1800, 1040));
        for scale in [1, 2, 3] {
            let scaled_area = (Position::new(area.0.x * scale, area.0.y * scale), Size::new(area.1.width * scale as u32, area.1.height * scale as u32));
            let inner = Size::new(1600 * scale as u32, 1000 * scale as u32);
            let frame = Size::new(16 * scale as u32, 40 * scale as u32);
            let (fits, at) = window_placement(inner, frame, scaled_area);
            assert_eq!(fits, inner);
            assert!(window_in_work_area(at, Size::new(fits.width + frame.width, fits.height + frame.height), scaled_area));
        }
        assert!(!window_in_work_area(Position::new(-1920, -1080), Size::new(1600, 1000), area));
    }

    #[test]
    fn large_coordinates_and_missing_frame_metrics_do_not_overflow() {
        let area = (Position::new(i32::MAX - 400, i32::MIN), Size::new(400, 300));
        let frame = window_frame(Size::new(800, 600), Size::new(0, 0));
        assert_eq!(frame, Size::new(0, 0));
        let (fits, at) = window_placement(Size::new(800, 600), frame, area);
        assert_eq!(fits, area.1);
        assert!(window_in_work_area(at, fits, area));
    }
}

/// System cursors made from cursor art at a pixel scale, each on a thread
/// of its own; by (cursor picture, pixel scale), including busy badges.
#[derive(Default)]
struct CursorBuilds {
    native_thread_init: Option<fn()>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// What is being made, and of which art (see `forget`).
    pending: std::collections::HashSet<(cursor::Shown, cursor::Scale)>,
    generation: u64,
    ready: Option<(std::sync::mpsc::Sender<CursorBuilt>, std::sync::mpsc::Receiver<CursorBuilt>)>,
}

type CursorBuilt = (u64, (cursor::Shown, cursor::Scale), Option<winit::window::CustomCursorSource>);

impl CursorBuilds {
    /// Makes the system cursor `key.0` of `art` at cursor scale `key.1`
    /// (`cursor::Art::native_image`: pointer and busy badge in one
    /// image), unless it is being made.
    fn start(&mut self, key: (cursor::Shown, cursor::Scale), art: Arc<cursor::Art>) {
        if !self.pending.insert(key) {
            return;
        }
        let tx = self.ready.get_or_insert_with(std::sync::mpsc::channel).0.clone();
        let wake = self.wake.clone();
        let generation = self.generation;
        let initialize = self.native_thread_init;
        std::thread::spawn(move || thread_start::initialized(initialize, || {
            let source = art.native_image(key.0, key.1).and_then(|native| {
                winit::window::CustomCursor::from_rgba(native.rgba, native.size.0, native.size.1, native.hotspot.0, native.hotspot.1)
                    .inspect_err(|e| log::warn!("viewer: cursor {:?}: {e}", key.0))
                    .ok()
            });
            if tx.send((generation, key, source)).is_ok() {
                if let Some(wake) = wake { wake(); }
            }
        }));
    }

    /// The cursors made since last asked (of the current art).
    fn take_ready(&mut self) -> Vec<((cursor::Shown, cursor::Scale), winit::window::CustomCursorSource)> {
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

#[cfg(test)]
mod snap_hover_tests {
    use super::{looks, motion_leaves_driver};
    use scene::{FreeSprite, Input, Interactable, InteractableKind, LookState, Looks, PointerState, ViewLooks};

    #[test]
    fn the_hidden_pointers_motion_keeps_the_snapped_rows_hover() {
        // Two menu rows; the driver shows row 2 hovered (the snapping
        // pointer landed there and the driver got it).
        let hover = LookState { pointer: PointerState::Hover, selected: false };
        let row = |id: u64, y: i32| Interactable { region: None, alt_right_click: false, rect: [0, y, 100, 20], anchor: None, kind: InteractableKind::Button, id };
        let looks_of = |id: u64, shown: LookState, sprites: (u32, u32)| Looks {
            id,
            select: scene::Select::None,
            shown,
            views: vec![ViewLooks { view: 5, sprites, states: vec![(hover, vec![FreeSprite { set: 11, ..Default::default() }]), (LookState::default(), vec![FreeSprite { set: 10, ..Default::default() }])] }],
        };
        let scene = looks::LookScene { interactables: vec![row(1, 0), row(2, 20)], looks: vec![looks_of(1, LookState::default(), (0, 1)), looks_of(2, hover, (1, 2))] };
        let mut feedback = looks::Feedback::default();
        feedback.sent(&Input::MouseMove { x: 50, y: 30 }, &scene, 0.0);
        assert!(feedback.replacements(&scene, 0.01).is_empty(), "as the driver shows it");
        // The system pointer moves while snapping: swallowed, not gone.
        assert!(!motion_leaves_driver(false, true, &[]));
        // (Without snapping, a move the driver does not get takes it off.)
        assert!(motion_leaves_driver(false, false, &[]));
        assert!(!motion_leaves_driver(true, false, &[]));
        assert!(!motion_leaves_driver(false, false, &[Input::MouseMove { x: 1, y: 1 }]));
        // Had it been taken off, row 2 would be drawn unhovered.
        feedback.pointer_gone(0.02);
        assert_eq!(feedback.replacements(&scene, 0.03).len(), 1);
    }
}

#[cfg(test)]
mod cursor_completion_tests {
    use super::*;

    thread_local! { static INITIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

    fn initialize_cursor_thread() { INITIALIZED.set(true); }

    #[test]
    fn cursor_worker_initializes_before_waking_main() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut builds = CursorBuilds::default();
        builds.native_thread_init = Some(initialize_cursor_thread);
        builds.wake = Some(Arc::new(move || { tx.send(INITIALIZED.get()).unwrap(); }));
        builds.start((cursor::Shown::Custom(7), cursor::Scale::default()), Arc::new(cursor::Art::with_image(7, cursor::Image {
            rgba: vec![255, 255, 255, 255], size: (1, 1), hotspot: (0, 0),
        })));
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        assert_eq!(builds.take_ready().len(), 1);
        assert!(!INITIALIZED.get(), "caller context must remain untouched");
    }


    #[test]
    fn prepared_cursor_wakes_the_event_loop_without_a_rendered_frame() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut builds = CursorBuilds::default();
        builds.wake = Some(Arc::new(move || { let _ = tx.send(()); }));
        builds.start((cursor::Shown::Custom(7), cursor::Scale::default()), Arc::new(cursor::Art::with_image(7, cursor::Image {
            rgba: vec![255, 255, 255, 255], size: (1, 1), hotspot: (0, 0),
        })));
        rx.recv_timeout(Duration::from_secs(5)).expect("cursor worker must wake main independently of frame completion");
        assert_eq!(builds.take_ready().len(), 1);
    }
}

#[cfg(test)]
mod incremental_scene_adoption_tests {
    use super::*;
    #[test]
    fn control_only_adoption_reuses_owned_world_and_pinned_readers_stay_immutable() {
        let (driver, mut display) = scene::connect();
        let mut shown = Arc::new(Scene::default());
        let mut pending = None;
        assert!(driver.send(vec![scene::Update::World { shape: scene::WorldShape { cols: 1024, rows: 1, ..Default::default() }, layers: vec![scene::LayerCells { values: vec![1; 1024], ..Default::default() }] }]));
        display.receive(Duration::ZERO);
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_some());
        let pointer = shown.layers[0].values.as_ptr();
        assert!(driver.send(vec![scene::Update::Tick(17)])); display.receive(Duration::ZERO);
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_some());
        assert_eq!(shown.layers[0].values.as_ptr(), pointer);
        let pinned = shown.clone();
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_none());
        assert!(Arc::ptr_eq(&shown, &pinned));
        assert!(driver.send(vec![scene::Update::Cells(vec![(0, 31, 9)]), scene::Update::Tick(19)])); display.receive(Duration::ZERO);
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_some());
        assert_eq!(*shown, *display.scene());
        assert_eq!(pinned.layers[0].values[31], 1);
        assert_eq!(pinned.tick, 17);
        assert_eq!(shown.layers[0].values[31], 9);
    }
}

#[cfg(test)]
mod shared_world_dirty_tests {
    use super::*;
    #[test]
    fn unchanged_published_world_keeps_identity_while_real_dirty_records_are_owned() {
        let mut value = TileWorld::new(tile_render::Grid { cols: 4, rows: 1, layout: tile_render::Layout::Orthogonal, wrap_x: false, wrap_y: false, step: (8, 8) }, tile_render::SpriteSets::default());
        value.add_layer(tile_render::LayerSpec::new("cells", vec![tile_render::SpriteSetId::EMPTY; 2]));
        value.take_dirty();
        let mut world = Arc::new(value);
        let pinned = world.clone();
        assert_eq!(take_world_dirty(&mut world), tile_render::Dirty::None);
        assert!(Arc::ptr_eq(&world, &pinned), "a clean frame must not copy pinned cell records");
        drop(pinned);
        Arc::get_mut(&mut world).unwrap().set(tile_render::LayerId(0), 2, 0, 1);
        let pinned = world.clone();
        assert_eq!(take_world_dirty(&mut world), tile_render::Dirty::Records(vec![(0, 2)]));
        assert_eq!(*world.dirty(), tile_render::Dirty::None);
        assert_eq!(*pinned.dirty(), tile_render::Dirty::Records(vec![(0, 2)]));
        assert_eq!(pinned.get(tile_render::LayerId(0), 2, 0), 1);
        drop(pinned);
        let slot = Arc::get_mut(&mut world).unwrap().add_slot(tile_render::LayerId(0));
        take_world_dirty(&mut world);
        Arc::get_mut(&mut world).unwrap().bind(7, tile_render::LayerId(0), Some(slot));
        let pinned = world.clone();
        assert_eq!(take_world_dirty(&mut world), tile_render::Dirty::Records(vec![]), "binding-only changes must still be published");
        assert_eq!(*world.dirty(), tile_render::Dirty::None);
        assert_eq!(*pinned.dirty(), tile_render::Dirty::Records(vec![]));
    }
}

#[cfg(test)]
mod rejected_snapshot_recovery_tests {
    use super::*;
    use scene::{Changes, FreeSprite, LayerCells, Update, View, WorldShape};

    fn connected() -> (scene::Driver, Display, Arc<Scene>, Option<PendingScene>) {
        let (driver, mut display) = scene::connect();
        let mut shown = Arc::new(Scene::default());
        let mut pending = None;
        assert!(driver.send(vec![Update::World { shape: WorldShape { cols: 4, rows: 1, ..Default::default() }, layers: vec![LayerCells { values: vec![1; 4], phases: Some(vec![0; 4]), offsets: Some(vec![(0, 0); 4]) }] }, Update::Assets(1)]));
        assert!(display.receive(Duration::ZERO));
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_some());
        (driver, display, shown, pending)
    }
    fn assert_full(changes: &Changes) {
        assert!(changes.world && changes.sprites && changes.views && changes.slots,
            "a mismatched consumed-delta predecessor must invalidate world, sprites, views and slots");
        assert!(changes.cells.is_empty() && changes.slot_cells.is_empty());
    }
    #[test]
    fn rejected_preparation_restores_complete_latest_scene_after_delta_was_consumed() {
        let (driver, mut display, mut shown, mut pending) = connected();
        let committed = shown.clone();
        assert!(driver.send(vec![Update::Cells(vec![(0, 0, 9)]), Update::Phases(vec![(0, 0, 13)]), Update::Offsets(vec![(0, 0, (-3, 7))]),
            Update::Sprites(vec![FreeSprite { x: 41, ..Default::default() }]), Update::View { id: 7, view: Some(View { camera_x: 111, ..Default::default() }) },
            Update::Overlay { view: 7, sprites: vec![FreeSprite { x: 19, ..Default::default() }] },
            Update::Slot { id: 3, slot: Some((0, LayerCells { values: vec![2; 4], ..Default::default() })) },
            Update::Bindings(vec![scene::Binding { id: 5, slots: vec![(0, 3)] }]), Update::Tick(17)]));
        display.receive(Duration::ZERO);
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).is_some());
        assert_eq!(*shown, *display.scene());
        assert!(display.changes().is_empty());
        // The presented input mapping stays committed when acquisition or
        // worker submission rejects this preparation. Its deltas are gone.
        shown = committed.clone();
        let recovered = adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).unwrap();
        assert_eq!(*shown, *display.scene(), "re-adoption must not leave stale cells, views or slots");
        assert_full(&recovered.changes);
        assert_eq!(committed.layers[0].values, [1; 4]);
        assert!(committed.views.is_empty() && committed.slots.is_empty());
    }
    #[test]
    fn new_delta_after_rejected_preparation_cannot_hide_the_earlier_changed_cell() {
        let (driver, mut display, mut shown, mut pending) = connected();
        let committed = shown.clone();
        assert!(driver.send(vec![Update::Cells(vec![(0, 0, 9)]), Update::Tick(17)]));display.receive(Duration::ZERO);
        adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).unwrap();
        shown = committed;
        assert!(driver.send(vec![Update::Cells(vec![(0, 1, 7)]), Update::Tick(19)]));display.receive(Duration::ZERO);
        let recovered = adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).unwrap();
        assert_eq!(shown.layers[0].values, [9, 7, 1, 1]);
        assert_eq!(*shown, *display.scene());
        assert_full(&recovered.changes);
    }
    #[test]
    fn pending_scene_and_later_latest_scene_require_their_actual_delta_predecessor() {
        let (driver, mut display, mut shown, mut pending) = connected();
        assert!(driver.send(vec![Update::Assets(2), Update::Cells(vec![(0, 0, 9)])]));display.receive(Duration::ZERO);
        retain_pending_scene(&mut pending, &shown, &display, 1, 1, 1);
        assert!(driver.send(vec![Update::Assets(3), Update::Cells(vec![(0, 1, 7)])]));display.receive(Duration::ZERO);
        assert!(adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 2).unwrap().changes.world,
            "pending adoption already replaces the whole world, including slots");
        assert_eq!(shown.layers[0].values, [9, 1, 1, 1]);
        let adopted = adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 3).unwrap();
        assert_eq!(*shown, *display.scene());
        assert_full(&adopted.changes);
    }
    #[test]
    fn uninterrupted_delta_adoption_preserves_the_existing_incremental_path() {
        let (driver, mut display, mut shown, mut pending) = connected();
        let cells = shown.layers[0].values.as_ptr();
        assert!(driver.send(vec![Update::Cells(vec![(0, 2, 11)]), Update::Tick(23)]));display.receive(Duration::ZERO);
        let adopted = adopt_available_shared_scene(&mut shown, &mut pending, &mut display, 1, 1).unwrap();
        assert!(!adopted.changes.world && !adopted.changes.sprites && !adopted.changes.views && !adopted.changes.slots);
        assert_eq!(adopted.changes.cells, [(0, 2)]);
        assert_eq!(shown.layers[0].values.as_ptr(), cells);
        assert_eq!(*shown, *display.scene());
    }
}
