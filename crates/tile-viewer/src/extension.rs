//! Extension points: behaviour a crate built on the viewer plugs in
//! without the viewer knowing it (`ViewerConfig::extensions`).
//!
//! An [`InterfaceExtension`] sees every input before it goes to the
//! driver, and may take it (sending something else instead, or nothing),
//! and draws over the driver's interface each frame. It usually reads and
//! writes messages of its own extension of the scene protocol
//! (`scene::Interface::extensions`, `scene::Input::Extension`).

use std::sync::Arc;

use scene::{Input, Scene};
use tile_render::PlacedSprite;

/// A rectangle, x, y, w, h.
pub type Rect = [i32; 4];

/// Viewer behaviour of an extension: see the module.
pub trait InterfaceExtension {
    /// An input on its way to the driver (in its pixels), with the scene
    /// shown: `Some` with what goes to the driver instead if it is the
    /// extension's, `None` to let it pass.
    fn route(&mut self, input: &Input, scene: &Scene) -> Option<Vec<Input>>;

    /// Holds something across frames that follows the pointer (frames are
    /// drawn again while it does).
    fn active(&self) -> bool {
        false
    }

    /// Lets go of what it holds (the window lost the focus).
    fn cancel(&mut self) {}

    /// Draws on `scene` (its frames are then drawn every time).
    fn draws(&self, scene: &Scene) -> bool;

    /// The driver's drawing `free` (screen-space sprites among them, with
    /// `rect_of` their rectangles) with the extension's own: `None` to
    /// leave it as it is.
    fn draw(&self, scene: &Scene, free: &[PlacedSprite], rect_of: &dyn Fn(&PlacedSprite) -> Option<Rect>) -> Option<Vec<PlacedSprite>>;
}

/// Makes the extension for a new viewer window.
pub type ExtensionFactory = Arc<dyn Fn() -> Box<dyn InterfaceExtension> + Send + Sync>;

/// The extensions a viewer runs with, in order: each input goes to the
/// first that takes it; each draws over the drawing of those before it.
#[derive(Clone, Default)]
pub struct Extensions(pub Vec<ExtensionFactory>);

impl Extensions {
    pub fn with(mut self, factory: impl Fn() -> Box<dyn InterfaceExtension> + Send + Sync + 'static) -> Self {
        self.0.push(Arc::new(factory));
        self
    }

    pub(crate) fn build(&self) -> Running {
        Running(self.0.iter().map(|f| f()).collect())
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Extensions({})", self.0.len())
    }
}

/// The extensions of a running viewer.
#[derive(Default)]
pub(crate) struct Running(Vec<Box<dyn InterfaceExtension>>);

impl Running {
    pub(crate) fn active(&self) -> bool {
        self.0.iter().any(|e| e.active())
    }

    pub(crate) fn cancel(&mut self) {
        self.0.iter_mut().for_each(|e| e.cancel());
    }

    pub(crate) fn route(&mut self, input: &Input, scene: &Scene) -> Option<Vec<Input>> {
        self.0.iter_mut().find_map(|e| e.route(input, scene))
    }

    pub(crate) fn draws(&self, scene: &Scene) -> bool {
        self.0.iter().any(|e| e.draws(scene))
    }

    /// `free` with what each extension that draws on `scene` adds, `None`
    /// if none changed it.
    pub(crate) fn draw(&self, scene: &Scene, free: &[PlacedSprite], rect_of: &dyn Fn(&PlacedSprite) -> Option<Rect>) -> Option<Vec<PlacedSprite>> {
        let mut out: Option<Vec<PlacedSprite>> = None;
        for e in self.0.iter().filter(|e| e.draws(scene)) {
            if let Some(drawn) = e.draw(scene, out.as_deref().unwrap_or(free), rect_of) {
                out = Some(drawn);
            }
        }
        out
    }
}
