// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Engine: turns a parsed [`crate::ioc_parser::DeviceSet`] into a working
//! C++ RT firmware project.
//!
//! # Responsibility
//!
//! The protocol layer (header packing, scheduler, CRC, handler interface)
//! lives in [`autogen_content::cpp`] as static C++ templates that are
//! embedded into the binary at compile time via `include_str!`. The engine
//! does **not** edit those files; it copies them verbatim into the output
//! tree on every run. What the engine produces:
//!
//! 1. A deterministic sub-address assignment (sub-address 1 reserved for
//!    a system handler; peripherals get 2..=31).
//! 2. A `device_registry.{h,cpp}` glue file that instantiates each
//!    per-peripheral handler with the right HAL handle / pin / channel
//!    arguments and registers them with a `RemoteTerminal`.
//! 3. An `rt_transport` adapter for the user-chosen UART.
//! 4. A per-sub-address YAML manifest set (an `api/` directory) documenting
//!    every sub-address and its command vocabulary, the `command_ids.h` /
//!    `sub_addresses.h` enum headers, and a 32-bit FNV-1a fingerprint over the
//!    set (embedded in the system manifest file and in `api_fingerprint.h`).
//!    The command vocabulary comes from the embedded command catalog
//!    ([`commands`]).
//!
//! # Entry points
//!
//! - [`select_transport`] — interactive prompt for the RT's own bus.
//! - [`build_artifacts`] — given a `DeviceSet` and a [`TransportChoice`],
//!   compute the sub-address table, the manifest YAML, and the
//!   fingerprint as a [`GeneratedArtifacts`] bundle.
//! - [`generate_rt`] — write the full `exhibition_rt/` tree.
//! - [`generate_bc`] — write the companion host-side `exhibition_bc/`
//!   tree. Both writers consume the same [`GeneratedArtifacts`] so the
//!   two trees agree on sub-addresses and API fingerprint.

pub mod addresses;
pub mod autogen_content;
pub mod bc_codegen;
pub mod codegen;
pub mod commands;
pub mod main_patch;
pub mod manifest;
pub mod merge;
pub mod transport_select;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::engine::main_patch::PatchReport;
use crate::gen_log::GenLog;
use crate::ioc_parser::{DeviceSet, ProjectLayout};

pub use transport_select::{select_transport, TransportChoice};

/// SPDX license/copyright banner prepended to every generated C/C++ source and
/// header file, including the trailing blank line that separates it from the
/// include guard. Kept in one place so all `render_*` functions emit an
/// identical header. The YAML and Markdown variants ([`SPDX_YAML_HEADER`],
/// [`SPDX_MARKDOWN_HEADER`]) carry the same identifiers with comment syntax
/// appropriate to those file types.
pub const SPDX_CPP_HEADER: &str = "// SPDX-License-Identifier: MIT OR Apache-2.0\n\
     // SPDX-FileCopyrightText: 2026 https://github.com/Sterling925\n\n";

/// SPDX banner for generated YAML manifest files (`#` comment lines). The
/// trailing newline separates it from the first manifest key. Comment lines are
/// stripped by [`manifest::canonicalize`] before fingerprinting, so this does
/// not perturb the API fingerprint.
pub const SPDX_YAML_HEADER: &str = "# SPDX-License-Identifier: MIT OR Apache-2.0\n\
     # SPDX-FileCopyrightText: 2026 https://github.com/Sterling925\n";

/// SPDX banner for generated Markdown files, wrapped in an HTML comment so it
/// does not render. Includes a trailing blank line before the first heading.
pub const SPDX_MARKDOWN_HEADER: &str = "<!--\n\
     SPDX-License-Identifier: MIT OR Apache-2.0\n\
     SPDX-FileCopyrightText: 2026 https://github.com/Sterling925\n\
     -->\n\n";


/// Inputs derived from a parsed device set that both the RT and BC
/// generators consume. Built once per run so the two trees share an
/// identical sub-address assignment, manifest, and API fingerprint.
pub struct GeneratedArtifacts {
    pub slots: Vec<addresses::DeviceSlot>,
    pub manifest_files: Vec<manifest::ManifestFile>,
    pub fingerprint: u32,
    /// On-disk project layout, so [`generate_rt`] routes sources to the
    /// directory the toolchain actually compiles. (Unused by the BC writer.)
    pub layout: ProjectLayout,
}


/// Assign sub-addresses, build the per-sub-address manifest files, and stamp
/// the fingerprint. The result is suitable as input to either
/// [`generate_rt`] or [`generate_bc`] (or both).
///
/// This non-logging form discards the generation log; use
/// [`build_artifacts_logged`] to capture the sub-address and fingerprint
/// decisions.
// Retained as the non-logging entry point exercised by the engine/BC tests.
#[allow(dead_code)]
pub fn build_artifacts(
    devices: &DeviceSet,
    transport: &TransportChoice,
) -> Result<GeneratedArtifacts, String> {
    build_artifacts_logged(devices, transport, &mut GenLog::disabled())
}


/// Like [`build_artifacts`], but record the sub-address assignment and the
/// final API fingerprint into `log`.
pub fn build_artifacts_logged(
    devices: &DeviceSet,
    transport: &TransportChoice,
    log: &mut GenLog,
) -> Result<GeneratedArtifacts, String> {
    let slots = addresses::assign_sub_addresses(devices, Some(transport.bus_name()))?;

    log.decision(format!(
        "Transport `{}` claims the RT's own bus and is not assigned a sub-address.",
        transport.bus_name(),
    ));
    for slot in &slots {
        log.decision(format!(
            "sub-address {} -> {} `{}` ({} member(s))",
            slot.sub_address,
            slot.kind.manifest_name(),
            slot.instance_name,
            slot.members.len(),
        ));
    }

    let mut manifest_files = manifest::build_manifest_files(&slots);
    let canonical = manifest::canonical_set(&manifest_files);
    let fingerprint = manifest::fingerprint_of_canonical(&canonical);
    manifest::stamp_fingerprint_in_set(&mut manifest_files, fingerprint);
    log.set_fingerprint(fingerprint);

    Ok(GeneratedArtifacts {
        slots,
        manifest_files,
        fingerprint,
        layout: devices.layout.clone(),
    })
}


/// Write the `exhibition_rt/` project under `out_root`.
///
/// This non-logging form discards the generation log; use
/// [`generate_rt_logged`] to also write the `generation_report.md` traceability
/// file into the project's `api/` directory.
// Retained as the non-logging counterpart to `generate_rt_logged`.
#[allow(dead_code)]
pub fn generate_rt(
    art: &GeneratedArtifacts,
    transport: &TransportChoice,
    out_root: &Path,
) -> Result<(), String> {
    generate_rt_logged(art, transport, out_root, &mut GenLog::disabled()).map(|_| ())
}


/// Like [`generate_rt`], but record code-generation decisions into `log` and
/// write the rendered report to `<src>/api/generation_report.md`.
///
/// Returns the result of auto-wiring the RT into the user's `main.c`: `Some`
/// with the patched path and a [`PatchReport`] when the file was found, or
/// `None` when it was not (the CLI then prints the manual integration steps).
pub fn generate_rt_logged(
    art: &GeneratedArtifacts,
    transport: &TransportChoice,
    out_root: &Path,
    log: &mut GenLog,
) -> Result<Option<(PathBuf, PatchReport)>, String> {
    codegen::write_project_logged(
        out_root,
        &art.slots,
        transport,
        &art.manifest_files,
        art.fingerprint,
        &art.layout,
        log,
    )
}


/// Write the `exhibition_bc/` project under `out_root`.
///
/// `transport` is consulted so the generated `main.cpp` can advertise the
/// real baud rate (for UART transports whose `.ioc` carried
/// `<PERIPH>.BaudRate`) instead of an unconditional 115200 default.
pub fn generate_bc(
    art: &GeneratedArtifacts,
    transport: &TransportChoice,
    out_root: &Path,
) -> Result<(), String> {
    bc_codegen::write_bc_project(
        out_root,
        &art.slots,
        transport,
        &art.manifest_files,
        art.fingerprint,
    )
}
