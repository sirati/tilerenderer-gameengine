//! The viewer's shader module: `viewer-shaders`, which links the tile
//! renderer's, the glass lens's and the selection marker's shader
//! libraries, compiled to one SPIR-V module by this crate's build script.
//!
//! It holds bytes only (no renderer types), so that every crate drawing
//! with it, and the tests of the crates whose shaders it holds, can depend
//! on it without a cycle. `VIEWER_PRECOMPILED_SPV` (an absolute path to a
//! module built from the same sources elsewhere) replaces the compile, e.g.
//! on a macOS host without rust-gpu's toolchain.
#![forbid(unsafe_code)]

/// The module, with the dual-source text and ClearType-like entry points
/// (for a device with dual-source blending).
pub const SPIRV: &[u8] = include_bytes!(env!("VIEWER_SPV"));

/// The same module without dual-source outputs (for any other device).
pub const SPIRV_FALLBACK: &[u8] = include_bytes!(env!("VIEWER_SPV_FALLBACK"));
