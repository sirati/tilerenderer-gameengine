//! The viewer's shader module: every entry point of the shader libraries
//! it links (the tile renderer's, the glass lens's), compiled together by
//! rust-gpu into one SPIR-V module (one compile per build, see
//! `viewer-spirv`), and the one entry point wiring two of them together:
//! [`painted_fs`], the tile renderer's painted shapes painted by the
//! selection marker.
#![cfg_attr(target_arch = "spirv", no_std)]

pub use glass_shaders;
pub use marker_shaders;
pub use tile_render_shaders;

use spirv_std::glam::{IVec2, UVec2, UVec3, UVec4, Vec2, Vec4};
use spirv_std::spirv;
use tile_render_shared::FrameParams;

/// The tile renderer's painter (`tile_render::Shaders::painter`): its
/// painted shapes are the selection marker's arrow and shadow. Its inputs
/// are `tile_fs`' (from `free_vs`).
#[allow(clippy::too_many_arguments)]
#[spirv(fragment)]
pub fn painted_fs(
    local: Vec2,
    #[spirv(flat)] _origin: IVec2,
    #[spirv(flat)] page: u32,
    #[spirv(flat)] _scale_den: u32,
    #[spirv(flat)] _tint: u32,
    #[spirv(flat)] _base: IVec2,
    #[spirv(flat)] _lo: IVec2,
    #[spirv(flat)] _scaled_view: u32,
    #[spirv(flat)] _step: u32,
    #[spirv(flat)] _palette: UVec3,
    #[spirv(flat)] background: UVec4,
    #[spirv(flat)] primitive: UVec2,
    #[spirv(uniform, descriptor_set = 0, binding = 0)] params: &FrameParams,
    output: &mut Vec4,
) {
    tile_render_shaders::painted(local, page, background, primitive, params, output, marker_shaders::paint);
}
