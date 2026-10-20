//! CoreGraphics presentation for macOS VMs without a Metal display device.
//! One offscreen target and one staging buffer are retained across frames.

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use winit::window::Window;

use super::{copy_rgba_to_bitmap, readback_layout};
use crate::ViewerError;

pub(crate) struct Presenter {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    // Keep the display alive until its surface has been dropped.
    _context: softbuffer::Context<Arc<Window>>,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    row_bytes: u32,
    size: (u32, u32),
}

fn error(error: impl std::fmt::Display) -> ViewerError {
    ViewerError::SoftwarePresent(error.to_string())
}

impl Presenter {
    pub(super) fn new(window: Arc<Window>, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Result<Self, ViewerError> {
        let context = softbuffer::Context::new(window.clone()).map_err(error)?;
        let mut surface = softbuffer::Surface::new(&context, window).map_err(error)?;
        let (target, readback, row_bytes) = Self::storage(device, config)?;
        surface
            .resize(NonZeroU32::new(config.width).unwrap(), NonZeroU32::new(config.height).unwrap())
            .map_err(error)?;
        Ok(Self {
            surface,
            _context: context,
            target,
            readback,
            row_bytes,
            size: (config.width, config.height),
        })
    }

    fn storage(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Result<(wgpu::Texture, wgpu::Buffer, u32), ViewerError> {
        let (row_bytes, bytes) = readback_layout(config.width, config.height)
            .ok_or_else(|| error(format!("{}x{} bitmap exceeds the 64 MiB readback limit", config.width, config.height)))?;
        if config.width > device.limits().max_texture_dimension_2d || config.height > device.limits().max_texture_dimension_2d {
            return Err(error("bitmap exceeds the GPU texture dimension limit"));
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewer software-present target"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer software-present readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok((target, readback, row_bytes))
    }

    pub(super) fn configure(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Result<(), ViewerError> {
        if self.size == (config.width, config.height) {
            return Ok(());
        }
        let (target, readback, row_bytes) = Self::storage(device, config)?;
        self.surface
            .resize(NonZeroU32::new(config.width).unwrap(), NonZeroU32::new(config.height).unwrap())
            .map_err(error)?;
        self.target = target;
        self.readback = readback;
        self.row_bytes = row_bytes;
        self.size = (config.width, config.height);
        Ok(())
    }

    pub(super) fn texture(&self) -> &wgpu::Texture {
        &self.target
    }

    pub(super) fn copy_readback(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row_bytes),
                    rows_per_image: Some(self.size.1),
                },
            },
            texture.size(),
        );
    }

    /// Rendering remains on the selected wgpu device. Only its completed
    /// RGBA output is copied to AppKit. The wait and allocations are bounded.
    pub(super) fn present(&mut self, device: &wgpu::Device) -> Result<(), ViewerError> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        if let Err(e) = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        }) {
            self.readback.unmap();
            return Err(error(format!("GPU readback did not complete: {e}")));
        }
        let mapped = match rx.try_recv() {
            Ok(result) => result.map_err(error),
            Err(e) => Err(error(format!("GPU readback callback did not complete: {e}"))),
        };
        if let Err(e) = mapped {
            self.readback.unmap();
            return Err(e);
        }
        let result = (|| {
            let source = self.readback.slice(..).get_mapped_range().map_err(error)?;
            let mut output = self.surface.buffer_mut().map_err(error)?;
            copy_rgba_to_bitmap(&source, self.row_bytes as usize, self.size.0 as usize, &mut output);
            output.present().map_err(error)
        })();
        self.readback.unmap();
        result
    }
}
