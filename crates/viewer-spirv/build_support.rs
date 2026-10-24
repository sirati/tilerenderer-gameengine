//! Environment for the rust-gpu codegen installer, scoped to a child process.

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::Command;

/// cargo-gpu-install removes CARGO_TARGET_DIR, but Cargo can still inherit
/// build.target-dir from the user's config. Override that key for this child
/// so the installer writes to the cache location it expects, not the outer
/// renderer's already locked target directory. Shader output has a separate
/// explicit SpirvBuilder target directory.
pub fn isolate_codegen_target(command: &mut Command, cache_target: &Path) {
    command.env("CARGO_BUILD_TARGET_DIR", cache_target);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn codegen_target_override_is_scoped_to_the_child_command() {
        let before = std::env::var_os("CARGO_BUILD_TARGET_DIR");
        let mut child = Command::new("cargo");
        isolate_codegen_target(&mut child, Path::new("private-codegen-target"));
        assert!(
            child
                .get_envs()
                .any(|(key, value)| key == "CARGO_BUILD_TARGET_DIR" && value == Some(std::ffi::OsStr::new("private-codegen-target")))
        );
        assert_eq!(std::env::var_os("CARGO_BUILD_TARGET_DIR"), before);
    }

    #[test]
    fn nested_codegen_build_ignores_a_global_shared_target_configuration() {
        let base = std::env::var_os("C3R_SHADER_BUILD_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = base.join(format!("c3r-shader-build-{}-{nonce}", std::process::id()));
        let cargo_home = root.join("cargo-home");
        let outer = root.join("outer");
        let backend = root.join("backend");
        let backend_target = root.join("private-codegen-target");
        for path in [&cargo_home, &outer.join("src"), &backend.join("src")] {
            fs::create_dir_all(path).unwrap();
        }
        let quoted = |p: &Path| format!("{:?}", p.to_string_lossy());
        fs::write(
            cargo_home.join("config.toml"),
            format!("[build]\ntarget-dir={}\n", quoted(&root.join("shared-renderer-target"))),
        )
        .unwrap();
        fs::write(
            outer.join("Cargo.toml"),
            "[package]\nname='outer-renderer-probe'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(outer.join("src/lib.rs"), "pub fn renderer() {}\n").unwrap();
        fs::write(
            backend.join("Cargo.toml"),
            "[package]\nname='private-codegen-probe'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(backend.join("src/lib.rs"), "pub fn backend() {}\n").unwrap();
        let helper = PathBuf::from(file!()).canonicalize().unwrap();
        let build_script = format!(
            r#"
#[path = {helper}]
mod build_support;
fn main() {{
    let mut nested = std::process::Command::new(std::env::var_os("CARGO").unwrap());
    nested.args(["build", "--offline", "--manifest-path"]).arg({manifest});
    // This is the filtering performed by cargo-gpu-install 0.11.
    nested.env_remove("CARGO_TARGET_DIR");
    build_support::isolate_codegen_target(&mut nested, std::path::Path::new({target}));
    let mut child = nested.spawn().unwrap();
    let began = std::time::Instant::now();
    loop {{
        if let Some(status) = child.try_wait().unwrap() {{ assert!(status.success()); break; }}
        if began.elapsed() > std::time::Duration::from_secs(8) {{
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("nested Cargo waited for the outer target lock");
        }}
        std::thread::sleep(std::time::Duration::from_millis(20));
    }}
}}
"#,
            helper = quoted(&helper),
            manifest = quoted(&backend.join("Cargo.toml")),
            target = quoted(&backend_target)
        );
        fs::write(outer.join("build.rs"), build_script).unwrap();
        let log_path = root.join("cargo.log");
        let log = fs::File::create(&log_path).unwrap();
        let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        command
            .args(["build", "--offline", "--manifest-path"])
            .arg(outer.join("Cargo.toml"))
            .env("CARGO_HOME", &cargo_home)
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET_DIR")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        let mut child = command.spawn().unwrap();
        let began = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if began.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("outer Cargo did not complete within 30 seconds; evidence at {}", root.display());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let log = fs::read_to_string(&log_path).unwrap();
        assert!(status.success(), "bounded nested build failed; evidence at {}\n{log}", root.display());
        assert!(
            !log.contains("Blocking waiting for file lock"),
            "nested build contended with its parent: {log}"
        );
        assert!(
            backend_target.join("debug/libprivate_codegen_probe.rlib").is_file(),
            "backend must land in its expected private cache"
        );
        assert!(root.join("shared-renderer-target/debug/libouter_renderer_probe.rlib").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
