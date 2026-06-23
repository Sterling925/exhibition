// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! YAML manifest and 32-bit fingerprint for the generated API.
//!
//! The manifest is a *set* of files, one per sub-address, written into an
//! `api/` directory (e.g. `01_system.yaml`, `02_<instance>.yaml`, …). Each
//! file documents one sub-address: its number, name, description, type, a
//! per-service hash, the per-instance details, and the command vocabulary
//! (opcode + name + wire payload shapes). The command vocabulary and the
//! per-service hash are both sourced from the embedded command catalog
//! ([`crate::engine::commands`]).
//!
//! The (global) fingerprint is an FNV-1a hash over the canonical form of the
//! whole set (file names plus content, whitespace/comments/the `fingerprint:`
//! and `service_hash:` lines stripped). It lets the BC verify it was built
//! against the same API revision as the RT before issuing commands. The value
//! is stamped into the system file (`01_system.yaml`) and into the generated
//! `api_fingerprint.h`.
//!
//! Each file also carries a `service_hash:` line — an FNV-1a hash over just
//! that sub-address's command vocabulary
//! ([`crate::engine::commands::service_hash_of_set`]). It is project-independent
//! (every sub-address of the same peripheral kind shares it) and is read at
//! runtime via the universal `GET_SERVICE_HASH` command (opcode `0x00`) so the
//! BC can localize which service diverged when the global fingerprint mismatches.

use crate::engine::addresses::{DeviceKind, DeviceSlot, SYSTEM_SUB_ADDRESS};
use crate::engine::commands::{self, CommandDef, PayloadDef};


/// One generated per-sub-address manifest file.
pub struct ManifestFile {
    /// File name within the `api/` directory, e.g. `02_adc_hadc1_ch0.yaml`.
    /// The leading `NN_` is the zero-padded sub-address.
    pub file_name: String,
    /// Full YAML body.
    pub content: String,
}


/// Append the YAML lines for one command list under a top-level `commands:`
/// key (two-space list indent).
///
/// Each command's `description` defaults to the build-time text from the
/// command catalog ([`CommandDef::description`]), or the placeholder `TODO`
/// when the catalog has none. Either way the real text (when the user has
/// filled one in, or edited the default) is spliced back from the existing
/// on-disk file at write time by
/// [`crate::engine::merge::preserve_command_descriptions`]. Descriptions are
/// excluded from the fingerprint (see [`canonicalize`]).
pub(crate) fn push_commands(out: &mut String, cmds: &[CommandDef]) {
    for c in cmds {
        out.push_str(&format!("  - opcode: 0x{:02X}\n", c.opcode));
        out.push_str(&format!("    name: {}\n", c.name));
        let desc = c.description.as_deref().unwrap_or("TODO");
        out.push_str(&format!("    description: {}\n", desc));
        push_payload(out, "cmd_payload", &c.cmd_payload);
        push_payload(out, "resp_payload", &c.resp_payload);
    }
}


/// Append one payload (`cmd_payload` / `resp_payload`) as a nested mapping with
/// a `bytes` size expression and, when present, an ordered `fields` list of
/// `name: type` entries:
///
/// ```yaml
///     resp_payload:
///       bytes: 12
///       fields:
///         - uid0: uint32_t
/// ```
///
/// The `fields:` line is omitted entirely when the payload has no sub-fields.
fn push_payload(out: &mut String, key: &str, payload: &PayloadDef) {
    out.push_str(&format!("    {}:\n", key));
    out.push_str(&format!("      bytes: {}\n", payload.bytes));
    if !payload.fields.is_empty() {
        out.push_str("      fields:\n");
        for f in &payload.fields {
            out.push_str(&format!("        - {}: {}\n", f.name, f.ty));
        }
    }
}


/// `NN_<name>.yaml` file name for a sub-address (zero-padded to two digits so
/// the directory listing sorts by sub-address).
fn file_name_for(sub_address: u8, name: &str) -> String {
    format!("{:02}_{}.yaml", sub_address, name)
}


/// Build the per-sub-address manifest files for an RT containing `slots`
/// peripheral devices plus the system handler on sub-address 1.
///
/// The system file carries a placeholder `fingerprint: 0x00000000` line; call
/// [`fingerprint_of_canonical`] on [`canonical_set`] of the result to compute
/// the real fingerprint, then [`stamp_fingerprint_in_set`] to substitute it.
/// Files are returned in sub-address order (system first).
pub fn build_manifest_files(slots: &[DeviceSlot]) -> Vec<ManifestFile> {
    let mut files = Vec::with_capacity(slots.len() + 1);
    files.push(render_system_file());
    for slot in slots {
        files.push(render_slot_file(slot));
    }
    files
}


/// The system handler file (sub-address 1). Carries the manifest `version` and
/// the fingerprint placeholder, since the system handler is what serves the
/// runtime fingerprint.
fn render_system_file() -> ManifestFile {
    let mut content = String::new();
    content.push_str(crate::engine::SPDX_YAML_HEADER);
    content.push_str("version: 1\n");
    content.push_str("fingerprint: 0x00000000\n");
    content.push_str(&format!("sub_address: {}\n", SYSTEM_SUB_ADDRESS));
    content.push_str("name: system\n");
    content.push_str("description: System handler (API fingerprint, device count)\n");
    content.push_str("type: system\n");
    content.push_str(&format!(
        "service_hash: 0x{:08X}\n",
        commands::service_hash_of_set(commands::system())
    ));
    content.push_str("commands:\n");
    push_commands(&mut content, &commands::system().commands);
    ManifestFile {
        file_name: file_name_for(SYSTEM_SUB_ADDRESS, "system"),
        content,
    }
}


/// One peripheral slot's file. `description` reuses the slot's human-readable
/// label.
fn render_slot_file(slot: &DeviceSlot) -> ManifestFile {
    let mut content = String::new();
    content.push_str(crate::engine::SPDX_YAML_HEADER);
    content.push_str(&format!("sub_address: {}\n", slot.sub_address));
    content.push_str(&format!("name: {}\n", slot.instance_name));
    content.push_str(&format!("description: {}\n", slot.label));
    content.push_str(&format!("type: {}\n", slot.kind.manifest_name()));
    content.push_str(&format!(
        "service_hash: 0x{:08X}\n",
        commands::service_hash_of_set(commands::for_kind(slot.kind))
    ));
    for (k, v) in &slot.details {
        content.push_str(&format!("{}: {}\n", k, v));
    }
    if matches!(slot.kind, DeviceKind::GpioOutput | DeviceKind::GpioInput) {
        content.push_str("pins:\n");
        for m in &slot.members {
            if let Some(pin_idx) = m.pin_index {
                content.push_str(&format!(
                    "  - {{ name: {}, label: {}, pin_idx: {} }}\n",
                    m.instance_name, m.label, pin_idx
                ));
            }
        }
    }
    content.push_str("commands:\n");
    push_commands(&mut content, &commands::for_kind(slot.kind).commands);
    ManifestFile {
        file_name: file_name_for(slot.sub_address, &slot.instance_name),
        content,
    }
}


/// Canonical form of the whole manifest set, in file order: each file's name
/// followed by its [`canonicalize`]d content. The file name is included so a
/// sub-address reassignment (which renames files) perturbs the fingerprint.
pub fn canonical_set(files: &[ManifestFile]) -> String {
    let mut out = String::new();
    for f in files {
        out.push_str(&f.file_name);
        out.push('\n');
        out.push_str(&canonicalize(&f.content));
    }
    out
}


/// Strip whitespace-only lines, comments, the placeholder `fingerprint:` line,
/// every `service_hash:` line, and every `description:` line so the hash is
/// stable across cosmetic edits and documentation changes.
///
/// The `fingerprint:` line is dropped entirely (not just blanked) so that the
/// eventual real fingerprint value cannot perturb the hash of itself. The
/// per-sub-address `service_hash:` lines are dropped for the same reason: they
/// are derived from the same command vocabulary the global fingerprint already
/// covers via [`push_commands`], so including them would be redundant coupling.
///
/// `description:` lines (both the per-device label and each per-command
/// description) are dropped because descriptions are documentation, not API
/// surface: editing or filling in a `TODO` description must not change the
/// fingerprint. This relies on descriptions being single-line — `TODO` or a
/// quoted string — which the generator and the description-preservation merge
/// both guarantee.
pub(crate) fn canonicalize(yaml: &str) -> String {
    let mut out = String::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("fingerprint:")
            || trimmed.starts_with("service_hash:")
            || trimmed.starts_with("description:")
        {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}


/// FNV-1a 32-bit hash of arbitrary bytes.
pub(crate) fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}


/// Compute the fingerprint of an already-canonicalized YAML string.
pub fn fingerprint_of_canonical(canonical: &str) -> u32 {
    fnv1a_32(canonical.as_bytes())
}


/// Replace the placeholder `fingerprint: 0x00000000` line with the real value
/// in whichever file carries it (the system file). Other files are untouched.
pub fn stamp_fingerprint_in_set(files: &mut [ManifestFile], fp: u32) {
    let placeholder = "fingerprint: 0x00000000";
    let real = format!("fingerprint: 0x{:08X}", fp);
    for f in files.iter_mut() {
        if f.content.contains(placeholder) {
            f.content = f.content.replacen(placeholder, &real, 1);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A command with no opcode fields, used to exercise the `description`
    /// branch of `push_commands` in isolation.
    fn bare_command(name: &str, opcode: u8, description: Option<&str>) -> CommandDef {
        CommandDef {
            name: name.to_string(),
            opcode,
            description: description.map(str::to_string),
            cmd_payload: PayloadDef::default(),
            resp_payload: PayloadDef::default(),
        }
    }

    /// t_desc
    ///
    /// `push_commands` emits a command's catalog `description` verbatim when one
    /// is present, and falls back to the `TODO` placeholder when it is `None` —
    /// the two branches a manifest consumer relies on (catalog defaults plus the
    /// "fill me in" marker).
    #[test]
    fn t_desc() {
        let cmds = vec![
            bare_command("SET", 0x01, Some("Set GPIO pin (high)")),
            bare_command("MYSTERY", 0x02, None),
        ];
        let mut out = String::new();
        push_commands(&mut out, &cmds);

        assert!(
            out.contains("    name: SET\n    description: Set GPIO pin (high)\n"),
            "a catalog description renders verbatim; got:\n{out}"
        );
        assert!(
            out.contains("    name: MYSTERY\n    description: TODO\n"),
            "a command without a description falls back to TODO; got:\n{out}"
        );
    }

    /// t_canon_service_hash
    ///
    /// `canonicalize` drops `service_hash:` lines (like `fingerprint:`) so the
    /// per-sub-address service hash never perturbs the global fingerprint: two
    /// manifests differing only by their `service_hash:` line canonicalize
    /// identically.
    #[test]
    fn t_canon_service_hash() {
        let with_a = "version: 1\nservice_hash: 0xAAAAAAAA\ntype: system\n";
        let with_b = "version: 1\nservice_hash: 0xBBBBBBBB\ntype: system\n";
        assert_eq!(
            canonicalize(with_a),
            canonicalize(with_b),
            "service_hash: lines must be stripped by canonicalize"
        );
        assert!(
            !canonicalize(with_a).contains("service_hash"),
            "canonicalize must not retain the service_hash line"
        );
    }
}
