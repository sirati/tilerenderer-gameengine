//! Geometry of a rounded view frame, used for layout and pointer hit bounds.
//! The application supplies the renderer effect which decorates this frame.

/// Frame geometry in composition pixels. `raised` requests the application's
/// raised frame style; it does not select or implement a particular shader.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundedFrame {
    pub radius: f32,
    pub rim: f32,
    pub raised: bool,
}

impl RoundedFrame {
    /// Signed distance to the ordinary rounded-rectangle hit boundary.
    pub fn distance(self, size: (u32, u32), p: (f32, f32)) -> f32 {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let radius = self.radius.clamp(0.0, w.min(h) * 0.5);
        let (x, y) = ((p.0 - w * 0.5).abs() - w * 0.5 + radius,
                      (p.1 - h * 0.5).abs() - h * 0.5 + radius);
        (x.max(0.0) * x.max(0.0) + y.max(0.0) * y.max(0.0)).sqrt() + x.max(y).min(0.0) - radius
    }

    pub fn of(effect: tile_render::Effect) -> Self {
        Self { radius: effect.lengths[0], rim: effect.lengths[1], raised: effect.variant != 0 }
    }
}

impl From<RoundedFrame> for tile_render::Effect {
    fn from(frame: RoundedFrame) -> Self {
        Self { lengths: [frame.radius, frame.rim], variant: u32::from(frame.raised) }
    }
}
