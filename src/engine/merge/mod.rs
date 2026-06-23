// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! STM32CubeMX-style preservation of hand-written code across regeneration.
//!
//! Generated source files may carry protected regions delimited by marker
//! comments in the CubeMX convention:
//!
//! ```c
//! /* USER CODE BEGIN Init */
//! init_rt();                  // <- survives regeneration
//! /* USER CODE END Init */
//! ```
//!
//! When the generator rewrites such a file, [`preserve_user_code`] splices the
//! text the user wrote between each `BEGIN`/`END` pair in the *existing*
//! on-disk file back into the freshly *generated* text, keyed by the tag that
//! follows `BEGIN` (`Init` above). Everything outside a recognized region is
//! taken verbatim from the generated text, so low-level scaffolding always
//! refreshes while the user's edits stay put.
//!
//! A region whose tag is present in the existing file but absent from the new
//! template (for example, because a future generator dropped that region) is
//! never discarded silently: its body is appended to an `ORPHANED USER CODE`
//! trailer at the end of the file so the user can recover or relocate it.
//!
//! The merge is line based but byte exact: terminators (`\n` / `\r\n`) and the
//! exact bytes of each preserved region are round-tripped unchanged, which is
//! what lets a regeneration with no user edits reproduce the generated text
//! byte-for-byte (the generator's idempotency guarantee).

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;


/// Matches a `USER CODE BEGIN <tag>` marker line.
///
/// Tolerates leading whitespace (regions inside a function body are indented)
/// and the surrounding `/* … */` block-comment framing. Capture group 1 is the
/// tag. The trailing `\s*\*/\s*$` anchor requires the comment to close at the
/// end of the line, so a marker embedded mid-line inside a string literal such
/// as `puts("/* USER CODE BEGIN X */");` is intentionally *not* matched.
pub(crate) fn begin_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*/\*\s*USER CODE BEGIN\s+(\S.*?)\s*\*/\s*$").unwrap())
}

/// Matches a `USER CODE END <tag>` marker line; see [`begin_re`].
pub(crate) fn end_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*/\*\s*USER CODE END\s+(\S.*?)\s*\*/\s*$").unwrap())
}


/// Byte ranges of each line of `s`, *including* the line's trailing terminator
/// (`\n` or `\r\n`). A final line without a terminator is included as-is.
/// Concatenating `s[start..end]` over the returned ranges reproduces `s`.
pub(crate) fn line_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            ranges.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        ranges.push((start, bytes.len()));
    }
    ranges
}


/// Extract every protected region from `existing`, in the order encountered.
///
/// Each entry is `(tag, inner)` where `inner` is the verbatim bytes between the
/// `BEGIN` line's terminator and the start of the `END` line (so a region's own
/// trailing newline is part of `inner` and round-trips). Duplicate tags keep
/// the first occurrence; nested `BEGIN`s are treated as literal body text; an
/// unterminated `BEGIN` is ignored. Each anomaly is reported on stderr.
fn extract_user_blocks(existing: &str) -> Vec<(String, String)> {
    let begin = begin_re();
    let end = end_re();
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // (tag, byte offset where the region body begins)
    let mut open: Option<(String, usize)> = None;

    for (s, e) in line_ranges(existing) {
        let line = &existing[s..e];
        if let Some((tag, inner_start)) = open.as_ref() {
            // While a region is open, only its matching END closes it; any
            // intervening BEGIN/END for other tags is just body text.
            if let Some(caps) = end.captures(line)
                && caps.get(1).unwrap().as_str().trim() == tag {
                    let inner = existing[*inner_start..s].to_string();
                    if seen.insert(tag.clone()) {
                        blocks.push((tag.clone(), inner));
                    } else {
                        eprintln!(
                            "warning: duplicate USER CODE region '{tag}'; keeping the first copy"
                        );
                    }
                    open = None;
                }
        } else if let Some(caps) = begin.captures(line) {
            let tag = caps.get(1).unwrap().as_str().trim().to_string();
            open = Some((tag, e));
        }
    }

    if let Some((tag, _)) = open {
        eprintln!("warning: unterminated USER CODE region '{tag}'; ignoring it");
    }
    blocks
}


/// Splice the protected regions of `existing` into `generated`.
///
/// For every `USER CODE BEGIN/END` pair in `generated`, if a region with the
/// same tag exists in `existing` its body replaces the generated default while
/// the generated marker lines (framing, indentation) are kept. Text outside any
/// region is copied from `generated` verbatim. Regions found only in `existing`
/// (orphans) are appended in an `ORPHANED USER CODE` trailer rather than lost.
///
/// Passing identical `generated` and `existing` returns `generated` unchanged,
/// so regenerating an unedited project is a byte-for-byte no-op.
pub fn preserve_user_code(generated: &str, existing: &str) -> String {
    let begin = begin_re();
    let end = end_re();
    let preserved = extract_user_blocks(existing);

    // tag -> index into `preserved`; tags are unique (extract is first-wins).
    let index: HashMap<&str, usize> = preserved
        .iter()
        .enumerate()
        .map(|(i, (tag, _))| (tag.as_str(), i))
        .collect();
    let mut consumed = vec![false; preserved.len()];

    let ranges = line_ranges(generated);
    let mut out = String::with_capacity(generated.len());

    let mut i = 0usize;
    while i < ranges.len() {
        let (s, e) = ranges[i];
        let line = &generated[s..e];

        if let Some(caps) = begin.captures(line) {
            let tag = caps.get(1).unwrap().as_str().trim();
            if let Some(&idx) = index.get(tag) {
                // Emit the generated BEGIN line, swap in the preserved body,
                // then skip the generated body up to the matching END line.
                out.push_str(line);
                out.push_str(&preserved[idx].1);
                consumed[idx] = true;

                let mut j = i + 1;
                while j < ranges.len() {
                    let (s2, e2) = ranges[j];
                    let line2 = &generated[s2..e2];
                    if let Some(caps2) = end.captures(line2)
                        && caps2.get(1).unwrap().as_str().trim() == tag {
                            out.push_str(line2);
                            j += 1;
                            break;
                        }
                    j += 1;
                }
                i = j;
                continue;
            }
        }

        out.push_str(line);
        i += 1;
    }

    append_orphans(&mut out, &preserved, &consumed);
    out
}


/// Append any preserved region that the generated template no longer contains
/// to a labeled trailer, in discovery order. Rebuilt from scratch every call
/// (never copied from the prior trailer), so re-merging an already-trailered
/// file reproduces the same bytes.
fn append_orphans(out: &mut String, preserved: &[(String, String)], consumed: &[bool]) {
    let orphans: Vec<&(String, String)> = preserved
        .iter()
        .enumerate()
        .filter(|(i, _)| !consumed[*i])
        .map(|(_, block)| block)
        .collect();
    if orphans.is_empty() {
        return;
    }

    let tags: Vec<&str> = orphans.iter().map(|(tag, _)| tag.as_str()).collect();
    eprintln!(
        "warning: {} USER CODE region(s) no longer in the template, moved to the ORPHANED trailer: {}",
        orphans.len(),
        tags.join(", ")
    );

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("/* ===== ORPHANED USER CODE (no longer generated) ===== */\n");
    for (tag, inner) in orphans {
        out.push_str(&format!("/* USER CODE BEGIN {tag} */\n"));
        out.push_str(inner);
        out.push_str(&format!("/* USER CODE END {tag} */\n"));
    }
}


/// Parse the opcode byte from a trimmed manifest command line of the form
/// `- opcode: 0x05`. Returns `None` for any other line (including payload field
/// entries such as `- uid0: uint32_t`, which share the `- ` list prefix but not
/// the `opcode:` key).
fn parse_opcode_line(trimmed: &str) -> Option<u8> {
    let rest = trimmed.strip_prefix("- opcode:")?.trim();
    let hex = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X"))?;
    u8::from_str_radix(hex, 16).ok()
}


/// Collect each command's filled-in `description` from an existing manifest
/// file, keyed by opcode. The per-device (top-level) `description:` precedes any
/// command, so it is skipped (no current opcode). Placeholder (`TODO`) and empty
/// descriptions are not collected, so they never override a regenerated `TODO`.
fn extract_command_descriptions(existing: &str) -> HashMap<u8, String> {
    let mut map: HashMap<u8, String> = HashMap::new();
    let mut current: Option<u8> = None;
    for (s, e) in line_ranges(existing) {
        let trimmed = existing[s..e].trim();
        if let Some(op) = parse_opcode_line(trimmed) {
            current = Some(op);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("description:")
            && let Some(op) = current {
                let val = rest.trim();
                if !val.is_empty() && val != "TODO" {
                    map.entry(op).or_insert_with(|| val.to_string());
                }
            }
    }
    map
}


/// Splice per-command `description:` values from an existing manifest file into
/// a freshly generated one.
///
/// The generator emits every command's `description:` as the placeholder
/// `TODO` (see [`crate::engine::manifest`]). This walks `generated` and, for
/// each command keyed by its `opcode`, replaces that placeholder with the
/// description the user had written into the same command in the on-disk
/// `existing` file — so filled-in documentation survives regeneration even
/// though the manifest is otherwise rewritten wholesale.
///
/// A command whose existing description is still `TODO` (or absent — a brand-new
/// command, or an older file with no `description:` lines at all) keeps the
/// generated `TODO`. Passing identical `generated` and `existing` returns
/// `generated` unchanged, so an unedited regeneration is a byte-for-byte no-op.
///
/// Descriptions are assumed single-line, matching what the generator writes and
/// what the fingerprint canonicalizer expects; the swapped-in value reuses the
/// generated line's indentation, so only the text after `description:` changes.
pub fn preserve_command_descriptions(generated: &str, existing: &str) -> String {
    let saved = extract_command_descriptions(existing);
    if saved.is_empty() {
        return generated.to_string();
    }

    let mut out = String::with_capacity(generated.len());
    let mut current: Option<u8> = None;
    for (s, e) in line_ranges(generated) {
        let line = &generated[s..e];
        let trimmed = line.trim();

        if let Some(op) = parse_opcode_line(trimmed) {
            current = Some(op);
            out.push_str(line);
            continue;
        }

        if trimmed.starts_with("description:")
            && let Some(desc) = current.and_then(|op| saved.get(&op)) {
                let indent_len = line.len() - line.trim_start().len();
                out.push_str(&line[..indent_len]);
                out.push_str("description: ");
                out.push_str(desc);
                out.push('\n');
                continue;
            }

        out.push_str(line);
    }
    out
}


#[cfg(test)]
mod tests;
