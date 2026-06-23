// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Regression guard: every project-owned source file must begin with the SPDX
//! license banner.
//!
//! Only project-owned source is checked. Third-party / vendor code (STM32 HAL,
//! CMSIS, ST USB middleware) and STM32CubeIDE-generated `Core/` files carry
//! their own upstream licenses and are intentionally excluded. Rather than
//! maintain a vendor denylist, this test uses an allowlist of owned roots — see
//! [`OWNED_ROOTS`].

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");

use std::path::{Path, PathBuf};

/// Owned source trees, each paired with the file extensions to check within it.
///
/// Paths are relative to the crate root (`CARGO_MANIFEST_DIR`, the repo root).
/// The `src` root is restricted to `.rs` so the embedded C fixture under
/// `src/tests/discovery_f4/` (which has no `.rs` files) is skipped; the owned
/// C++ template tree under `src/engine/autogen_content/cpp/` is listed
/// separately. For `example/rt`, only the exhibition-generated `rt/` subtrees
/// are owned — the rest is CubeMX/vendor.
const OWNED_ROOTS: &[(&str, &[&str])] = &[
    ("src", &["rs"]),
    ("src/engine/autogen_content/cpp", &["cpp", "h"]),
    ("example/exhibition_bc", &["cpp", "h"]),
    ("example/rt/Core/Src/rt", &["cpp", "h"]),
    ("example/rt/Core/Inc/rt", &["cpp", "h"]),
];

/// Directory names that hold bundled third-party code under an owned root (e.g.
/// the amalgamated Catch2 test framework). These carry their own upstream
/// licenses and are skipped.
const VENDORED_DIRS: &[&str] = &["thirdparty"];

/// Recursively collect files under `dir` whose extension is in `exts`, skipping
/// any [`VENDORED_DIRS`] subtree.
fn collect(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // A missing optional root (e.g. example/ trimmed from a checkout) should
        // not fail the test; a present-but-unreadable dir would surface here.
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if VENDORED_DIRS.contains(&name) {
                continue;
            }
            collect(&path, exts, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if exts.contains(&ext) {
                out.push(path);
            }
        }
    }
}

#[test]
fn owned_sources_have_spdx_header() {
    // The expected banner is the first two lines of the canonical C++ header
    // constant (Rust and C++ share identical `//` comment syntax), so the test
    // and the generator can never diverge. `trim_end` drops the trailing blank
    // line the constant carries for separation from include guards.
    let expected = crate::engine::SPDX_CPP_HEADER.trim_end();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut files = Vec::new();
    for (rel, exts) in OWNED_ROOTS {
        collect(&root.join(rel), exts, &mut files);
    }
    assert!(
        !files.is_empty(),
        "no owned source files found — allowlist or layout is wrong"
    );

    let mut missing = Vec::new();
    for path in &files {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        if !content.starts_with(expected) {
            missing.push(path.strip_prefix(root).unwrap_or(path).display().to_string());
        }
    }

    assert!(
        missing.is_empty(),
        "the following project-owned files are missing the SPDX license header:\n  {}\n\n\
         Prepend this banner (followed by a blank line) to each:\n{}",
        missing.join("\n  "),
        expected,
    );
}
