//! An application's image, applied to its window and native application tile.
//! Platform calls stay in the selected backend; callers supply ordinary PNG data.

#![forbid(unsafe_code)]

use std::sync::Arc;
use winit::window::{Icon, Window};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("application icon PNG: {0}")]
    Png(#[from] image::ImageError),
    #[error("application icon: {0}")]
    Window(#[from] winit::window::BadIcon),
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Native(#[from] application_icon_macos::Error),
}

/// A decoded icon with its original PNG retained for native application APIs.
#[derive(Clone, Debug)]
pub struct ApplicationIcon {
    window: Icon,
    png: Arc<[u8]>,
    size: (u32, u32),
}

impl ApplicationIcon {
    pub fn from_png(png: &[u8]) -> Result<Self, Error> {
        let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
        let size = rgba.dimensions();
        let window = Icon::from_rgba(rgba.into_raw(), size.0, size.1)?;
        Ok(Self {
            window,
            png: png.into(),
            size,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn png(&self) -> &[u8] {
        &self.png
    }

    /// Call from the window event thread. macOS updates the process-wide Dock
    /// icon because AppKit has no per-window icon. Other backends use the
    /// window icon supplied to their window manager.
    pub fn apply(&self, window: &Window) -> Result<(), Error> {
        #[cfg(target_os = "macos")]
        application_icon_macos::set_png(&self.png)?;
        window.set_window_icon(Some(self.window.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two RGBA pixels: opaque red, then transparent green. This fixture is
    // entirely local to the common crate and carries no application artwork.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0xf4,
        0x22, 0x7f, 0x8a, 0x00, 0x00, 0x00, 0x0f, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x08, 0x19, 0x00, 0x0f, 0xf9, 0x02, 0xfe, 0x94, 0x3c, 0x3d, 0xb3,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn decodes_rgba_dimensions_and_retains_the_original_png() {
        let icon = ApplicationIcon::from_png(PNG).unwrap();
        assert_eq!(icon.size(), (2, 1));
        assert_eq!(icon.png(), PNG);
        let rgba = image::load_from_memory_with_format(icon.png(), image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(rgba.get_pixel(1, 0).0, [0, 255, 0, 0]);
    }

    #[test]
    fn rejects_invalid_or_truncated_pngs_before_calling_a_platform_api() {
        assert!(ApplicationIcon::from_png(b"not a PNG").is_err());
        assert!(ApplicationIcon::from_png(&PNG[..PNG.len() / 2]).is_err());
    }
}
