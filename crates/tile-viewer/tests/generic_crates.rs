//! The generic crates stay game-agnostic (docs/generic-crates.md): none
//! depends on a game's crate or on a crate that does, and their code (not
//! their comments) names no game crate. Run alone:
//! `cargo test -p tile-viewer --test generic_crates`.

use std::path::{Path, PathBuf};

/// The game-agnostic crates (directories under `crates/`).
const GENERIC: &[&str] = &[
    "application-icon",
    "application-icon-macos",
    "function-keys",
    "function-keys-macos",
    "glass",
    "glass-shaders",
    "glass-shared",
    "gpu-text",
    "handoff",
    "marker",
    "marker-shaders",
    "menu",
    "pointer-api",
    "pointer-control",
    "pointer-fallback",
    "pointer-macos",
    "pointer-wayland",
    "pointer-windows",
    "pointer-windows-ffi",
    "pointer-x11",
    "render-desc",
    "render-errors",
    "scene",
    "tile-game",
    "tile-render",
    "tile-render-shaders",
    "tile-render-shared",
    "tile-viewer",
    "viewer-settings",
    "viewer-shaders",
    "viewer-spirv",
    "viewer-transport",
    "wl-pointer",
];

/// Words in code that name a game's crate or the injected-code crates
/// (lowercase; matched case-insensitively).
const GAME_WORDS: [&str; 3] = ["civ3", "jgl", "inproc"];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// The names of the crates `manifest` depends on, in any dependency table
/// (normal, dev, build, per target), with the `package` they rename.
fn dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            let header = line.trim_matches(|c| c == '[' || c == ']');
            in_deps = header.ends_with("dependencies") || header.contains("dependencies.");
            // `[dependencies.name]`
            if let Some((_, name)) = header.rsplit_once("dependencies.") {
                out.push(name.trim_matches('"').to_owned());
            }
            continue;
        }
        if !in_deps || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else { continue };
        let name = name.trim().trim_matches('"');
        if name.is_empty() || name.contains(' ') {
            continue;
        }
        out.push(name.to_owned());
        if let Some(at) = value.find("package") {
            if let Some(package) = value[at..].split('"').nth(1) {
                out.push(package.to_owned());
            }
        }
    }
    out
}

/// The crates of the workspace, by directory name (each `crates/*/Cargo.toml`).
fn workspace_crates() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(crates_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn generic_crates_depend_only_on_generic_crates() {
    let ours = workspace_crates();
    let mut found = Vec::new();
    for name in GENERIC {
        let path = crates_dir().join(name).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for dep in dependencies(&manifest) {
            let lower = dep.to_lowercase();
            if GAME_WORDS.iter().any(|w| lower.starts_with(w)) {
                found.push(format!("{name} depends on {dep}, a game crate"));
            } else if ours.contains(&dep) && !GENERIC.contains(&dep.as_str()) {
                found.push(format!("{name} depends on {dep}, a workspace crate that is not generic"));
            }
        }
    }
    assert!(found.is_empty(), "generic crates must stay game-agnostic (docs/generic-crates.md):\n{}", found.join("\n"));
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !matches!(path.file_name().and_then(|n| n.to_str()), Some("target" | ".git")) {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `line` without a trailing `//` comment (outside string literals, roughly:
/// an even number of quotes before it).
fn code_of(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    let mut from = 0;
    while let Some(i) = line[from..].find("//") {
        let at = from + i;
        if line[..at].matches('"').count() % 2 == 0 {
            return &line[..at];
        }
        from = at + 2;
    }
    line
}

#[test]
fn generic_code_names_no_game_crate() {
    let mut found = Vec::new();
    for name in GENERIC {
        let mut files = Vec::new();
        sources(&crates_dir().join(name), &mut files);
        for path in files {
            if path.ends_with("tests/generic_crates.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = code_of(line).to_lowercase();
                if GAME_WORDS.iter().any(|w| code.contains(w)) {
                    found.push(format!("{}:{}: {}", path.strip_prefix(crates_dir()).unwrap().display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(found.is_empty(), "generic crates' code must not name a game crate (docs/generic-crates.md):\n{}", found.join("\n"));
}

#[test]
fn the_checks_see_what_they_look_for() {
    let manifest = "[package]\nname = \"x\"\n[dependencies]\nscene = { path = \"../scene\" }\nfoo = { package = \"civ3-sys\", path = \"../civ3-sys\" }\n# civ3-ui = 1\n[target.'cfg(windows)'.dependencies]\ninproc = { path = \"../inproc\" }\n[dev-dependencies.jgl]\npath = \"../jgl\"\n";
    let deps = dependencies(manifest);
    for d in ["scene", "foo", "civ3-sys", "inproc", "jgl"] {
        assert!(deps.iter().any(|x| x == d), "{d} in {deps:?}");
    }
    assert!(!deps.iter().any(|x| x == "civ3-ui"), "comments are not dependencies");
    assert!(!deps.iter().any(|x| x == "name"), "the package table is not one");
    assert_eq!(code_of("    // a unit of civ3"), "");
    assert_eq!(code_of("let x = civ3_sys::y; // ok").trim(), "let x = civ3_sys::y;");
    assert_eq!(code_of("let url = \"http://a\"; // civ3"), "let url = \"http://a\"; ");
    for listed in GENERIC {
        assert!(workspace_crates().iter().any(|c| c == listed), "{listed} is a crate");
    }
}
