//! Compiles the umbrella shader crate `viewer-shaders` (the tile renderer's,
//! the glass lens's and the selection marker's shaders) to one SPIR-V module
//! with rust-gpu: one shader compile per build.
//!
//! `cargo-gpu-install` fetches the exact nightly toolchain rust-gpu needs and
//! builds its codegen backend once (cached per user), so this crate itself
//! builds on stable and for any target: the SPIR-V is produced on the host
//! and embedded, which is what makes cross-compiling the renderer to Windows
//! work without a Windows shader toolchain.
#![forbid(unsafe_code)]

mod build_support;
mod build_dual_source;

use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_gpu_install::install::Install;
use cargo_gpu_install::spirv_builder::{ShaderPanicStrategy, SpirvMetadata};
use cargo_gpu_install::spirv_source::{CrateMetadata, SpirvSource};

/// The crates (beside this one) the shader module is compiled from.
const SHADER_SOURCES: &[&str] = &["viewer-shaders", "tile-render-shaders", "tile-render-shared", "glass-shaders", "glass-shared"];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mode) = std::env::args_os().nth(1) {
        if mode == "--compile-source-shader" {
            let shader_crate = std::env::args_os().nth(2).ok_or("missing shader crate")?;
            let shader_target = std::env::args_os().nth(3).ok_or("missing shader target")?;
            return compile_source_shader(shader_crate.into(), shader_target.into());
        }
        return Err("unknown shader build helper mode".into());
    }
    println!("cargo::rerun-if-env-changed=VIEWER_PRECOMPILED_SPV");
    println!("cargo::rerun-if-changed=build_dual_source.rs");
    // A shader compiled on another build host is also usable on macOS:
    // SPIR-V is independent of the CPU and wgpu translates it for Metal.
    // This explicit override avoids requiring rust-gpu's compiler backend
    // on the target machine. Normal builds still compile from source.
    if let Some(path) = std::env::var_os("VIEWER_PRECOMPILED_SPV") {
        let path = PathBuf::from(path).canonicalize()?;
        println!("cargo::rerun-if-changed={}", path.display());
        return emit_shader(&path, false);
    }
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let shader_crate = manifest_dir.join("../viewer-shaders").canonicalize()?;
    // The umbrella and every crate it links (shader libraries and the
    // types they share with host code).
    for linked in SHADER_SOURCES {
        println!("cargo::rerun-if-changed={}", manifest_dir.join("..").join(linked).canonicalize()?.display());
    }

    println!("cargo::rerun-if-changed=build_support.rs");
    // The installer clears CARGO_TARGET_DIR but does not override a global
    // Cargo build.target-dir. Such a config would make its nested Cargo
    // wait on this outer build's target lock. Apply the cache override only
    // to a Rust child, without mutating process-global environment state.
    let metadata = CrateMetadata::query(shader_crate.clone())?;
    let source = SpirvSource::new(&metadata, None, None)?;
    let backend_target = source.install_dir()?.join("target");
    let shader_target = metadata.target_directory.as_std_path().join("spirv-builder");
    run_source_child(&shader_crate, &backend_target, &shader_target)
}

fn run_source_child(shader_crate: &Path, backend_target: &Path, shader_target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Command::new(std::env::current_exe()?);
    child.arg("--compile-source-shader").arg(shader_crate).arg(shader_target);
    build_support::isolate_codegen_target(&mut child, backend_target);
    let status = child.status()?;
    if !status.success() {
        return Err(format!(
            "source shader build failed ({status}); codegen cache {}, shader target {}",
            backend_target.display(),
            shader_target.display()
        )
        .into());
    }
    Ok(())
}

fn compile_source_shader(shader_crate: PathBuf, shader_target: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let backend = Install::from_shader_crate(shader_crate.clone()).within_build_script().run()?;
    let mut builder = backend
        .to_spirv_builder(shader_crate, "spirv-unknown-vulkan1.2")
        .target_dir_path(shader_target);
    builder.build_script.defaults = true;
    builder.spirv_metadata = SpirvMetadata::NameVariables;
    builder.shader_panic_strategy = ShaderPanicStrategy::SilentExit;
    let result = builder.build()?;
    let spv = result.module.unwrap_single();
    emit_shader(&spv, true)
}

fn emit_shader(path: &Path, compiler_output: bool) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let mut dual = bytes.clone();
    let mut fallback = bytes;
    for entry in ["text_dual_fs", "subpixel_dual_fs"] {
        dual = build_dual_source::decorate(&dual, entry)?;
        fallback = build_dual_source::fallback(&fallback, entry)?;
    }
    // Keep the compiler's canonical output path for source-build provenance.
    // Explicit precompiled inputs are immutable and copied to the build output.
    let output = if compiler_output { path.to_owned() } else {
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?).join("viewer_shaders.spv")
    };
    let fallback_path = output.with_extension("fallback.spv");
    std::fs::write(&output, dual)?;
    std::fs::write(&fallback_path, fallback)?;
    println!("cargo::rustc-env=VIEWER_SPV={}", output.display());
    println!("cargo::rustc-env=VIEWER_SPV_FALLBACK={}", fallback_path.display());
    Ok(())
}
