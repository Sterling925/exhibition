// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Code-generation back-end for the engine.
//!
//! Responsible for:
//!   - Splitting every emitted file across `<out_root>/Core/Inc/rt/`
//!     (headers) and `<out_root>/Core/Src/rt/` (sources, plus non-code
//!     companions like the `api/` manifest set and `INTEGRATION_GUIDE.md`).
//!   - Rewriting `#include` directives in the embedded static templates so
//!     they reference each other via the single `rt/<area>/<file>.h`
//!     prefix (resolved via the STM32CubeIDE default `Core/Inc` include
//!     path). The on-disk templates under `autogen_content/cpp/` are
//!     shared with the BC project verbatim; the rewrites happen only on
//!     the RT write path.
//!   - Emitting `api_fingerprint.h` with a `constexpr uint32_t` matching
//!     the YAML manifest.
//!   - Emitting `device_registry.{h,cpp}` that instantiates every device
//!     handler, registers it with a [`RemoteTerminal`], and exposes the
//!     `init_rt()` / `tick_rt()` entry points.
//!   - Emitting the `command_ids.h` (per-kind command opcode enums) and
//!     `sub_addresses.h` (`SubAddress` enum) headers.
//!   - Writing the generated per-sub-address `api/` manifest set.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::addresses::{DeviceKind, DeviceSlot};
use crate::engine::autogen_content::{
    device_handler_files, StaticFile, STATIC_FILES, UART_TRANSPORT_FILES,
    USB_CDC_TRANSPORT_FILES,
};
use crate::engine::commands;
use crate::engine::main_patch::{self, LineStatus, PatchReport};
use crate::engine::manifest::ManifestFile;
use crate::engine::transport_select::TransportChoice;
use crate::gen_log::GenLog;
use crate::ioc_parser::ProjectLayout;


/// Subdirectory (relative to the user-supplied project root) where every
/// generated header lands. STM32CubeIDE puts `Core/Inc` on the default
/// include path, so headers under `Core/Inc/rt/...` are reachable as
/// `#include "rt/..."` with no IDE configuration step.
pub const INC_DIR_REL: &str = "Core/Inc/rt";

/// Default subdirectory (relative to the user-supplied project root) where
/// generated implementation files land for an "under root" project
/// (`ProjectManager.UnderRoot=true`). STM32CubeIDE's `Core/Src` source folder
/// recurses into subdirectories, so `.cpp` files under `Core/Src/rt/...` are
/// picked up by the build automatically. Non-code companion files (the `api/`
/// manifest set, `INTEGRATION_GUIDE.md`, `communication/README.md`) also live
/// here so they stay adjacent to the code they describe; the build ignores
/// them.
///
/// For `UnderRoot=false` projects the source location differs — see
/// [`source_dir_rel`].
pub const SRC_DIR_REL: &str = "Core/Src/rt";


/// Resolve the source subdirectory (relative to the project root) for a
/// given [`ProjectLayout`].
///
/// - `UnderRoot=true` → [`SRC_DIR_REL`] (`Core/Src/rt`).
/// - `UnderRoot=false` with the STM32CubeIDE toolchain → CubeMX builds user
///   code from `<toolchain>/Application/User/Core`, so sources go in an `rt/`
///   subfolder there (the source folder compiles recursively).
/// - `UnderRoot=false` with any other / unknown toolchain → fall back to
///   [`SRC_DIR_REL`]; the build layout for those toolchains is not modeled, so
///   the caller should warn that the path may need manual adjustment.
///
/// Headers always go to [`INC_DIR_REL`] (`Core/Inc/rt`), which is on the
/// include path for both layouts.
pub fn source_dir_rel(layout: &ProjectLayout) -> String {
    if layout.under_root {
        return SRC_DIR_REL.to_string();
    }
    match layout.toolchain.as_str() {
        "STM32CubeIDE" => "STM32CubeIDE/Application/User/Core/rt".to_string(),
        _ => SRC_DIR_REL.to_string(),
    }
}


/// Whether [`source_dir_rel`] modeled this layout, or fell back to the
/// default. Used by the CLI to warn when an `UnderRoot=false` project uses a
/// toolchain whose on-disk structure is not known.
pub fn layout_is_modeled(layout: &ProjectLayout) -> bool {
    layout.under_root || layout.toolchain == "STM32CubeIDE"
}


/// Top-level codegen entry point.
///
/// Creates `<out_root>/Core/Inc/rt/` and `<out_root>/Core/Src/rt/` if
/// they do not exist, then writes every static template (with includes
/// rewritten for the new layout), the chosen transport's adapter, the
/// registry file, `api_fingerprint.h`, the `command_ids.h` / `sub_addresses.h`
/// enum headers, the per-sub-address `api/` manifest set, and
/// `INTEGRATION_GUIDE.md` — each routed to whichever tree matches its
/// extension.
///
/// This non-logging form discards the generation log; use
/// [`write_project_logged`] to also write the `generation_report.md`
/// traceability file.
// Retained as the non-logging counterpart to `write_project_logged`.
#[allow(dead_code)]
pub fn write_project(
    out_root: &Path,
    slots: &[DeviceSlot],
    transport: &TransportChoice,
    manifest_files: &[ManifestFile],
    fingerprint: u32,
    layout: &ProjectLayout,
) -> Result<(), String> {
    write_project_logged(
        out_root, slots, transport, manifest_files, fingerprint, layout,
        &mut GenLog::disabled(),
    )
    .map(|_| ())
}


/// Like [`write_project`], but record code-generation decisions into `log`
/// and write the rendered report to `<src>/api/generation_report.md` as the
/// final step, alongside the YAML manifest set.
///
/// As a last step it also auto-wires the RT into the user's CubeMX `main.c`
/// (see [`main_patch`]). The returned `Option` reports what happened: `Some`
/// with the patched path and a [`PatchReport`] when a `main.c`/`main.cpp` was
/// found, or `None` when neither exists (the caller then prints the manual
/// integration steps). A failure to patch is logged as a warning and does not
/// abort generation — the generated tree is still complete.
pub fn write_project_logged(
    out_root: &Path,
    slots: &[DeviceSlot],
    transport: &TransportChoice,
    manifest_files: &[ManifestFile],
    fingerprint: u32,
    layout: &ProjectLayout,
    log: &mut GenLog,
) -> Result<Option<(PathBuf, PatchReport)>, String> {
    let inc_root = out_root.join(INC_DIR_REL);
    let src_root = out_root.join(source_dir_rel(layout));
    for root in [&inc_root, &src_root] {
        fs::create_dir_all(root)
            .map_err(|e| format!("Failed to create '{}': {}", root.display(), e))?;
    }

    for f in STATIC_FILES {
        write_static_file(&inc_root, &src_root, f)?;
    }

    // Per-peripheral handlers: emit only the kinds actually used so the build
    // never compiles a handler that references HAL symbols the `.ioc` never
    // enabled (which would fail to link).
    for kind in kinds_used(slots) {
        log.decision(format!(
            "emitted `{}` device handler (kind in use)",
            kind.manifest_name(),
        ));
        for f in device_handler_files(kind) {
            write_static_file(&inc_root, &src_root, f)?;
        }
    }

    match transport {
        TransportChoice::Uart { .. } => {
            log.decision("emitted UART frame-transport adapter for the chosen transport");
            for f in UART_TRANSPORT_FILES {
                write_static_file(&inc_root, &src_root, f)?;
            }
        }
        TransportChoice::UsbCdc { .. } => {
            log.decision("emitted USB-CDC frame-transport adapter for the chosen transport");
            for f in USB_CDC_TRANSPORT_FILES {
                write_static_file(&inc_root, &src_root, f)?;
            }
        }
    }

    write_text(&inc_root.join("api_fingerprint.h"),
               &render_fingerprint_header(fingerprint))?;
    write_text(&inc_root.join("command_ids.h"),
               &commands::render_command_ids_header())?;
    write_text(&inc_root.join("service_hashes.h"),
               &commands::render_service_hashes_header())?;
    write_text(&inc_root.join("sub_addresses.h"),
               &commands::render_sub_addresses_header(slots))?;
    write_text(&inc_root.join("device_registry.h"),
               &render_registry_header())?;
    write_text(&src_root.join("device_registry.cpp"),
               &render_registry_source(slots, transport))?;
    log.decision(format!(
        "generated glue: api_fingerprint.h (0x{:08x}), command_ids.h, service_hashes.h, \
         sub_addresses.h, device_registry.{{h,cpp}} for {} sub-address slot(s)",
        fingerprint, slots.len(),
    ));

    let api_dir = src_root.join("api");
    fs::create_dir_all(&api_dir)
        .map_err(|e| format!("Failed to create '{}': {}", api_dir.display(), e))?;
    for f in manifest_files {
        write_manifest_preserving(&api_dir.join(&f.file_name), &f.content)?;
    }

    write_text(&src_root.join("INTEGRATION_GUIDE.md"),
               &render_integration_guide(slots, transport, fingerprint, layout))?;

    // Auto-wire the RT into the user's CubeMX main.c (include + init_rt() +
    // tick_rt()), confined to USER CODE regions. Never fatal: a missing file or
    // region just means the user keeps doing it by hand, so a patch failure is
    // logged and folded into the returned report rather than propagated.
    let patch = match main_patch::patch_main_file(out_root, layout) {
        Ok(Some((path, report))) => {
            log_patch(log, &path, &report);
            Some((path, report))
        }
        Ok(None) => {
            log.decision(
                "main.c / main.cpp not found next to the RT sources; the include, \
                 init_rt(), and tick_rt() lines must be added by hand (see \
                 INTEGRATION_GUIDE.md)",
            );
            None
        }
        Err(e) => {
            log.decision(format!(
                "could not patch main.c automatically ({e}); add the include, \
                 init_rt(), and tick_rt() lines by hand (see INTEGRATION_GUIDE.md)"
            ));
            None
        }
    };

    // Write the generation report last, next to the YAML manifests, so it is
    // the authoritative trace of everything decided above.
    log.write_to(&api_dir.join("generation_report.md"))?;

    Ok(patch)
}


/// Record into `log` what [`main_patch::patch_main_file`] did to each of the
/// three integration lines.
fn log_patch(log: &mut GenLog, path: &Path, report: &PatchReport) {
    for (code, _tag, status) in report.lines() {
        match status {
            LineStatus::Inserted => log.decision(format!(
                "inserted `{}` into {}", code, path.display()
            )),
            LineStatus::AlreadyPresent => log.decision(format!(
                "`{}` already present in {}; left as-is", code, path.display()
            )),
            LineStatus::RegionMissing => log.decision(format!(
                "could not insert `{}` into {}: its USER CODE region is missing; \
                 add it by hand (see INTEGRATION_GUIDE.md)",
                code, path.display()
            )),
        }
    }
}


/// Route a [`StaticFile`] into either `inc_root` (headers) or `src_root`
/// (everything else: `.cpp`, `.md`, etc.) based on extension, and rewrite
/// its embedded `#include` directives so they match the new on-disk
/// layout. The template source on disk is shared with the BC project, so
/// the rewrite happens only here on the way out.
fn write_static_file(
    inc_root: &Path,
    src_root: &Path,
    f: &StaticFile,
) -> Result<(), String> {
    let base = if is_header_path(f.rel_path) { inc_root } else { src_root };
    let full = base.join(f.rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }
    let rewritten = rewrite_includes_for_rt(f.contents);
    write_text(&full, &rewritten)
}


fn is_header_path(rel_path: &str) -> bool {
    rel_path.ends_with(".h") || rel_path.ends_with(".hpp")
}


/// Rewrite the small set of `#include` directives used inside the shared
/// `autogen_content/cpp/` templates so they reference each other through
/// the `rt/<area>/<file>.h` prefix that resolves via `Core/Inc`. Each
/// entry matches the full quoted include (including the leading
/// `#include` and the closing quote) so substitutions cannot cascade —
/// once an include has been rewritten to `"rt/..."` no later rule
/// matches it.
fn rewrite_includes_for_rt(contents: &str) -> String {
    // (old, new) pairs. Order does not matter because each `old` is the
    // complete `#include "..."` string and the rewritten form contains a
    // slash that none of the `old` patterns include — so no rule can
    // re-fire on a previous rule's output.
    const REWRITES: &[(&str, &str)] = &[
        // communication/ headers including each other (bare filenames).
        ("#include \"message_header.h\"",
         "#include \"rt/communication/message_header.h\""),
        ("#include \"i_clock.h\"",
         "#include \"rt/communication/i_clock.h\""),
        ("#include \"i_frame_transport.h\"",
         "#include \"rt/communication/i_frame_transport.h\""),
        ("#include \"i_sub_address_handler.h\"",
         "#include \"rt/communication/i_sub_address_handler.h\""),
        ("#include \"scheduler.h\"",
         "#include \"rt/communication/scheduler.h\""),
        ("#include \"sync.h\"",
         "#include \"rt/communication/sync.h\""),
        ("#include \"fault_counter.h\"",
         "#include \"rt/communication/fault_counter.h\""),

        // common/crc.cpp including its own header.
        ("#include \"crc.h\"",
         "#include \"rt/common/crc.h\""),

        // common/helper_functions.cpp including its own header.
        ("#include \"helper_functions.h\"",
         "#include \"rt/common/helper_functions.h\""),

        // Same-folder source-to-header includes that need the rt/ prefix.
        ("#include \"system_handler.h\"",
         "#include \"rt/system/system_handler.h\""),
        ("#include \"uart_frame_transport.h\"",
         "#include \"rt/transports/uart_frame_transport.h\""),
        ("#include \"usb_cdc_frame_transport.h\"",
         "#include \"rt/transports/usb_cdc_frame_transport.h\""),
        ("#include \"gpio_output_handler.h\"",
         "#include \"rt/devices/gpio_output_handler.h\""),
        ("#include \"gpio_input_handler.h\"",
         "#include \"rt/devices/gpio_input_handler.h\""),
        ("#include \"adc_handler.h\"",
         "#include \"rt/devices/adc_handler.h\""),
        ("#include \"dac_handler.h\"",
         "#include \"rt/devices/dac_handler.h\""),
        ("#include \"pwm_handler.h\"",
         "#include \"rt/devices/pwm_handler.h\""),
        ("#include \"spi_handler.h\"",
         "#include \"rt/devices/spi_handler.h\""),
        ("#include \"uart_handler.h\"",
         "#include \"rt/devices/uart_handler.h\""),
        ("#include \"i2c_handler.h\"",
         "#include \"rt/devices/i2c_handler.h\""),
        ("#include \"can_handler.h\"",
         "#include \"rt/devices/can_handler.h\""),
        ("#include \"qspi_handler.h\"",
         "#include \"rt/devices/qspi_handler.h\""),
        ("#include \"rtc_handler.h\"",
         "#include \"rt/devices/rtc_handler.h\""),

        // One-up traversal from communication/ and transports/.
        ("#include \"../common/crc.h\"",
         "#include \"rt/common/crc.h\""),
        ("#include \"../common/helper_functions.h\"",
         "#include \"rt/common/helper_functions.h\""),
        ("#include \"../api_fingerprint.h\"",
         "#include \"rt/api_fingerprint.h\""),
        ("#include \"../communication/i_clock.h\"",
         "#include \"rt/communication/i_clock.h\""),
        ("#include \"../communication/i_frame_transport.h\"",
         "#include \"rt/communication/i_frame_transport.h\""),
        ("#include \"../communication/i_sub_address_handler.h\"",
         "#include \"rt/communication/i_sub_address_handler.h\""),
        ("#include \"../communication/message_header.h\"",
         "#include \"rt/communication/message_header.h\""),
        ("#include \"../communication/sync.h\"",
         "#include \"rt/communication/sync.h\""),
        ("#include \"../communication/offset_smoother.h\"",
         "#include \"rt/communication/offset_smoother.h\""),
        ("#include \"../communication/phase_anchor.h\"",
         "#include \"rt/communication/phase_anchor.h\""),
        ("#include \"../communication/fault_counter.h\"",
         "#include \"rt/communication/fault_counter.h\""),

        // Two-up traversal from devices/<area>/.
        ("#include \"../../communication/i_sub_address_handler.h\"",
         "#include \"rt/communication/i_sub_address_handler.h\""),
        ("#include \"../../communication/i_clock.h\"",
         "#include \"rt/communication/i_clock.h\""),

        // Generated command-id enums, included by the device handlers
        // (two-up) and the system handler (one-up).
        ("#include \"../../generated/command_ids.h\"",
         "#include \"rt/command_ids.h\""),
        ("#include \"../generated/command_ids.h\"",
         "#include \"rt/command_ids.h\""),
    ];

    let mut s = contents.to_string();
    for (from, to) in REWRITES {
        s = s.replace(from, to);
    }
    s
}


fn write_text(path: &PathBuf, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))
}


/// Write a manifest YAML file, first splicing in any per-command `description`
/// the user filled into the copy already on disk (if one exists). Behaves like
/// [`write_text`] when there is no existing file. See
/// [`crate::engine::merge::preserve_command_descriptions`].
fn write_manifest_preserving(path: &PathBuf, generated: &str) -> Result<(), String> {
    let to_write = match fs::read_to_string(path) {
        Ok(existing) => crate::engine::merge::preserve_command_descriptions(generated, &existing),
        Err(_) => generated.to_string(),
    };
    fs::write(path, &to_write)
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))
}


fn render_fingerprint_header(fp: u32) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef API_FINGERPRINT_H\n");
    s.push_str("#define API_FINGERPRINT_H\n");
    s.push('\n');
    s.push_str("#include <stdint.h>\n");
    s.push('\n');
    s.push_str("/// @brief 32-bit FNV-1a fingerprint of the generated API manifest.\n");
    s.push_str("/// @details Regenerated each time `exhibition` runs. The BC can\n");
    s.push_str("///          read this value via the system handler (sub-address 1,\n");
    s.push_str("///          opcode 0x01) and compare against its own copy.\n");
    s.push_str(&format!("constexpr uint32_t kApiFingerprint = 0x{:08X}u;\n", fp));
    s.push('\n');
    s.push_str("#endif // API_FINGERPRINT_H\n");
    s
}


fn render_registry_header() -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef DEVICE_REGISTRY_H\n");
    s.push_str("#define DEVICE_REGISTRY_H\n");
    s.push('\n');
    s.push_str("/// @file device_registry.h\n");
    s.push_str("/// @brief Entry points into the auto-generated RemoteTerminal.\n");
    s.push_str("/// @details\n");
    s.push_str("/// Call `init_rt()` once at startup after the HAL is initialized.\n");
    s.push_str("/// Call `tick_rt()` from the main loop (or a timer task) as often\n");
    s.push_str("/// as you can afford; each call advances the RT state machine by\n");
    s.push_str("/// at most one step. Both are non-blocking.\n");
    s.push('\n');
    // Wrap in a C linkage guard so a C translation unit (e.g. a CubeMX
    // `main.c`) links against the same unmangled symbols that the C++
    // `device_registry.cpp` defines. Without this the C++ definitions are
    // name-mangled and a C caller fails to link.
    s.push_str("#ifdef __cplusplus\n");
    s.push_str("extern \"C\" {\n");
    s.push_str("#endif\n");
    s.push('\n');
    s.push_str("void init_rt();\n");
    s.push_str("void tick_rt();\n");
    s.push('\n');
    s.push_str("#ifdef __cplusplus\n");
    s.push_str("}\n");
    s.push_str("#endif\n");
    s.push('\n');
    // Optional user hook (C++ only, so a C `main.c` including this header is
    // unaffected). `device_registry.cpp` supplies a weak no-op default.
    s.push_str("#ifdef __cplusplus\n");
    s.push_str("class RemoteTerminal;\n");
    s.push('\n');
    s.push_str("/// @brief Optional user hook, called once at the end of `init_rt()`,\n");
    s.push_str("///        after every generated handler is registered.\n");
    s.push_str("/// @details Define it in your own source file (outside the generated\n");
    s.push_str("///          folders, so regeneration never touches it) to register extra\n");
    s.push_str("///          ISubAddressHandler instances on unused sub-addresses, e.g.\n");
    s.push_str("///          `rt.register_handler(30, &my_handler, kMyServiceHash);`.\n");
    s.push_str("///          `device_registry.cpp` provides an empty weak default, so a\n");
    s.push_str("///          project that does not define it is unaffected.\n");
    s.push_str("void rt_user_init(RemoteTerminal& rt);\n");
    s.push_str("#endif\n");
    s.push('\n');
    s.push_str("#endif // DEVICE_REGISTRY_H\n");
    s
}


fn render_registry_source(slots: &[DeviceSlot], transport: &TransportChoice) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("/// @file device_registry.cpp\n");
    s.push_str("/// @brief Wiring for the auto-generated RemoteTerminal.\n");
    s.push_str("/// @details Auto-generated by Exhibition. Instantiates each device\n");
    s.push_str("///          handler with its HAL handle/pin/channel and registers it\n");
    s.push_str("///          with the RemoteTerminal. Regenerated on every run; edit the\n");
    s.push_str("///          .ioc and rerun rather than hand-editing this file.\n");
    s.push('\n');
    s.push_str("#include \"rt/device_registry.h\"\n");
    s.push('\n');
    s.push_str("#include \"main.h\"\n");
    // CubeMX peripheral headers that declare the HAL handles instantiated
    // below (`&hi2c1`, `&huart3`, …). `main.h` does not pull these in.
    for header in hal_peripheral_headers(slots, transport) {
        s.push_str(&format!("#include \"{}\"\n", header));
    }
    s.push_str("#include \"rt/communication/scheduler.h\"\n");
    s.push_str("#include \"rt/system/system_handler.h\"\n");
    s.push_str("#include \"rt/sub_addresses.h\"\n");
    s.push_str("#include \"rt/service_hashes.h\"\n");
    s.push_str("#include \"rt/transports/sys_tick_clock.h\"\n");
    match transport {
        TransportChoice::Uart { .. } => {
            s.push_str("#include \"rt/transports/uart_frame_transport.h\"\n");
        }
        TransportChoice::UsbCdc { .. } => {
            s.push_str("#include \"rt/transports/usb_cdc_frame_transport.h\"\n");
        }
    }

    // Includes for handler classes actually used by at least one slot.
    for kind in kinds_used(slots) {
        s.push_str(&format!("#include \"rt/devices/{}\"\n", handler_header_filename(kind)));
    }
    s.push('\n');

    // Transport + clock + system handler.
    match transport {
        TransportChoice::Uart { hal_handle, .. } => {
            s.push_str(&format!(
                "static UartFrameTransport g_transport(&{});\n",
                hal_handle
            ));
        }
        TransportChoice::UsbCdc { .. } => {
            // SAFETY: `transmit_fn_name()` is always Some for UsbCdc.
            let fn_name = transport.transmit_fn_name().unwrap();
            s.push_str(&format!(
                "extern \"C\" uint8_t {fn}(uint8_t* buf, uint16_t len);\n\
                 static UsbCdcFrameTransport g_transport({fn});\n\
                 extern \"C\" void exhibition_usb_rx(uint8_t const* buf, uint32_t len)\n\
                 {{\n\
                 \x20\x20\x20\x20g_transport.deposit_rx(buf, len);\n\
                 }}\n",
                fn = fn_name,
            ));
        }
    }
    s.push_str("static SysTickClock g_clock;\n");
    // Shared message-fault counter: the RemoteTerminal increments it on timeout
    // / CRC / malformed frames; the system handler reads it back over the wire.
    s.push_str("static FaultCounter g_fault_counter;\n");
    // Smooths the BC time offset so now_bc_ms() glides toward the BC clock instead
    // of stepping at every SYNC_FINAL.
    s.push_str("static OffsetSmoother g_offset_smoother;\n");
    s.push_str(&format!(
        "static SystemHandler g_system_handler({}, g_clock, g_fault_counter, g_offset_smoother);\n",
        slots.len()
    ));
    s.push('\n');

    // Per-device handler instances.
    for slot in slots {
        s.push_str(&render_instance_decl(slot));
    }
    s.push('\n');

    // RT singleton + entry points.
    s.push_str("static RemoteTerminal g_rt(g_transport, g_clock, g_fault_counter);\n");
    s.push('\n');
    // Weak no-op default for the user hook declared in device_registry.h; a
    // definition in user code replaces it at link time.
    s.push_str("// Weak default for the optional user hook declared in device_registry.h.\n");
    s.push_str("// A definition in user code replaces it at link time.\n");
    s.push_str("__attribute__((weak)) void rt_user_init(RemoteTerminal& rt)\n{\n");
    s.push_str("    (void)rt;\n");
    s.push_str("}\n");
    s.push('\n');
    s.push_str("void init_rt()\n{\n");
    // Cold-start invariant: SystemCoreClock must reflect the post-PLL frequency before the
    // clock latches its nominal SysTick reload. SystemClock_Config() already updates it, but
    // refresh it defensively here so the reload is provably SystemCoreClock/1000 (168000 on
    // the F407) regardless of how the user's main.c is wired -- the file-scope SysTickClock
    // ctor ran before the PLL and cached the 16 MHz reset default. Emit a matching C++
    // comment into the generated source so the line is self-explanatory there too.
    s.push_str("    // Refresh SystemCoreClock from the live PLL config before latching the\n");
    s.push_str("    // SysTick reload: the file-scope clock ctor ran pre-PLL (16 MHz default).\n");
    s.push_str("    SystemCoreClockUpdate();\n");
    s.push_str("    g_clock.sync_from_core_clock();\n");
    s.push_str(&format!(
        "    g_rt.register_handler(static_cast<uint8_t>(SubAddress::System), \
         &g_system_handler, {});\n",
        commands::service_hash_const_name(commands::system()),
    ));
    for slot in slots {
        s.push_str(&format!(
            "    g_rt.register_handler(static_cast<uint8_t>(SubAddress::{}), &g_{}, {});\n",
            commands::to_pascal_case(&slot.instance_name),
            slot.instance_name,
            commands::service_hash_const_name(commands::for_kind(slot.kind)),
        ));
    }
    s.push_str("    // Let user code register its own handlers (no-op unless defined).\n");
    s.push_str("    rt_user_init(g_rt);\n");
    s.push_str("}\n");
    s.push('\n');
    s.push_str("void tick_rt()\n{\n");
    s.push_str("    g_rt.tick();\n");
    s.push_str("    if (g_system_handler.reset_pending()) {\n");
    s.push_str("        HAL_Delay(10);\n");
    s.push_str("        HAL_NVIC_SystemReset();\n");
    s.push_str("    }\n");
    s.push_str("}\n");
    s
}


/// The distinct device kinds present in `slots`, sorted deterministically by
/// [`DeviceKind::sort_key`]. Drives both which handler source files are
/// emitted ([`write_project`]) and which handler headers / HAL includes the
/// registry needs.
fn kinds_used(slots: &[DeviceSlot]) -> Vec<DeviceKind> {
    let mut kinds: Vec<DeviceKind> = slots.iter().map(|s| s.kind).collect();
    kinds.sort_by_key(|k| k.sort_key());
    kinds.dedup();
    kinds
}


/// The CubeMX peripheral headers (`i2c.h`, `usart.h`, …) that declare the HAL
/// handles `device_registry.cpp` instantiates. Without them the registry's
/// `&hi2c1` / `&huart3` references are undeclared and the file will not
/// compile. Derived from the device kinds in use plus the transport's own
/// peripheral. Sorted and de-duplicated for deterministic output.
fn hal_peripheral_headers(slots: &[DeviceSlot], transport: &TransportChoice) -> Vec<String> {
    let mut headers: Vec<String> = slots
        .iter()
        .filter_map(hal_header_for_slot)
        .map(str::to_string)
        .collect();
    // A UART transport references its own `&huartN` handle in the registry.
    if let TransportChoice::Uart { .. } = transport {
        headers.push("usart.h".to_string());
    }
    headers.sort();
    headers.dedup();
    headers
}


/// The CubeMX header that declares the HAL handle for `slot`, or `None` for
/// kinds whose symbols arrive through `main.h` (GPIO needs only the port
/// macros and `HAL_GPIO_*`, both reachable via `main.h`).
fn hal_header_for_slot(slot: &DeviceSlot) -> Option<&'static str> {
    match slot.kind {
        DeviceKind::GpioOutput | DeviceKind::GpioInput => None,
        DeviceKind::Adc  => Some("adc.h"),
        DeviceKind::Dac  => Some("dac.h"),
        DeviceKind::Pwm  => Some("tim.h"),
        DeviceKind::Qspi => Some("quadspi.h"),
        DeviceKind::Rtc  => Some("rtc.h"),
        DeviceKind::Spi  => Some("spi.h"),
        DeviceKind::Uart => Some("usart.h"),
        DeviceKind::I2c  => Some("i2c.h"),
        // bxCAN (`can.h`) and FDCAN (`fdcan.h`) live in differently-named
        // CubeMX headers; the handle name (`hcan1` vs `hfdcan1`) disambiguates.
        DeviceKind::Can => {
            if detail(slot, "hal_handle").unwrap_or("").contains("fdcan") {
                Some("fdcan.h")
            } else {
                Some("can.h")
            }
        }
    }
}


/// Filename within `devices/` for the header that defines this kind's
/// handler class.
fn handler_header_filename(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::GpioOutput => "gpio_output_handler.h",
        DeviceKind::GpioInput  => "gpio_input_handler.h",
        DeviceKind::Adc        => "adc_handler.h",
        DeviceKind::Dac        => "dac_handler.h",
        DeviceKind::Pwm        => "pwm_handler.h",
        DeviceKind::Qspi       => "qspi_handler.h",
        DeviceKind::Rtc        => "rtc_handler.h",
        DeviceKind::Spi        => "spi_handler.h",
        DeviceKind::Uart       => "uart_handler.h",
        DeviceKind::I2c        => "i2c_handler.h",
        DeviceKind::Can        => "can_handler.h",
    }
}


/// Lookup helper for slot `details` rows.
fn detail<'a>(slot: &'a DeviceSlot, key: &str) -> Option<&'a str> {
    slot.details
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}


/// Human-readable name for a [`DeviceKind`] used in the integration guide
/// table. (Distinct from [`DeviceKind::sort_key`] / [`DeviceKind::manifest_name`]
/// so the table reads naturally — e.g. `GPIO output` instead of `gpio_out`.)
fn kind_display_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::GpioOutput => "GPIO output",
        DeviceKind::GpioInput  => "GPIO input",
        DeviceKind::Adc        => "ADC",
        DeviceKind::Dac        => "DAC",
        DeviceKind::Pwm        => "PWM",
        DeviceKind::Qspi       => "QUAD SPI",
        DeviceKind::Rtc        => "RTC",
        DeviceKind::Spi        => "SPI",
        DeviceKind::Uart       => "UART",
        DeviceKind::I2c        => "I2C",
        DeviceKind::Can        => "CAN",
    }
}


/// Render `INTEGRATION_GUIDE.md` — a step-by-step guide for wiring the
/// generated `exhibition_rt/` tree into an STM32CubeIDE project. The guide
/// is rendered (not static) so the chosen transport and the registered
/// device list are named concretely.
fn render_integration_guide(
    slots: &[DeviceSlot],
    transport: &TransportChoice,
    fingerprint: u32,
    layout: &ProjectLayout,
) -> String {
    let bus_name = transport.bus_name();
    let src_rel = source_dir_rel(layout);

    let mut s = String::new();

    // ---- Header ----
    s.push_str(crate::engine::SPDX_MARKDOWN_HEADER);
    s.push_str("# Integrating `exhibition_rt` into an STM32CubeIDE project\n");
    s.push('\n');
    s.push_str("This guide was auto-generated alongside the rest of `exhibition_rt/` ");
    s.push_str("and references the actual transport and devices you selected.\n");
    s.push('\n');

    // ---- 1. What this is ----
    s.push_str("## 1. What this is\n");
    s.push('\n');
    match transport {
        TransportChoice::Uart { hal_handle, .. } => {
            s.push_str(&format!(
                "The Remote Terminal (RT) is a single-threaded, polling-based slave that \
                 listens on **{bus}** (HAL handle `{handle}`) and dispatches incoming \
                 commands to the device handlers listed below. There is no RTOS \
                 dependency: the RT advances one step per call to `tick_rt()` and returns \
                 immediately if there is nothing to do.\n",
                bus = bus_name,
                handle = hal_handle,
            ));
        }
        TransportChoice::UsbCdc { speed, .. } => {
            s.push_str(&format!(
                "The Remote Terminal (RT) is a single-threaded, polling-based slave that \
                 listens on **{bus}** as a USB CDC virtual COM port (USB {sp}). Once the \
                 board enumerates, the host operating system exposes the link as a \
                 standard serial device — `/dev/ttyACM*` on Linux, `COMx` on Windows — so \
                 the BC side can drive it through its normal serial transport. There is \
                 no RTOS dependency: the RT advances one step per call to `tick_rt()` \
                 and returns immediately if there is nothing to do.\n",
                bus = bus_name,
                sp = speed.suffix(),
            ));
        }
    }
    s.push('\n');
    s.push_str(&format!(
        "API fingerprint for this build: `0x{:08X}`. The BC can read this back over the \
         wire via the system handler to confirm both sides agree on the device layout.\n",
        fingerprint,
    ));
    s.push('\n');
    s.push_str(
        "Every sub-address (including the system handler) also answers a universal \
         command, opcode `0x00` (`GET_SERVICE_HASH`), returning a per-service API hash \
         over just that service's command vocabulary. When the global fingerprint check \
         fails, the BC's `verify_service_hashes()` reads each sub-address's hash to \
         pinpoint which device handler diverged.\n",
    );
    s.push('\n');

    // ---- 2. Devices registered ----
    s.push_str("## 2. Devices registered\n");
    s.push('\n');
    s.push_str("| Sub-address | Kind | Instance | Label |\n");
    s.push_str("|------------:|------|----------|-------|\n");
    s.push_str("| 1 | System | `g_system_handler` | reserved (fingerprint / device count) |\n");
    for slot in slots {
        s.push_str(&format!(
            "| {sa} | {kind} | `g_{inst}` | {label} |\n",
            sa = slot.sub_address,
            kind = kind_display_name(slot.kind),
            inst = slot.instance_name,
            label = slot.label,
        ));
    }
    s.push('\n');

    // ---- 3. STM32CubeMX prerequisites ----
    s.push_str("## 3. STM32CubeMX prerequisites\n");
    s.push('\n');
    s.push_str(
        "Before integrating, confirm the following in your `.ioc` (open in STM32CubeMX \
         and regenerate code if you change anything):\n\n",
    );
    match transport {
        TransportChoice::Uart { .. } => {
            s.push_str(&format!(
                "- **{bus}** is enabled and `MX_{bus}_UART_Init()` (or `MX_{bus}_Init()`, \
                   depending on your HAL version) is called from the generated `main` \
                   before the main loop. Polling mode is sufficient — no global interrupt \
                   is required.\n",
                bus = bus_name,
            ));
        }
        TransportChoice::UsbCdc { speed, .. } => {
            s.push_str(&format!(
                "- **{bus}** is enabled with the USB middleware configured as a CDC \
                   device class for {sp} (the matching CubeMX entry is \
                   `USB_DEVICE.CLASS_NAME_{sp} = CDC`).\n\
                   - `MX_USB_DEVICE_Init()` is called from the generated `main` before \
                   the main loop.\n\
                   - The USB peripheral clock source is set to 48 MHz (HSI48, the PLL Q \
                   output, or `MSI` on L4 — CubeMX flags the right option). CDC will \
                   silently fail to enumerate if this is wrong.\n\
                   - You will install a small hook in the CubeMX-generated \
                   `USB_DEVICE/App/usbd_cdc_if.c` to forward received bytes to the RT — \
                   see section 5a below.\n",
                bus = bus_name,
                sp = speed.suffix(),
            ));
        }
    }
    s.push_str(
        "- **SysTick** is running so `HAL_GetTick()` returns a monotonically-increasing \
         millisecond count. This is the default for CubeMX projects; do not disable it.\n\
         - Every peripheral listed in section 2 above is enabled in the `.ioc` and the \
         CubeMX-generated HAL handle (`hadc1`, `htim2`, etc.) matches the name baked \
         into `device_registry.cpp`. If you rename a peripheral, re-run `exhibition` \
         against the updated `.ioc`.\n",
    );
    s.push('\n');

    // ---- 4. Build-system integration (STM32CubeIDE) ----
    s.push_str("## 4. Pick up the new files in STM32CubeIDE\n");
    s.push('\n');
    s.push_str(&format!(
        "Generated headers live under `{inc}/` and generated implementations live \
         under `{src}/`. Both locations are already covered by a CubeMX-generated \
         project's default include path and source folders, so no *Paths and Symbols* \
         configuration is required.\n",
        inc = INC_DIR_REL,
        src = src_rel,
    ));
    s.push('\n');
    s.push_str(&format!(
        "1. Right-click the project in the *Project Explorer* and choose \
         *Refresh* (or press **F5**) so the indexer notices the new files.\n\
         2. *Project \u{2192} Clean...* and then rebuild. CubeIDE should compile every \
         `{src}/**/*.cpp` automatically.\n",
        src = src_rel,
    ));
    s.push('\n');
    s.push_str(
        "Every internal `#include` between the generated files uses the form \
         `\"rt/<area>/<file>.h\"`, resolved through the default `Core/Inc` include path.\n",
    );
    s.push('\n');
    s.push_str(
        "**C / C++ note:** the RT is C++. STM32CubeMX defaults to C (`main.c`). If your \
         project has not been switched to C++ yet, either rename `main.c` to `main.cpp` \
         (and right-click the project \u{2192} *Convert to C++*) or keep `main.c` and \
         include `\"rt/device_registry.h\"` from C \u{2014} the entry points are already \
         declared with C linkage on the RT side via `device_registry.h`.\n",
    );
    s.push('\n');
    s.push_str(
        "**C++ standard:** build the project as **C++17** (or later). The generated RT \
         uses `if constexpr`, a C++17 feature; under an older standard the compiler emits a \
         `-Wc++17-extensions` warning and rejects it outright with `-pedantic-errors`. In \
         STM32CubeIDE this is *Project \u{2192} Properties \u{2192} C/C++ Build \u{2192} \
         Settings \u{2192} Tool Settings \u{2192} MCU G++ Compiler \u{2192} General \u{2192} \
         Language standard* \u{2192} **GNU C++17 (`-std=gnu++17`)** (or ISO C++17); *Apply and \
         Close* and rebuild.\n",
    );
    s.push('\n');

    // ---- 5. main.cpp wiring ----
    s.push_str("## 5. Wire `init_rt()` and `tick_rt()` into `main.cpp`\n");
    s.push('\n');
    s.push_str(
        "Exhibition normally does this for you: at the end of generation it inserts \
         the three lines below into your `main.c` (or `main.cpp`), each inside its \
         *USER CODE* fence, and skips any that are already present. The steps here are \
         the manual fallback for when your `main.c` lives somewhere exhibition did not \
         find it, or a USER CODE region is missing. The lines belong inside the *USER \
         CODE* fences so they survive CubeMX regeneration:\n",
    );
    s.push('\n');
    let init_call = match transport {
        TransportChoice::Uart { .. } => format!(
            "    MX_{}_UART_Init();   // and the other MX_*_Init() calls CubeMX generated\n",
            bus_name,
        ),
        TransportChoice::UsbCdc { .. } => String::from(
            "    MX_USB_DEVICE_Init();   // and the other MX_*_Init() calls CubeMX generated\n",
        ),
    };
    s.push_str("```cpp\n");
    s.push_str("/* USER CODE BEGIN Includes */\n");
    s.push_str("#include \"rt/device_registry.h\"\n");
    s.push_str("/* USER CODE END Includes */\n");
    s.push('\n');
    s.push_str("int main(void)\n");
    s.push_str("{\n");
    s.push_str("    HAL_Init();\n");
    s.push_str("    SystemClock_Config();\n");
    s.push_str("    MX_GPIO_Init();\n");
    s.push_str(&init_call);
    s.push_str("    /* ...other MX_*_Init() calls... */\n");
    s.push('\n');
    s.push_str("    /* USER CODE BEGIN 2 */\n");
    s.push_str("    init_rt();\n");
    s.push_str("    /* USER CODE END 2 */\n");
    s.push('\n');
    s.push_str("    while (1)\n");
    s.push_str("    {\n");
    s.push_str("        /* USER CODE BEGIN 3 */\n");
    s.push_str("        tick_rt();\n");
    s.push_str("        /* USER CODE END 3 */\n");
    s.push_str("    }\n");
    s.push_str("}\n");
    s.push_str("```\n");
    s.push('\n');
    s.push_str(
        "Call `tick_rt()` as often as you can afford. It is non-blocking and returns \
         immediately when the transport has no pending bytes; long stretches between \
         calls translate directly into request latency on the bus.\n",
    );
    s.push('\n');

    // ---- 5a. USB CDC receive hook (USB transport only) ----
    if let TransportChoice::UsbCdc { speed, .. } = transport {
        let sp = speed.suffix();
        s.push_str("## 5a. Install the USB CDC receive hook\n");
        s.push('\n');
        s.push_str(&format!(
            "The CubeMX USB CDC middleware delivers received bytes through a callback \
             in `USB_DEVICE/App/usbd_cdc_if.c` named `CDC_Receive_{sp}`. The generated \
             `device_registry.cpp` exports a C-linkage forwarder \
             `exhibition_usb_rx(buf, len)` that pushes the bytes into the RT's ring \
             buffer; you need to call it from `CDC_Receive_{sp}` so the bytes actually \
             reach the scheduler.\n",
            sp = sp,
        ));
        s.push('\n');
        s.push_str(
            "Open `USB_DEVICE/App/usbd_cdc_if.c`, locate the function below, and add \
             the two lines marked NEW inside the existing `USER CODE BEGIN 6` fence (so \
             they survive CubeMX regeneration):\n",
        );
        s.push('\n');
        s.push_str("```c\n");
        s.push_str(&format!(
            "static int8_t CDC_Receive_{sp}(uint8_t* Buf, uint32_t* Len)\n\
             {{\n\
             \x20\x20/* USER CODE BEGIN 6 */\n\
             \x20\x20extern void exhibition_usb_rx(uint8_t const* buf, uint32_t len); /* NEW */\n\
             \x20\x20exhibition_usb_rx(Buf, *Len);                                    /* NEW */\n\
             \x20\x20USBD_CDC_SetRxBuffer(&hUsbDevice{sp}, &Buf[0]);\n\
             \x20\x20USBD_CDC_ReceivePacket(&hUsbDevice{sp});\n\
             \x20\x20return (USBD_OK);\n\
             \x20\x20/* USER CODE END 6 */\n\
             }}\n",
            sp = sp,
        ));
        s.push_str("```\n");
        s.push('\n');
        s.push_str(
            "Place the `exhibition_usb_rx(...)` call **before** \
             `USBD_CDC_ReceivePacket(...)` so the next OUT transfer is re-armed promptly \
             — the hook only copies bytes into a ring buffer, so the time it spends in \
             the callback is negligible.\n",
        );
        s.push('\n');
    }

    // ---- 6. Verification ----
    s.push_str("## 6. Verify the integration end-to-end\n");
    s.push('\n');
    let (wiring_step, common_failures) = match transport {
        TransportChoice::Uart { baud_rate, .. } => {
            let (baud_for_cmd, baud_caveat) = match baud_rate {
                Some(b) => (
                    *b,
                    " (matches the `<PERIPH>.BaudRate` from your `.ioc`)".to_string(),
                ),
                None => (
                    115200u32,
                    "\n\n> **Note:** Your `.ioc` did not specify a baud rate for the chosen UART; the value above is a fallback. Override it from the BC command line if your firmware uses a different baud".to_string(),
                ),
            };
            (
                format!(
                    "Wire **{bus}** (TX/RX/GND) between the board and the host serial \
                     adapter, then run `./exhibition_bc /dev/ttyUSB0 {baud}` (Linux) or \
                     `./exhibition_bc.exe COM7 {baud}` (Windows){caveat}",
                    bus = bus_name,
                    baud = baud_for_cmd,
                    caveat = baud_caveat,
                ),
                "the host serial adapter being connected to the wrong UART".to_string(),
            )
        }
        TransportChoice::UsbCdc { .. } => (
            format!(
                "Plug the board's **{bus}** USB cable into the host. Once the board \
                 enumerates as a CDC device, run `./exhibition_bc /dev/ttyACM0 115200` \
                 (Linux) or `./exhibition_bc.exe COM7 115200` (Windows) — the baud \
                 argument is required by the BC even though USB CDC ignores it",
                bus = bus_name,
            ),
            "the host opening the wrong /dev/ttyACM* or COM port, or the \
             `exhibition_usb_rx` hook not being installed in `usbd_cdc_if.c`"
                .to_string(),
        ),
    };
    let first_gpio_output_pin = slots.iter()
        .filter(|s| s.kind == DeviceKind::GpioOutput)
        .flat_map(|s| s.members.iter()
                       .filter(|m| m.pin_index.is_some())
                       .map(move |m| (s, m)))
        .next();
    match first_gpio_output_pin {
        Some((slot, member)) => {
            s.push_str(&format!(
                "Your generated devices include a GPIO output (`{label}`, on bank \
                 sub-address {sa}). Use it as the smoke test:\n\n\
                 1. Build and flash the firmware.\n\
                 2. On the host side, build the BC project that was generated in the \
                    sibling `exhibition_bc/` folder (`cmake --preset Debug && \
                    cmake --build --preset Debug` in that directory).\n\
                 3. {wiring}.\n\
                 4. The relevant lines in `main.cpp` are:\n\n\
                 ```cpp\n\
                 client.{inst}.set();    // drives {label} high\n\
                 client.{inst}.clear();  // drives {label} low\n\
                 ```\n\n\
                 5. Confirm the LED tied to **{label}** toggles. If it does, the wire \
                    protocol, transport, scheduler, and your `tick_rt()` call site are \
                    all correct.\n",
                label = member.label,
                sa = slot.sub_address,
                inst = member.instance_name,
                wiring = wiring_step,
            ));
            s.push('\n');
            s.push_str(&format!(
                "If the BC reports timeouts instead of `Status::Ok`, the most common \
                 causes are: `init_rt()` not being called (no handler registered), \
                 `tick_rt()` not being called from the main loop (no scheduler progress), \
                 or {common}.\n",
                common = common_failures,
            ));
        }
        None => {
            s.push_str(&format!(
                "Your generated devices do not include a GPIO output, so the LED-toggle \
                 smoke test is not directly applicable. Use the system handler instead:\n\n\
                 1. Build and flash the firmware.\n\
                 2. Build the BC project that was generated in the sibling \
                    `exhibition_bc/` folder (`cmake --preset Debug && \
                    cmake --build --preset Debug` in that directory).\n\
                 3. {wiring}.\n\
                 4. `exhibition_bc` calls `client.get_api_fingerprint(fp)` against \
                    sub-address 1; the returned value should match `0x{fp:08X}` (also in \
                    `api_fingerprint.h`).\n",
                wiring = wiring_step,
                fp = fingerprint,
            ));
            s.push('\n');
            s.push_str(&format!(
                "If the BC reports timeouts instead of `Status::Ok`, the most common \
                 causes are: `init_rt()` not being called, `tick_rt()` not being called \
                 from the main loop, or {common}.\n",
                common = common_failures,
            ));
        }
    }
    s.push('\n');

    // ---- 7. Where to learn more ----
    s.push_str("## 7. Where to learn more\n");
    s.push('\n');
    s.push_str(
        "See [`communication/README.md`](communication/README.md) (shipped in this same \
         folder) for the wire protocol details: message-header layout, CRC-16 (MODBUS) \
         coverage, the `RemoteTerminal` / `BusController` APIs, and the configuration \
         constants you can tune for your platform.\n",
    );

    s
}


fn render_instance_decl(slot: &DeviceSlot) -> String {
    match slot.kind {
        DeviceKind::GpioOutput => format!(
            "static GpioOutputBankHandler g_{}({}, {}, g_clock);\n",
            slot.instance_name,
            detail(slot, "port").unwrap_or("nullptr"),
            detail(slot, "pin_mask").unwrap_or("0")
        ),
        DeviceKind::GpioInput => format!(
            "static GpioInputBankHandler g_{}({}, {}, g_clock);\n",
            slot.instance_name,
            detail(slot, "port").unwrap_or("nullptr"),
            detail(slot, "pin_mask").unwrap_or("0")
        ),
        DeviceKind::Adc => format!(
            "static AdcHandler g_{}({}, {}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr"),
            detail(slot, "channel").unwrap_or("0")
        ),
        DeviceKind::Dac => format!(
            "static DacHandler g_{}({}, {}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr"),
            detail(slot, "channel").unwrap_or("0")
        ),
        DeviceKind::Pwm => format!(
            "static PwmHandler g_{}({}, {}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr"),
            detail(slot, "channel").unwrap_or("0")
        ),
        DeviceKind::Spi => format!(
            "static SpiHandler g_{}({}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
        DeviceKind::Uart => format!(
            "static UartHandler g_{}({}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
        DeviceKind::I2c => format!(
            "static I2cHandler g_{}({}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
        DeviceKind::Can => format!(
            "static CanHandler g_{}({}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
        DeviceKind::Qspi => format!(
            "static QspiHandler g_{}({}, g_clock);\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
        DeviceKind::Rtc => format!(
            "static RtcHandler g_{}({});\n",
            slot.instance_name,
            detail(slot, "hal_handle").unwrap_or("nullptr")
        ),
    }
}
