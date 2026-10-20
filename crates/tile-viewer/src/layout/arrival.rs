use super::*;

impl Layout {
    /// Inspect only the mapping and cameras of an accepted presented frame.
    /// The primary following view must show the complete bounds. A smaller
    /// auxiliary view cannot acknowledge it or make that primary impossible.
    pub(crate) fn camera_arrival_receipt(&self, scene: &Scene, frame: &Frame) -> Option<Input> {
        let request = scene.camera_arrival?;
        if scene.camera_focus.is_none_or(|f| f.serial != request.serial) { return None; }
        let unavailable = || Some(Input::CameraUnavailable { serial: request.serial });
        if request.bounds[2] <= request.bounds[0] || request.bounds[3] <= request.bounds[1] { return unavailable(); }
        let required = if matches!(self.main_view.follow, Follow::Driver | Follow::Focus) { Some(Hit::Main) }
            else if self.split.is_some_and(|s| s.view.follow == Follow::Focus) { Some(Hit::Pane) }
            else { self.insets.iter().position(|v| v.view.follow == Follow::Focus).map(Hit::Inset) };
        let mut found = false;
        let mut pending = false;
        for (hit, shown) in &self.shown {
            if Some(*hit) != required { continue; }
            found = true;
            // The final camera may carry an additional target scissor.
            let Some(camera) = frame.cameras.iter().find(|c| c.cells && !c.stacked
                && c.x == shown.x && c.y == shown.y && c.zoom == shown.zoom
                && c.origin == shown.origin && c.viewport == shown.viewport) else { return unavailable(); };
            let (x, y) = camera.origin;
            let mut clip = [x.max(0), y.max(0), (x + camera.viewport.0 as i32).min(self.window.0 as i32), (y + camera.viewport.1 as i32).min(self.window.1 as i32)];
            if let Some(c) = camera.target_clip {
                clip = [clip[0].max(c[0]), clip[1].max(c[1]), clip[2].min(c[2]), clip[3].min(c[3])];
            }
            let mut area = request.bounds;
            if let Some(bounds) = self.bounds {
                for axis in 0..2 {
                    let (period, wraps) = bounds.axis(axis);
                    if !wraps || period <= 0 { continue; }
                    let middle = area[axis] + (area[axis + 2] - area[axis]) / 2;
                    let at = if axis == 0 { camera.x } else { camera.y };
                    let span = if axis == 0 { camera.viewport.0 } else { camera.viewport.1 };
                    let at = at + (span as i64 * camera.zoom.1 as i64 / camera.zoom.0 as i64 / 2) as i32;
                    let shift = at + (middle - at + period / 2).rem_euclid(period) - period / 2 - middle;
                    area[axis] += shift; area[axis + 2] += shift;
                }
            }
            let (n, d) = (camera.zoom.0 as i64, camera.zoom.1 as i64);
            if (area[2] - area[0]) as i64 * n > (clip[2] - clip[0]) as i64 * d
                || (area[3] - area[1]) as i64 * n > (clip[3] - clip[1]) as i64 * d { return unavailable(); }
            let left = (area[0] - camera.x) as i64 * n + x as i64 * d;
            let top = (area[1] - camera.y) as i64 * n + y as i64 * d;
            let right = (area[2] - camera.x) as i64 * n + x as i64 * d;
            let bottom = (area[3] - camera.y) as i64 * n + y as i64 * d;
            pending |= left < clip[0] as i64 * d || top < clip[1] as i64 * d
                || right > clip[2] as i64 * d || bottom > clip[3] as i64 * d;
        }
        if !found { unavailable() }
        else if pending { None }
        else { Some(Input::CameraArrived { serial: request.serial }) }
    }
}
