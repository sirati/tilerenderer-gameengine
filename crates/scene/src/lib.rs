//! What a game driver tells a renderer, and what the renderer tells back.
//!
//! The driver (the only game-specific part) publishes [`Scene`] snapshots:
//! the view, the grid shape and, for every layer of the render description,
//! one value per cell (0 = nothing; other values were resolved from set names
//! via the description). The renderer draws the latest snapshot and queues
//! [`Input`] for the driver to apply to the game.
//!
//! [`Link`] connects the two sides inside one process.

pub mod wire;

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

/// The visible part of the world.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// World pixel at the top-left of the viewport.
    pub camera_x: i32,
    pub camera_y: i32,
    /// Screen pixels per world pixel.
    pub zoom: (u32, u32),
    /// Viewport size in screen pixels.
    pub viewport: (u32, u32),
    /// Draw only cells whose logical anchor lies in this world-pixel
    /// rectangle (x0, y0, x1, y1), if set (games that clip by tile range).
    pub cull: Option<(i32, i32, i32, i32)>,
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
}

/// One frame's worth of world state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    /// Increases with every published scene.
    pub seq: u64,
    /// Animation clock.
    pub tick: u32,
    pub view: View,
    pub shape: WorldShape,
    /// In the description's layer order.
    pub layers: Vec<LayerCells>,
    /// In drawing order (within the pass each follows).
    pub sprites: Vec<FreeSprite>,
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
}

#[derive(Default)]
struct Shared {
    scene: Mutex<Option<Arc<Scene>>>,
    changed: Condvar,
    input: Mutex<VecDeque<Input>>,
}

/// Both ends of a driver–renderer connection inside one process.
#[derive(Clone, Default)]
pub struct Link {
    shared: Arc<Shared>,
}

impl Link {
    pub fn new() -> Self {
        Self::default()
    }

    /// Driver side: replaces the current scene.
    pub fn publish(&self, scene: Scene) {
        *self.shared.scene.lock().unwrap() = Some(Arc::new(scene));
        self.shared.changed.notify_all();
    }

    /// Renderer side: the latest scene, if any.
    pub fn latest(&self) -> Option<Arc<Scene>> {
        self.shared.scene.lock().unwrap().clone()
    }

    /// Renderer side: waits up to `timeout` for a scene newer than `seq`.
    pub fn wait_newer(&self, seq: u64, timeout: std::time::Duration) -> Option<Arc<Scene>> {
        let guard = self.shared.scene.lock().unwrap();
        let (guard, _) = self
            .shared
            .changed
            .wait_timeout_while(guard, timeout, |s| s.as_ref().is_none_or(|s| s.seq <= seq))
            .unwrap();
        guard.clone().filter(|s| s.seq > seq)
    }

    /// Renderer side: queues input for the driver.
    pub fn send_input(&self, input: Input) {
        self.shared.input.lock().unwrap().push_back(input);
    }

    /// Driver side: takes all queued input.
    pub fn take_input(&self) -> Vec<Input> {
        self.shared.input.lock().unwrap().drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn latest_scene_and_input_cross_threads() {
        let link = Link::new();
        let driver = link.clone();
        let t = std::thread::spawn(move || {
            let mut s = Scene::empty(WorldShape { cols: 2, rows: 2, ..Default::default() }, 1);
            s.seq = 7;
            s.set(0, 1, 1, 42);
            driver.publish(s);
        });
        let got = link.wait_newer(0, Duration::from_secs(5)).expect("scene");
        t.join().unwrap();
        assert_eq!(got.seq, 7);
        assert_eq!(got.layers[0].values, vec![0, 0, 0, 42]);
        assert!(link.wait_newer(7, Duration::from_millis(10)).is_none());
        link.send_input(Input::Key { vk: 0x0D, down: true, text: None });
        assert_eq!(link.take_input().len(), 1);
        assert!(link.take_input().is_empty());
    }
}
