//! What a game driver tells a renderer, and what the renderer tells back.
//!
//! The renderer owns the whole world: a [`Scene`] with the grid shape, for
//! every layer of the render description one value per cell (0 = nothing;
//! other values were resolved from set names via the description), the
//! world's sprites, and any number of views of it. The driver (the only
//! game-specific part) sends [`Update`]s; the renderer applies them, draws
//! every view and queues [`Input`] for the driver to apply to the game.
//!
//! [`connect`] links the two sides, each on its own thread.

pub mod wire;


use std::sync::mpsc;
use std::time::Duration;

/// The visible part of the world.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// World pixel at the top-left of the viewport.
    pub camera_x: i32,
    pub camera_y: i32,
    /// Screen pixels per world pixel (a renderer may arrange it otherwise).
    pub zoom: (u32, u32),
    /// Viewport size in screen pixels.
    pub viewport: (u32, u32),
    /// Draw only cells whose logical anchor lies in this world-pixel
    /// rectangle (x0, y0, x1, y1), if set (games that clip by tile range).
    pub cull: Option<(i32, i32, i32, i32)>,
    /// Where the view sits in the window (its top-left, window pixels).
    pub origin: (i32, i32),
    /// Shows only its overlay, not the world (an interface on top).
    pub interface_only: bool,
}

/// Shape of the cell grid (layout and spacing come from the description).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldShape {
    pub cols: u32,
    pub rows: u32,
    pub wrap_x: bool,
    pub wrap_y: bool,
}

/// Cell values of one layer, row-major, `cols × rows`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerCells {
    pub values: Vec<u32>,
    /// Per-cell animation phase, if the layer has one.
    pub phases: Option<Vec<u8>>,
}

/// A sprite not tied to a cell (units, effects, UI).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FreeSprite {
    /// Sprite set (animation) id, resolved from a set name by the
    /// description.
    pub set: u32,
    /// Anchor position: world pixels, or viewport pixels if `screen`.
    pub x: i32,
    pub y: i32,
    pub screen: bool,
    /// Drawn after this render pass (beyond the last: on top).
    pub after_pass: u32,
    pub phase: u32,
    pub ticks_per_frame: u32,
    /// 0: alpha blending; otherwise darkens like a scale layer.
    pub scale_den: u32,
    /// The colour of ClearType-like text (`scale_den` = the renderer's
    /// `SCALE_SUBPIXEL`), 0xRRGGBB.
    pub tint: u32,
    /// 0 any zoom; 1 the 1:1 twin of sprites made for zooms, left out in
    /// views at a zoom the scene has art for ([`Scene::zooms`]); 2 made for
    /// `zoom`: drawn in views at that zoom, at the sprite's 1:1 position
    /// scaled plus `zoom_offset` (pixels at that zoom).
    pub zoom_art: u8,
    pub zoom: (u16, u16),
    pub zoom_offset: (i32, i32),
    /// Drawn only inside this viewport rectangle (left, top, right,
    /// bottom; exclusive).
    pub clip: Option<[i32; 4]>,
}

/// One view of the world: where it sits in the window, what it looks at,
/// and what is drawn over it in screen space (an interface).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Chosen by the driver; views are drawn in id order (higher on top).
    pub id: u32,
    pub view: View,
    /// Screen-space sprites (window pixels) drawn in this view only.
    pub overlay: Vec<FreeSprite>,
}

/// What the renderer knows: the whole world and the views of it. Drivers
/// send [`Update`]s; the renderer applies them ([`Scene::apply`]) and draws
/// every view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    /// Increases with every applied batch of updates.
    pub seq: u64,
    /// Animation clock.
    pub tick: u32,
    pub shape: WorldShape,
    /// In the description's layer order.
    pub layers: Vec<LayerCells>,
    /// World-space sprites (units, effects), drawn in every view, in
    /// drawing order (within the pass each follows).
    pub sprites: Vec<FreeSprite>,
    /// By id.
    pub views: Vec<ViewState>,
    /// The zooms the scene has zoomed art for (see `FreeSprite::zoom_art`).
    pub zooms: Vec<(u32, u32)>,
    /// The generation of the art (render assets) the scene's sprite ids
    /// refer to; 0: whatever the renderer has.
    pub assets: u64,
    /// The world pixel the driver considers its point of interest (e.g. the
    /// unit a player is moving), if any.
    pub focus: Option<(i32, i32)>,
    /// New pixels for atlas sprites whose content changes at run time.
    pub patches: Vec<Patch>,
}

/// The pixels of one atlas sprite (row-major RGBA, its exact size).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    /// Sprite name (`Compiled::sprite_names`): ids change with every
    /// compile, and patches outlive compiles.
    pub sprite: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One change of a [`Scene`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// A new world: shape and every layer (replaces all cells).
    World { shape: WorldShape, layers: Vec<LayerCells> },
    /// Some cells changed: (layer, cell index, value).
    Cells(Vec<(u32, u32, u32)>),
    /// The world-space sprites (replaced).
    Sprites(Vec<FreeSprite>),
    /// View `id` added or changed (its overlay kept), or removed (`None`).
    View { id: u32, view: Option<View> },
    /// The screen-space sprites over view `id` (replaced).
    Overlay { view: u32, sprites: Vec<FreeSprite> },
    /// Sprite pixels to write into the atlas.
    Patches(Vec<Patch>),
    Tick(u32),
    /// The point of interest moved (see [`Scene::focus`]).
    Focus(Option<(i32, i32)>),
    /// The scene now refers to art generation `n` (see [`Scene::assets`]).
    Assets(u64),
    /// See [`Scene::zooms`].
    Zooms(Vec<(u32, u32)>),
}

impl Scene {
    pub fn empty(shape: WorldShape, layer_count: usize) -> Self {
        let n = (shape.cols * shape.rows) as usize;
        Scene { shape, layers: vec![LayerCells { values: vec![0; n], phases: None }; layer_count], ..Default::default() }
    }

    pub fn set(&mut self, layer: usize, col: u32, row: u32, value: u32) {
        let i = (row * self.shape.cols + col) as usize;
        self.layers[layer].values[i] = value;
    }

    /// View `id`, if there is one.
    pub fn view(&self, id: u32) -> Option<&ViewState> {
        self.views.iter().find(|v| v.id == id)
    }

    /// View `id`, added (at the origin, nothing over it) if missing.
    pub fn view_mut(&mut self, id: u32) -> &mut ViewState {
        let i = match self.views.binary_search_by_key(&id, |v| v.id) {
            Ok(i) => i,
            Err(i) => {
                self.views.insert(i, ViewState { id, ..Default::default() });
                i
            }
        };
        &mut self.views[i]
    }

    /// Applies one update.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::World { shape, layers } => {
                self.shape = shape;
                self.layers = layers;
            }
            Update::Cells(changes) => {
                for (layer, cell, value) in changes {
                    if let Some(v) = self.layers.get_mut(layer as usize).and_then(|l| l.values.get_mut(cell as usize)) {
                        *v = value;
                    }
                }
            }
            Update::Sprites(sprites) => self.sprites = sprites,
            Update::View { id, view: Some(view) } => self.view_mut(id).view = view,
            Update::View { id, view: None } => self.views.retain(|v| v.id != id),
            Update::Overlay { view, sprites } => self.view_mut(view).overlay = sprites,
            Update::Patches(patches) => self.patches.extend(patches),
            Update::Tick(tick) => self.tick = tick,
            Update::Focus(focus) => self.focus = focus,
            Update::Assets(n) => self.assets = n,
            Update::Zooms(z) => self.zooms = z,
        }
    }

    /// The updates that turn `old` into `self` (as small as cheaply found:
    /// changed cells rather than whole layers when the shape is the same).
    pub fn updates_since(&self, old: &Scene) -> Vec<Update> {
        let mut out = Vec::new();
        let same_layout = self.shape == old.shape
            && self.layers.len() == old.layers.len()
            && self.layers.iter().zip(&old.layers).all(|(a, b)| a.values.len() == b.values.len() && a.phases == b.phases);
        if !same_layout {
            out.push(Update::World { shape: self.shape, layers: self.layers.clone() });
        } else {
            let mut cells = Vec::new();
            for (l, (new, old)) in self.layers.iter().zip(&old.layers).enumerate() {
                for (i, (a, b)) in new.values.iter().zip(&old.values).enumerate() {
                    if a != b {
                        cells.push((l as u32, i as u32, *a));
                    }
                }
            }
            if !cells.is_empty() {
                out.push(Update::Cells(cells));
            }
        }
        if self.sprites != old.sprites {
            out.push(Update::Sprites(self.sprites.clone()));
        }
        for v in &old.views {
            if self.view(v.id).is_none() {
                out.push(Update::View { id: v.id, view: None });
            }
        }
        for v in &self.views {
            let before = old.view(v.id);
            if before.map(|b| b.view) != Some(v.view) {
                out.push(Update::View { id: v.id, view: Some(v.view) });
            }
            let overlay_before = before.map_or(&[][..], |b| &b.overlay[..]);
            if overlay_before != &v.overlay[..] {
                out.push(Update::Overlay { view: v.id, sprites: v.overlay.clone() });
            }
        }
        if !self.patches.is_empty() {
            out.push(Update::Patches(self.patches.clone()));
        }
        if self.tick != old.tick {
            out.push(Update::Tick(self.tick));
        }
        if self.focus != old.focus {
            out.push(Update::Focus(self.focus));
        }
        if self.assets != old.assets {
            out.push(Update::Assets(self.assets));
        }
        if self.zooms != old.zooms {
            out.push(Update::Zooms(self.zooms.clone()));
        }
        out
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// User input captured by the renderer, in viewport pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: Button, down: bool, x: i32, y: i32 },
    Wheel { delta: i32, x: i32, y: i32 },
    /// A key, as a Windows virtual-key code (the de-facto lingua franca of
    /// keyboard input), with the character it produced, if any.
    Key { vk: u32, down: bool, text: Option<char> },
    /// The renderer's drawing area now has this size (sent at start and on
    /// every resize).
    Resized { width: u32, height: u32 },
    /// The renderer's window was closed.
    Quit,
    /// The renderer shows the world at these zooms (`(num, den)` reduced;
    /// `(0, 0)`: unused): a driver may make art for them (e.g. text at that
    /// size, `FreeSprite::zoom_art` 2) and list them in `Scene::zooms`.
    Zooms([(u8, u8); 8]),
    /// The user wants to see world pixel (x, y) in the driver's main view
    /// (e.g. clicked it in another view): a driver may move its camera there.
    Look { x: i32, y: i32 },
}

/// Connects a driver (the game side) and a renderer, each on its own
/// thread: updates flow to the renderer, input flows back, both through
/// multiple-producer single-consumer queues ([`std::sync::mpsc`]).
pub fn connect() -> (Driver, Display) {
    let (updates_tx, updates_rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    (Driver { updates: updates_tx, input: input_rx }, Display { updates: updates_rx, input: input_tx, scene: Scene::default() })
}

/// A sender of update batches (any number of them may feed one renderer).
pub type Updater = mpsc::Sender<Vec<Update>>;
/// A sender of input (any number of views or windows may report to one
/// driver).
pub type InputSender = mpsc::Sender<Input>;

/// The driver's end: sends updates, takes input.
pub struct Driver {
    updates: Updater,
    input: mpsc::Receiver<Input>,
}

impl Driver {
    /// Sends a batch of updates, applied by the renderer at once; false
    /// once the renderer is gone.
    pub fn send(&self, batch: Vec<Update>) -> bool {
        batch.is_empty() || self.updates.send(batch).is_ok()
    }

    /// Another sender to the same renderer.
    pub fn updater(&self) -> Updater {
        self.updates.clone()
    }

    /// All input queued so far.
    pub fn take_input(&self) -> Vec<Input> {
        self.input.try_iter().collect()
    }
}

/// The renderer's end: owns the [`Scene`] the updates build.
pub struct Display {
    updates: mpsc::Receiver<Vec<Update>>,
    input: InputSender,
    scene: Scene,
}

impl Display {
    /// Applies every batch received so far, waiting up to `timeout` for the
    /// first if none is there; whether anything changed.
    pub fn receive(&mut self, timeout: Duration) -> bool {
        let mut changed = false;
        if let Ok(batch) = self.updates.recv_timeout(timeout) {
            self.apply(batch);
            changed = true;
        }
        while let Ok(batch) = self.updates.try_recv() {
            self.apply(batch);
            changed = true;
        }
        changed
    }

    fn apply(&mut self, batch: Vec<Update>) {
        for u in batch {
            self.scene.apply(u);
        }
        self.scene.seq += 1;
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The sprite pixels received since the last call (to write into the
    /// atlas once).
    pub fn take_patches(&mut self) -> Vec<Patch> {
        std::mem::take(&mut self.scene.patches)
    }

    pub fn send_input(&self, input: Input) {
        let _ = self.input.send(input);
    }

    /// Another sender of input to the same driver.
    pub fn input_sender(&self) -> InputSender {
        self.input.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_and_input_cross_threads() {
        let (driver, mut display) = connect();
        let t = std::thread::spawn(move || {
            let mut s = Scene::empty(WorldShape { cols: 2, rows: 2, ..Default::default() }, 1);
            let empty = Scene::default();
            driver.send(s.updates_since(&empty));
            let before = s.clone();
            s.set(0, 1, 1, 42);
            s.view_mut(0).view.camera_x = 5;
            s.view_mut(3).overlay.push(FreeSprite { set: 9, screen: true, ..Default::default() });
            assert_eq!(s.updates_since(&before).len(), 4);
            driver.send(s.updates_since(&before));
            driver
        });
        let driver = t.join().unwrap();
        assert!(display.receive(Duration::from_secs(5)));
        assert_eq!(display.scene().layers[0].values, vec![0, 0, 0, 42]);
        assert_eq!(display.scene().views.iter().map(|v| v.id).collect::<Vec<_>>(), vec![0, 3]);
        assert_eq!(display.scene().view(0).unwrap().view.camera_x, 5);
        assert!(!display.receive(Duration::from_millis(1)));
        display.send_input(Input::Key { vk: 0x0D, down: true, text: None });
        let other = display.input_sender();
        other.send(Input::Quit).unwrap();
        assert_eq!(driver.take_input().len(), 2);
        assert!(driver.take_input().is_empty());
    }
}
