//! The viewer's shader module: every entry point of the shader libraries
//! it links, compiled together by rust-gpu into one SPIR-V module (one
//! compile per build, see `viewer-spirv`).
#![cfg_attr(target_arch = "spirv", no_std)]

pub use tile_render_shaders;
