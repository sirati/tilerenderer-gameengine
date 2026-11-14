//! Compile this package's shader module using the shared host build helper.
#![forbid(unsafe_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    shader_build::build(shader_build::Config {
        shader_crate: std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../viewer-shaders"),
        source_packages: &["viewer-shaders", "tile-render-shaders", "tile-render-shared", "marker-shaders"],
        precompiled_envs: &["VIEWER_PRECOMPILED_SPV"],
        spirv_env: "VIEWER_SPV",
        fallback_env: "VIEWER_SPV_FALLBACK",
        output_name: "viewer_shaders.spv",
        dual_source_entries: &["text_dual_fs", "subpixel_dual_fs"],
    })
}
