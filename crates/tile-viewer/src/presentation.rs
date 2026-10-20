//! Window presentation. The opt-in bitmap path keeps all rendering on
//! TileRenderer's GPU backend, including a software Vulkan device in a VM.

#![forbid(unsafe_code)]

use crate::ViewerError;

#[cfg(target_os = "macos")]
mod bitmap;

pub(crate) enum Presentation {
    Window(wgpu::Surface<'static>),
    #[cfg(target_os = "macos")]
    Bitmap(bitmap::Presenter),
}

pub(crate) enum Frame {
    Window(wgpu::SurfaceTexture),
    #[cfg(target_os = "macos")]
    Bitmap(wgpu::Texture),
}

impl Frame {
    pub(crate) fn texture(&self) -> &wgpu::Texture {
        match self {
            Self::Window(frame) => &frame.texture,
            #[cfg(target_os = "macos")]
            Self::Bitmap(texture) => texture,
        }
    }
}

impl Presentation {
    #[cfg(target_os = "macos")]
    pub(crate) fn bitmap(
        window: std::sync::Arc<winit::window::Window>,
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> Result<Self, ViewerError> {
        bitmap::Presenter::new(window, device, config).map(Self::Bitmap)
    }

    pub(crate) fn configure(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Result<(), ViewerError> {
        match self {
            Self::Window(surface) => {
                surface.configure(device, config);
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Self::Bitmap(presenter) => presenter.configure(device, config),
        }
    }

    pub(crate) fn acquire(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Option<Frame> {
        match self {
            Self::Window(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(Frame::Window(frame)),
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(device, config);
                    None
                }
                other => {
                    log::warn!("surface: {other:?}");
                    None
                }
            },
            #[cfg(target_os = "macos")]
            Self::Bitmap(presenter) => Some(Frame::Bitmap(presenter.texture().clone())),
        }
    }

    /// Added to the very same command encoder as the production GPU draw.
    pub(crate) fn copy_readback(&self, _encoder: &mut wgpu::CommandEncoder, _frame: &Frame) {
        #[cfg(target_os = "macos")]
        if let (Self::Bitmap(presenter), Frame::Bitmap(texture)) = (self, _frame) {
            presenter.copy_readback(_encoder, texture);
        }
    }

    pub(crate) fn present(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, frame: Frame) -> Result<(), ViewerError> {
        match (self, frame) {
            (Self::Window(_), Frame::Window(frame)) => {
                queue.present(frame);
                Ok(())
            }
            #[cfg(target_os = "macos")]
            (Self::Bitmap(presenter), Frame::Bitmap(_)) => presenter.present(_device),
            #[cfg(target_os = "macos")]
            _ => unreachable!("frame belongs to its presentation backend"),
        }
    }
}

#[cfg(any(target_os = "macos", test))]
const MAX_READBACK_BYTES: u64 = 64 * 1024 * 1024;

/// The reusable staging allocation has a fixed maximum, including GPU row
/// alignment. Reject impossible sizes before creating GPU or bitmap storage.
#[cfg(any(target_os = "macos", test))]
fn readback_layout(width: u32, height: u32) -> Option<(u32, u64)> {
    if width == 0 || height == 0 {
        return None;
    }
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let row = width.checked_mul(4)?.checked_add(alignment - 1)? / alignment * alignment;
    let bytes = u64::from(row).checked_mul(u64::from(height))?;
    (bytes <= MAX_READBACK_BYTES).then_some((row, bytes))
}

#[cfg(any(target_os = "macos", test))]
fn copy_rgba_to_bitmap(source: &[u8], row_bytes: usize, width: usize, output: &mut [u32]) {
    for (source, output) in source.chunks_exact(row_bytes).zip(output.chunks_exact_mut(width)) {
        for (rgba, pixel) in source[..width * 4].chunks_exact(4).zip(output) {
            *pixel = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_rejects_zero_overflow_and_unbounded_sizes() {
        assert_eq!(readback_layout(0, 1), None);
        assert_eq!(readback_layout(1, 0), None);
        assert_eq!(readback_layout(u32::MAX, 1), None);
        assert_eq!(readback_layout(1, u32::MAX), None);
        assert_eq!(readback_layout(8192, 8192), None);
        assert_eq!(readback_layout(4096, 4096), Some((16384, MAX_READBACK_BYTES)));
        assert_eq!(readback_layout(65, 2), Some((512, 1024)));
    }

    #[test]
    fn bitmap_conversion_keeps_channels_rows_and_ignores_alignment_padding() {
        let mut source = [0xee; 32];
        source[..8].copy_from_slice(&[1, 2, 3, 255, 4, 5, 6, 255]);
        source[16..24].copy_from_slice(&[7, 8, 9, 255, 10, 11, 12, 255]);
        let mut output = [0; 4];
        copy_rgba_to_bitmap(&source, 16, 2, &mut output);
        assert_eq!(output, [0x010203, 0x040506, 0x070809, 0x0a0b0c]);
    }
}

/// Main-thread OS bitmap presentation only. Pixels were rendered and read
/// back by the GPU actor; this type never owns a GPU device or queue.
#[cfg(target_os = "macos")]
pub(crate) struct CpuBitmap {
    surface: softbuffer::Surface<std::sync::Arc<winit::window::Window>, std::sync::Arc<winit::window::Window>>,
    _context: softbuffer::Context<std::sync::Arc<winit::window::Window>>,
}
#[cfg(target_os = "macos")]
impl CpuBitmap {
    pub fn new(window: std::sync::Arc<winit::window::Window>) -> Result<Self, ViewerError> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| ViewerError::SoftwarePresent(e.to_string()))?;
        let surface = softbuffer::Surface::new(&context, window).map_err(|e| ViewerError::SoftwarePresent(e.to_string()))?;
        Ok(Self { surface, _context: context })
    }
    pub fn present(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), ViewerError> {
        let error = |e: softbuffer::SoftBufferError| ViewerError::SoftwarePresent(e.to_string());
        let width = std::num::NonZeroU32::new(width).ok_or_else(|| ViewerError::SoftwarePresent("zero bitmap width".into()))?;
        let height = std::num::NonZeroU32::new(height).ok_or_else(|| ViewerError::SoftwarePresent("zero bitmap height".into()))?;
        self.surface.resize(width, height).map_err(error)?;
        let mut buffer = self.surface.buffer_mut().map_err(error)?;
        if buffer.len() != pixels.len() { return Err(ViewerError::SoftwarePresent("bitmap size differs from its frame".into())); }
        buffer.copy_from_slice(pixels);
        buffer.present().map_err(error)
    }
}
