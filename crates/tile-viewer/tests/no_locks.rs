//! The renderer's own code shares no locks between threads (see
//! `docs/concurrency.md`): this fails if a lock type reappears in a crate's
//! Rust sources or a lock crate in a manifest, outside `locks-allowlist.txt`.
//! Run alone: `cargo test -p handoff --test no_locks`.

use std::path::{Path, PathBuf};

/// Names that mean a lock (split so this file does not list itself).
const FORBIDDEN: [&str; 4] = [concat!("Mu", "tex"), concat!("Rw", "Lock"), concat!("Cond", "var"), concat!("parking", "_lot")];

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // (Build output.)
            if !matches!(path.file_name().and_then(|n| n.to_str()), Some("target" | ".git")) {
                files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") || path.file_name().is_some_and(|n| n == "Cargo.toml") {
            out.push(path);
        }
    }
}

fn word_at(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        // (A prefix: also `MutexGuard`, `RwLockReadGuard`.)
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn no_locks_outside_the_allowlist() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let root = crates.parent().unwrap();
    let allow = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("locks-allowlist.txt")).unwrap();
    // Lines: `path/relative/to/repo[:line text substring] # justification`.
    let allowed: Vec<(&str, Option<&str>)> = allow
        .lines()
        .map(|l| l.split('#').next().unwrap().trim())
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once(':') {
            Some((path, text)) => (path.trim(), Some(text.trim())),
            None => (l, None),
        })
        .collect();
    let mut found = Vec::new();
    let mut all = Vec::new();
    files(&crates, &mut all);
    for path in all {
        let relative = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if relative.ends_with("/tests/no_locks.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (n, line) in text.lines().enumerate() {
            if FORBIDDEN.iter().any(|w| word_at(line, w))
                && !allowed.iter().any(|(p, t)| *p == relative && t.is_none_or(|t| line.contains(t)))
            {
                found.push(format!("{relative}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "locks are not used in this code base: use channels or `handoff` (docs/concurrency.md), or add a justified entry to crates/handoff/locks-allowlist.txt:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_check_sees_a_lock() {
    let lock = format!("static X: std::sync::{}<u32>", FORBIDDEN[0]);
    assert!(word_at(&lock, FORBIDDEN[0]));
    assert!(word_at(&format!("std::sync::{}Guard", FORBIDDEN[0]), FORBIDDEN[0]));
    assert!(!word_at(&format!("Async{}", FORBIDDEN[0]), FORBIDDEN[0]));
}
