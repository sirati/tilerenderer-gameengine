//! Active-view confinement for a visible captured-edge pointer. Windows
//! exposes one rectangular clip; zoom-area holes retain the layout's
//! ordinary pointer-warp behavior inside this rectangle.

use winit::window::Window;

#[derive(Default)]
pub(super) struct Clip {
    system: inproc::pointer::Confinement,
}

impl Clip {
    pub fn update(&mut self, window: &Window, bounds: Option<[i32; 4]>) {
        let screen = bounds.and_then(|[x, y, w, h]| {
            let origin = window.inner_position().ok()?;
            Some([origin.x + x, origin.y + y, origin.x + x + w, origin.y + y + h])
        });
        self.system.set(screen);
    }

    pub fn release(&mut self) { self.system.release(); }
}
