// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Automatic wiring of the RT into the STM32CubeMX-generated `main.c`.
//!
//! After the engine writes the `exhibition_rt/` tree, three lines must reach the
//! user's `main.c` (or `main.cpp`) to actually run the RT, each inside a CubeMX
//! protected "USER CODE" region so it survives the next CubeMX regeneration:
//!
//! | Line | USER CODE region |
//! |------|------------------|
//! | `#include "rt/device_registry.h"` | `Includes` |
//! | `init_rt();`                       | `2` (after the `MX_*_Init()` calls) |
//! | `tick_rt();`                        | `3` (top of the `while (1)` loop) |
//!
//! This module inserts those lines for the user. The engine does not own
//! `main.c` — CubeMX does — so every edit is confined to inside the USER CODE
//! markers, where CubeMX preserves hand-written code. The operation is
//! idempotent: a line already present (detected by a stable marker substring) is
//! left untouched, so re-running the generator never duplicates anything and a
//! fully-wired file round-trips byte-for-byte.
//!
//! [`integrate_rt`] is the pure text transform (no IO, unit-tested);
//! [`patch_main_file`] is the thin file wrapper that locates `main.c`, applies
//! the transform, and writes it back only when something changed.

use std::path::{Path, PathBuf};

use crate::engine::codegen::source_dir_rel;
use crate::engine::merge::{begin_re, end_re, line_ranges};
use crate::ioc_parser::ProjectLayout;


/// One line the generator wants present in `main.c`, together with the USER CODE
/// region it belongs in and how to tell whether it is already there.
struct Insertion {
    /// USER CODE tag (`Includes`, `2`, `3`) whose region this line goes into.
    tag: &'static str,
    /// The exact code to insert (without indentation or terminator — those are
    /// taken from the region's `BEGIN` marker line so the result matches the
    /// surrounding file).
    code: &'static str,
    /// Substring that, if found anywhere inside the region, means the line is
    /// already wired and must not be inserted again. Chosen to also match a
    /// hand-written variant (e.g. `init_rt()` with or without a trailing `;`).
    marker: &'static str,
}

/// The three lines, in the order the engine reports them. Order matches the
/// placement in [the example `main.c`](../../../example/rt/Core/Src/main.c):
/// include first in `Includes`, `init_rt()` first in `2`, `tick_rt()` first in
/// `3`.
const INSERTIONS: [Insertion; 3] = [
    Insertion {
        tag: "Includes",
        code: "#include \"rt/device_registry.h\"",
        marker: "device_registry.h",
    },
    Insertion {
        tag: "2",
        code: "init_rt();",
        marker: "init_rt(",
    },
    Insertion {
        tag: "3",
        code: "tick_rt();",
        marker: "tick_rt(",
    },
];


/// Outcome for a single line the patcher tried to wire in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStatus {
    /// The line was not present and was inserted into its USER CODE region.
    Inserted,
    /// The line (or a hand-written equivalent) was already in its region; left
    /// untouched.
    AlreadyPresent,
    /// The expected `USER CODE BEGIN <tag>` region was not found, so nothing was
    /// inserted for this line. The user must add it manually.
    RegionMissing,
}

/// What happened to each of the three integration lines, plus whether the file
/// text actually changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchReport {
    /// `#include "rt/device_registry.h"` in region `Includes`.
    pub include: LineStatus,
    /// `init_rt();` in region `2`.
    pub init: LineStatus,
    /// `tick_rt();` in region `3`.
    pub tick: LineStatus,
    /// `true` when at least one line was inserted (i.e. the returned text
    /// differs from the input).
    pub changed: bool,
}

impl PatchReport {
    /// Iterate the per-line outcomes as `(code, tag, status)` in report order,
    /// where `tag` is the USER CODE region the line targets. Used by the CLI /
    /// log to describe what was done (and, for a missing region, where the user
    /// must add the line by hand).
    pub fn lines(&self) -> [(&'static str, &'static str, LineStatus); 3] {
        [
            (INSERTIONS[0].code, INSERTIONS[0].tag, self.include),
            (INSERTIONS[1].code, INSERTIONS[1].tag, self.init),
            (INSERTIONS[2].code, INSERTIONS[2].tag, self.tick),
        ]
    }

    /// `true` when every line is wired (inserted now or already present), so the
    /// manual instructions can be dropped entirely.
    pub fn fully_wired(&self) -> bool {
        self.lines()
            .iter()
            .all(|(_, _, st)| matches!(st, LineStatus::Inserted | LineStatus::AlreadyPresent))
    }
}


/// Insert the three RT integration lines into CubeMX `main.c` text.
///
/// Each line is placed immediately after its `USER CODE BEGIN <tag>` marker,
/// inheriting that marker line's leading whitespace and line terminator so the
/// edit blends into the surrounding file and CRLF inputs round-trip exactly. A
/// line already present in its region (per [`Insertion::marker`]) is skipped; a
/// region whose `BEGIN` marker is absent yields [`LineStatus::RegionMissing`]
/// and leaves the text unchanged for that line.
///
/// Returns the (possibly unchanged) text and a [`PatchReport`]. Passing text
/// that already contains all three lines returns it byte-for-byte unchanged with
/// `changed == false`.
pub fn integrate_rt(source: &str) -> (String, PatchReport) {
    let mut status = [LineStatus::RegionMissing; 3];

    // Work line-by-line over the original byte ranges, emitting each line and,
    // when we pass a target region's BEGIN marker, splicing in the new line
    // right after it (unless the region already contains the marker substring).
    let ranges = line_ranges(source);
    let mut out = String::with_capacity(source.len() + 128);
    let begin = begin_re();
    let end = end_re();

    let mut i = 0usize;
    while i < ranges.len() {
        let (s, e) = ranges[i];
        let line = &source[s..e];
        out.push_str(line);

        if let Some(caps) = begin.captures(line) {
            let tag = caps.get(1).unwrap().as_str().trim();
            if let Some(idx) = INSERTIONS.iter().position(|ins| ins.tag == tag) {
                let ins = &INSERTIONS[idx];
                // Scan the region body (up to the matching END) to see whether
                // the line is already there.
                let mut already = false;
                let mut j = i + 1;
                while j < ranges.len() {
                    let (s2, e2) = ranges[j];
                    let line2 = &source[s2..e2];
                    if let Some(caps2) = end.captures(line2)
                        && caps2.get(1).unwrap().as_str().trim() == tag {
                            break;
                        }
                    if line2.contains(ins.marker) {
                        already = true;
                    }
                    j += 1;
                }

                if already {
                    status[idx] = LineStatus::AlreadyPresent;
                } else {
                    let (indent, terminator) = indent_and_terminator(line);
                    out.push_str(indent);
                    out.push_str(ins.code);
                    out.push_str(terminator);
                    status[idx] = LineStatus::Inserted;
                }
            }
        }

        i += 1;
    }

    let report = PatchReport {
        include: status[0],
        init: status[1],
        tick: status[2],
        changed: status.contains(&LineStatus::Inserted),
    };
    (out, report)
}


/// Split a `BEGIN` marker line into its leading whitespace and its trailing line
/// terminator (`"\r\n"`, `"\n"`, or `""` for an unterminated final line). The
/// inserted code reuses both so it lines up with the marker and preserves the
/// file's line-ending style.
fn indent_and_terminator(line: &str) -> (&str, &'static str) {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let terminator = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    (indent, terminator)
}


/// Locate the user's `main.c` (or `main.cpp`) for `project_root`/`layout`, wire
/// the three RT lines into it, and write it back if anything changed.
///
/// The file sits next to the RT source subdirectory: the parent of
/// [`source_dir_rel`] is `Core/Src` for an under-root project, or
/// `STM32CubeIDE/Application/User/Core` for the non-under-root CubeIDE layout —
/// both are exactly where CubeMX emits `main.c`. `main.c` is tried first, then
/// `main.cpp` (integrators often rename it for C++).
///
/// Returns `Ok(Some((path, report)))` for whichever file was found and
/// `Ok(None)` when neither exists (the caller then falls back to printing the
/// manual steps). Only IO failures (read/write) produce `Err`.
pub fn patch_main_file(
    project_root: &Path,
    layout: &ProjectLayout,
) -> Result<Option<(PathBuf, PatchReport)>, String> {
    let src_dir = project_root.join(source_dir_rel(layout));
    // `source_dir_rel` always returns a `<...>/rt` leaf, so a parent exists.
    let core_dir = match src_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => return Ok(None),
    };

    let main_path = ["main.c", "main.cpp"]
        .iter()
        .map(|name| core_dir.join(name))
        .find(|p| p.is_file());
    let main_path = match main_path {
        Some(p) => p,
        None => return Ok(None),
    };

    let source = std::fs::read_to_string(&main_path)
        .map_err(|e| format!("Failed to read '{}': {}", main_path.display(), e))?;
    let (patched, report) = integrate_rt(&source);
    if report.changed {
        std::fs::write(&main_path, &patched)
            .map_err(|e| format!("Failed to write '{}': {}", main_path.display(), e))?;
    }
    Ok(Some((main_path, report)))
}


#[cfg(test)]
mod tests;
