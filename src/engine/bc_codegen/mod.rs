// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Code-generation back-end for the BC (host) project.
//!
//! The BC tree mirrors the structure of the RT tree (one tree per board /
//! host pair, both buildable independently) but the BC tree is for a desktop
//! host: it has a serial-port IFrameTransport, a std::chrono-backed IClock,
//! and a generated `RtClient` whose nested per-peripheral objects expose
//! blocking methods that return [`Status`].
//!
//! What this module emits:
//!   - Every file in [`BC_STATIC_FILES`] (shared protocol layer + host
//!     adapters).
//!   - `api_fingerprint.h`, the `command_ids.h` / `sub_addresses.h` enum
//!     headers, and the per-sub-address `api/` manifest set — byte-identical
//!     to the RT-side copies, so the generated `RtClient::verify_fingerprint()`
//!     can sanity-check that BC and RT were built from the same `.ioc`.
//!   - `rt_client.h` / `rt_client.cpp` containing one peripheral-client
//!     class per `DeviceKind` actually used by the IOC, plus a top-level
//!     `RtClient` whose constructor binds each nested member to its
//!     assigned sub-address.
//!   - `main.cpp` with `int main()` that opens the serial port, verifies the
//!     fingerprint, and runs one representative call per peripheral kind in the
//!     IOC. Serves as a copy-paste starting point. Its `USER CODE` regions are
//!     preserved across regeneration (see [`crate::engine::merge`]).
//!   - A portable `CMakeLists.txt` + `CMakePresets.json` (CMake + Ninja; works
//!     on Linux, macOS, and Windows via MSVC or MinGW), mirroring the RT side.
//!
//! The blocking façade is implemented entirely in the generated
//! `rt_client.cpp` — every method submits a `Request` to the on-target
//! [`BusController`] and then polls `bc.tick()` until the completion
//! callback fires. The BC's own per-frame `kFrameTimeoutMs` is what
//! normally terminates a stalled wait; an outer guard of
//! `8 * kFrameTimeoutMs` is added as a safety net.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::addresses::{DeviceKind, DeviceSlot, SlotMember};
use crate::engine::autogen_content::{StaticFile, BC_STATIC_FILES};
use crate::engine::commands;
use crate::engine::manifest::ManifestFile;
use crate::engine::transport_select::TransportChoice;


/// `EnumName::Member` for the catalog command of `kind` whose wire name is
/// `screaming_name` (e.g. `cmd_enum_expr(DeviceKind::GpioOutput, "SET")` →
/// `"GpioOutputCmd::Set"`). Used to emit typed opcodes in the client bodies.
fn cmd_enum_expr(kind: DeviceKind, screaming_name: &str) -> String {
    let set = commands::for_kind(kind);
    let cmd = set
        .commands
        .iter()
        .find(|c| c.name == screaming_name)
        .unwrap_or_else(|| panic!("command '{screaming_name}' missing from catalog for {}", set.kind));
    format!("{}::{}", set.cpp_enum, commands::to_pascal_case(&cmd.name))
}


/// As [`cmd_enum_expr`] but for the system command set (sub-address 1).
fn sys_cmd_enum_expr(screaming_name: &str) -> String {
    let set = commands::system();
    let cmd = set
        .commands
        .iter()
        .find(|c| c.name == screaming_name)
        .unwrap_or_else(|| panic!("system command '{screaming_name}' missing from catalog"));
    format!("{}::{}", set.cpp_enum, commands::to_pascal_case(&cmd.name))
}

/// Subdirectory under the user-supplied BC output root that holds every
/// generated file.
pub const BC_OUTPUT_DIR_NAME: &str = "exhibition_bc";


/// Generated entry-point source file — the one that contains `int main()`.
///
/// This is the only generated BC file that carries `USER CODE` regions and is
/// therefore written through [`write_text_merging`], which preserves the user's
/// hand-written code across regeneration.
pub const BC_MAIN_SOURCE_NAME: &str = "main.cpp";


/// CMake target (output binary) name. Kept distinct from
/// [`BC_MAIN_SOURCE_NAME`] so the binary can be named independently of the
/// source file.
const BC_TARGET_NAME: &str = "exhibition_bc";


/// Fallback baud rate baked into the generated `main.cpp` when the `.ioc` did
/// not specify a `<PERIPH>.BaudRate` for the chosen UART.
///
/// When this fallback is in effect the generated `main.cpp` advertises the fact
/// in a leading comment block, so users editing the file know to override the
/// value. The generated `main()` accepts a CLI argument to override the baud at
/// run time without regenerating.
const DEFAULT_BAUD: u32 = 115200;


/// Top-level codegen entry point for the BC project.
///
/// Creates `<out_root>/exhibition_bc/` if it does not exist, writes every
/// static template, then the generated `api_fingerprint.h`, the
/// `command_ids.h` / `sub_addresses.h` enum headers, the per-sub-address
/// `api/` manifest set, `rt_client.{h,cpp}`, `main.cpp`, and the CMake build
/// files (`CMakeLists.txt` + `CMakePresets.json`).
///
/// `main.cpp` is written through [`write_text_merging`] so any `USER CODE`
/// regions the user edited survive regeneration; every other file is rewritten
/// wholesale.
///
/// `transport` is consulted to bake the configured UART baud rate (when
/// the `.ioc` provided one) into the generated `main.cpp`. For
/// `TransportChoice::UsbCdc` the baud is treated as irrelevant — the fallback
/// constant is used silently because USB CDC ignores baud anyway.
pub fn write_bc_project(
    out_root: &Path,
    slots: &[DeviceSlot],
    transport: &TransportChoice,
    manifest_files: &[ManifestFile],
    fingerprint: u32,
) -> Result<(), String> {
    let project_root = out_root.join(BC_OUTPUT_DIR_NAME);
    fs::create_dir_all(&project_root)
        .map_err(|e| format!("Failed to create '{}': {}", project_root.display(), e))?;

    for f in BC_STATIC_FILES {
        write_static_file(&project_root, f)?;
    }

    // Decide which baud value to bake and whether to advertise the
    // "fallback used" note. The note is suppressed for USB CDC because the
    // INTEGRATION_GUIDE already explains that baud is ignored over USB.
    let (baud, emit_unknown_note) = match transport {
        TransportChoice::Uart { baud_rate: Some(b), .. } => (*b, false),
        TransportChoice::Uart { baud_rate: None, .. } => (DEFAULT_BAUD, true),
        TransportChoice::UsbCdc { .. } => (DEFAULT_BAUD, false),
    };

    write_text(&project_root.join("api_fingerprint.h"),
               &render_fingerprint_header(fingerprint))?;
    write_text(&project_root.join("command_ids.h"),
               &commands::render_command_ids_header())?;
    write_text(&project_root.join("service_hashes.h"),
               &commands::render_service_hashes_header())?;
    write_text(&project_root.join("sub_addresses.h"),
               &commands::render_sub_addresses_header(slots))?;

    let api_dir = project_root.join("api");
    fs::create_dir_all(&api_dir)
        .map_err(|e| format!("Failed to create '{}': {}", api_dir.display(), e))?;
    for f in manifest_files {
        write_manifest_preserving(&api_dir.join(&f.file_name), &f.content)?;
    }

    write_text(&project_root.join("rt_client.h"),
               &render_rt_client_header(slots))?;
    write_text(&project_root.join("rt_client.cpp"),
               &render_rt_client_source(slots))?;
    write_text_merging(&project_root.join(BC_MAIN_SOURCE_NAME),
               &render_example_sequence(slots, baud, emit_unknown_note))?;
    write_text(&project_root.join("CMakeLists.txt"), &render_cmakelists())?;
    write_text(&project_root.join("CMakePresets.json"), &render_cmake_presets())?;

    Ok(())
}


fn write_static_file(project_root: &Path, f: &StaticFile) -> Result<(), String> {
    let full = project_root.join(f.rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }
    write_text(&full, f.contents)
}


fn write_text(path: &PathBuf, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))
}


/// Write `generated` to `path`, but first splice in any `USER CODE` regions the
/// user wrote into the version already on disk (if one exists). For a path with
/// no existing file the generated text is written as-is. Use this for files
/// that carry `USER CODE` markers; plain [`write_text`] is fine for the rest.
fn write_text_merging(path: &PathBuf, generated: &str) -> Result<(), String> {
    let to_write = match fs::read_to_string(path) {
        Ok(existing) => crate::engine::merge::preserve_user_code(generated, &existing),
        Err(_) => generated.to_string(),
    };
    fs::write(path, &to_write)
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


/// Same shape as the RT-side header so both trees carry an identical
/// `kApiFingerprint` constant.
fn render_fingerprint_header(fp: u32) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef API_FINGERPRINT_H\n");
    s.push_str("#define API_FINGERPRINT_H\n\n");
    s.push_str("#include <stdint.h>\n\n");
    s.push_str("/// @brief 32-bit FNV-1a fingerprint of the generated API manifest.\n");
    s.push_str("/// @details Stamped here at generation time. The BC reads the\n");
    s.push_str("///          RT's runtime fingerprint via the system handler\n");
    s.push_str("///          (sub-address 1, opcode 0x01) and compares to confirm\n");
    s.push_str("///          both sides were built from the same .ioc.\n");
    s.push_str(&format!("constexpr uint32_t kApiFingerprint = 0x{:08X}u;\n\n", fp));
    s.push_str("#endif // API_FINGERPRINT_H\n");
    s
}


///////////////////////////////////////////////////////////////////////////////
// rt_client.h
///////////////////////////////////////////////////////////////////////////////

/// Sort-stable set of kinds present in `slots` (sub-address 1 is implicit
/// since the system handler lives on the top-level `RtClient`).
fn kinds_used(slots: &[DeviceSlot]) -> Vec<DeviceKind> {
    let mut kinds: Vec<DeviceKind> = slots.iter().map(|s| s.kind).collect();
    kinds.sort_by_key(|k| k.sort_key());
    kinds.dedup();
    kinds
}


fn render_rt_client_header(slots: &[DeviceSlot]) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("/// @file rt_client.h\n");
    s.push_str("/// @brief Typed BC-side client for talking to the RemoteTerminal.\n");
    s.push_str("/// @details Auto-generated by Exhibition from the device set. One\n");
    s.push_str("///          call per peripheral command; regenerated on every run, so\n");
    s.push_str("///          edit the .ioc and rerun rather than hand-editing this file.\n");
    s.push('\n');
    s.push_str("#ifndef RT_CLIENT_H\n");
    s.push_str("#define RT_CLIENT_H\n\n");
    s.push_str("#include <stdint.h>\n\n");
    s.push_str("#include \"api_fingerprint.h\"\n");
    s.push_str("#include \"command_ids.h\"\n");
    s.push_str("#include \"service_hashes.h\"\n");
    s.push_str("#include \"sub_addresses.h\"\n");
    s.push_str("#include \"communication/i_clock.h\"\n");
    s.push_str("#include \"communication/i_frame_transport.h\"\n");
    s.push_str("#include \"communication/scheduler.h\"\n");
    s.push_str("#include \"communication/sync.h\"\n\n");

    s.push_str("class Logger; // host/logger.h\n\n");

    s.push_str("/// @brief Result of one time-synchronization exchange, in time units.\n");
    s.push_str("/// @details `units` is the negative power of ten the values are\n");
    s.push_str("///          expressed in (milliseconds == 3). `path_delay_units` is this\n");
    s.push_str("///          exchange's measured mean one-way path delay (>= 0);\n");
    s.push_str("///          `offset_units` is the RT's recommended offset (the value the\n");
    s.push_str("///          RT smoothed and applied to its own clock): the BC clock minus\n");
    s.push_str("///          the RT clock, added to an RT timestamp to express it on the BC\n");
    s.push_str("///          clock (signed: positive when the BC clock leads the RT clock).\n");
    s.push_str("///          `reload_ticks` is the last SysTick reload programmed by sync_adjust()\n");
    s.push_str("///          (0 until the first sync_adjust() call).\n");
    s.push_str("struct SyncMeasurement {\n");
    s.push_str("    double   path_delay_units;\n");
    s.push_str("    double   offset_units;\n");
    s.push_str("    uint32_t reload_ticks;\n");
    s.push_str("    uint8_t  units;\n");
    s.push_str("};\n\n");

    s.push_str("/// @brief Result of one SYNC_ADJUST: the skew the RT averaged over the\n");
    s.push_str("///        preceding telemetry phase and the SysTick reload it then programmed.\n");
    s.push_str("/// @details `avg_skew_ppb` is the BC-vs-RT skew in parts-per-billion (positive\n");
    s.push_str("///          when the BC gains on the RT, i.e. the RT runs slow). `reload_ticks`\n");
    s.push_str("///          is the full tick count now programmed; it moves a fraction of the\n");
    s.push_str("///          way toward the rate that cancels the skew and stays within +/-10%\n");
    s.push_str("///          of nominal, so the clocks lock over several telemetry/adjust cycles.\n");
    s.push_str("struct SyncAdjustResult {\n");
    s.push_str("    int32_t  avg_skew_ppb;\n");
    s.push_str("    uint32_t reload_ticks;\n");
    s.push_str("};\n\n");

    s.push_str("/// @file rt_client.h\n");
    s.push_str("/// @brief Auto-generated host-side client for this RT's API.\n");
    s.push_str("/// @details\n");
    s.push_str("/// Every method is blocking: it submits a Request to the BusController\n");
    s.push_str("/// and polls bc.tick() until the completion callback fires or the\n");
    s.push_str("/// outer watchdog expires. Not thread-safe: use one RtClient per\n");
    s.push_str("/// thread, or add external locking.\n\n");

    s.push_str("/// @brief Translate a Status value into a short human-readable name.\n");
    s.push_str("/// @param s Any Status value.\n");
    s.push_str("/// @return A pointer to a static string (\"Ok\", \"Timeout\", ...).\n");
    s.push_str("///         Returns \"Unknown\" for unrecognized values.\n");
    s.push_str("char const* status_to_string(Status s);\n\n");

    s.push_str("/// @brief Translate a FaultDomain tag into a short human-readable name.\n");
    s.push_str("/// @param d Any FaultDomain value (byte 0 of a fault payload).\n");
    s.push_str("/// @return A pointer to a static string (\"Protocol\", \"Hal\", \"App\").\n");
    s.push_str("///         Returns \"Unknown\" for unrecognized values.\n");
    s.push_str("char const* fault_domain_to_string(FaultDomain d);\n\n");

    s.push_str("/// @brief Decode a (sub-address, opcode) pair to a human-readable\n");
    s.push_str("///        operation name for logging.\n");
    s.push_str("/// @param sub_address Routed sub-address of the command.\n");
    s.push_str("/// @param opcode First payload byte of the command.\n");
    s.push_str("/// @return A static string; \"UNKNOWN\" for unrecognized pairs.\n");
    s.push_str("char const* op_name(uint8_t sub_address, uint8_t opcode);\n\n");

    s.push_str("/// @brief Resolve a routed sub-address to its human-readable name\n");
    s.push_str("///        (the instance name from the manifest) for logging.\n");
    s.push_str("/// @param sub_address Routed sub-address.\n");
    s.push_str("/// @return A static string; \"UNKNOWN\" for unrecognized sub-addresses.\n");
    s.push_str("char const* sub_address_name(uint8_t sub_address);\n\n");

    for kind in kinds_used(slots) {
        s.push_str(&render_peripheral_class_decl(kind));
        s.push('\n');
    }

    s.push_str("/// @brief Host observer that logs every failed BC transaction attempt as a\n");
    s.push_str("///        WARNING, including attempts later recovered by the single automatic\n");
    s.push_str("///        retry (which would otherwise leave no trace but the fault counter).\n");
    s.push_str("/// @details Injected into the RtClient's BusController; writes through the\n");
    s.push_str("///          same Logger as the rest of the client.\n");
    s.push_str("class LoggerFaultObserver : public IBusFaultObserver {\n");
    s.push_str("    public:\n\n");
    s.push_str("    explicit LoggerFaultObserver(Logger& log) : m_log(log) {}\n\n");
    s.push_str("    void on_fault(uint8_t sub_addr, Status status, bool will_retry) override;\n\n");
    s.push_str("    private:\n\n");
    s.push_str("    Logger& m_log;\n");
    s.push_str("};\n\n");

    s.push_str("class RtClient {\n");
    s.push_str("    private:\n\n");
    s.push_str("    // Declared first so the per-peripheral nested members below can\n");
    s.push_str("    // bind to them in their initializer list. C++ initializes class\n");
    s.push_str("    // members in declaration order, not initializer-list order.\n");
    s.push_str("    // m_fault_observer precedes m_bc: the BusController is constructed\n");
    s.push_str("    // with a pointer to it, so the observer must already exist.\n");
    s.push_str("    LoggerFaultObserver m_fault_observer;\n");
    s.push_str("    BusController m_bc;\n");
    s.push_str("    IClock&       m_clock;\n");
    s.push_str("    Logger&       m_log;\n\n");

    s.push_str("    // Latest time-sync state (see sync_once). `m_offset` is the RT's\n");
    s.push_str("    // recommended offset, smoothed and applied on the RT side and stored\n");
    s.push_str("    // here as-is; `m_path_delay` is the latest measured path delay.\n");
    s.push_str("    bool     m_have_sync    = false;\n");
    s.push_str("    double   m_offset       = 0.0;\n");
    s.push_str("    double   m_path_delay   = 0.0;\n");
    s.push_str("    uint32_t m_reload_ticks = 0;\n");
    s.push_str("    uint8_t  m_sync_units   = static_cast<uint8_t>(TimeUnits::Millisecond);\n\n");

    s.push_str("    public:\n\n");
    s.push_str("    /// @brief Construct; binds to the given transport, clock, and\n");
    s.push_str("    ///        logger. All three references must outlive this RtClient.\n");
    s.push_str("    RtClient(IFrameTransport& transport, IClock& clock, Logger& log);\n\n");

    s.push_str("    /// @brief System handler: read the RT's 32-bit API fingerprint.\n");
    s.push_str("    Status get_api_fingerprint(uint32_t& fingerprint_out);\n\n");
    s.push_str("    /// @brief Universal command: read the per-service API hash of @p sub_address\n");
    s.push_str("    ///        (opcode 0x00, GET_SERVICE_HASH). Works for any registered\n");
    s.push_str("    ///        sub-address, including the system handler.\n");
    s.push_str("    Status get_service_hash(uint8_t sub_address, uint32_t& hash_out);\n\n");
    s.push_str("    /// @brief System handler: read the count of registered devices.\n");
    s.push_str("    Status get_device_count(uint16_t& count_out);\n\n");
    s.push_str("    /// @brief System handler: read the RT's HAL millisecond uptime\n");
    s.push_str("    ///        counter (HAL_GetTick).\n");
    s.push_str("    Status get_sys_tick(uint32_t& ticks_out);\n\n");
    s.push_str("    /// @brief System handler: read the ARM core device ID and\n");
    s.push_str("    ///        revision ID (HAL_GetDEVID / HAL_GetREVID).\n");
    s.push_str("    Status get_core_device_id(uint32_t& devid_out, uint32_t& revid_out);\n\n");
    s.push_str("    /// @brief System handler: read the 96-bit CPU unique ID words 0..2\n");
    s.push_str("    ///        (HAL_GetUIDw0/1/2).\n");
    s.push_str("    Status get_cpu_id(uint32_t& uid0_out, uint32_t& uid1_out, uint32_t& uid2_out);\n\n");
    s.push_str("    /// @brief System handler: read the RT's running count of inbound messages\n");
    s.push_str("    ///        that failed due to a timeout or a CRC / malformed-frame error.\n");
    s.push_str("    Status get_message_fault_counter(uint32_t& count_out);\n\n");
    s.push_str("    /// @brief System handler: trigger a soft reset of the RT.\n");
    s.push_str("    ///        The RT sends a write-ack then resets via HAL_NVIC_SystemReset.\n");
    s.push_str("    Status soft_reset();\n\n");
    s.push_str("    /// @brief System handler: read @p len bytes of RT memory starting at\n");
    s.push_str("    ///        @p address into @p data_out. @p len must not exceed the RT's\n");
    s.push_str("    ///        max response frame (kMaxPayloadBytes); the RT also rejects an\n");
    s.push_str("    ///        address+length that wraps past the 32-bit range.\n");
    s.push_str("    Status read_memory(uint32_t address, uint16_t len, uint8_t* data_out);\n\n");
    s.push_str("    /// @brief Convenience: read fingerprint and compare to the\n");
    s.push_str("    ///        compile-time kApiFingerprint. Returns Status::Ok\n");
    s.push_str("    ///        on a match, ::HandlerError on mismatch, or the\n");
    s.push_str("    ///        original error if the read itself failed.\n");
    s.push_str("    Status verify_fingerprint();\n\n");
    s.push_str("    /// @brief Read each registered sub-address's per-service hash and compare\n");
    s.push_str("    ///        it to the expected compile-time kServiceHash<Kind> constant.\n");
    s.push_str("    /// @details Use after verify_fingerprint() fails to localize which device\n");
    s.push_str("    ///          handler(s) diverged: each mismatch is logged. Returns Status::Ok\n");
    s.push_str("    ///          only if every sub-address matched; otherwise ::HandlerError\n");
    s.push_str("    ///          (or the first read error encountered).\n");
    s.push_str("    Status verify_service_hashes();\n\n");

    s.push_str("    /// @brief Run one Cristian-style time-synchronization exchange with the\n");
    s.push_str("    ///        RT (SYNC_PROBE / SYNC_FINAL on sub-address 1).\n");
    s.push_str("    /// @details\n");
    s.push_str("    /// On success the RT returns its recommended clock offset -- the value\n");
    s.push_str("    /// it smoothed and applied to its own clock -- which is stored as-is in\n");
    s.push_str("    /// clock_offset_units() (no BC-side filtering). The RT seeds its filter\n");
    s.push_str("    /// on the first exchange and settles it over later ones, so call this\n");
    s.push_str("    /// repeatedly to let the RT average out bus jitter. path_delay_units()\n");
    s.push_str("    /// is the latest measured value.\n");
    s.push_str("    /// @param raw_out Optional; receives this exchange's reported values\n");
    s.push_str("    ///        (recommended offset, measured path delay, units).\n");
    s.push_str("    /// @return Status::Ok on a full exchange, else the first failing step's status.\n");
    s.push_str("    Status sync_once(SyncMeasurement* raw_out = nullptr);\n\n");
    s.push_str("    /// @brief True once at least one sync_once() has succeeded.\n");
    s.push_str("    bool has_sync() const { return m_have_sync; }\n\n");
    s.push_str("    /// @brief The RT's recommended clock offset (BC clock minus RT clock),\n");
    s.push_str("    ///        in time units (signed) -- smoothed and applied on the RT side.\n");
    s.push_str("    ///        Add to an RT timestamp to express it on the BC clock.\n");
    s.push_str("    ///        Meaningful only once has_sync() is true.\n");
    s.push_str("    double clock_offset_units() const { return m_offset; }\n\n");
    s.push_str("    /// @brief Latest measured mean one-way path delay, in time units.\n");
    s.push_str("    ///        Meaningful only once has_sync() is true.\n");
    s.push_str("    double path_delay_units() const { return m_path_delay; }\n\n");
    s.push_str("    /// @brief SysTick reload (full tick count) the RT last programmed to\n");
    s.push_str("    ///        discipline its tick rate onto the BC. 0 until has_sync().\n");
    s.push_str("    uint32_t reload_ticks() const { return m_reload_ticks; }\n\n");
    s.push_str("    /// @brief Time-unit code the sync estimates are expressed in\n");
    s.push_str("    ///        (negative power of ten; milliseconds == 3).\n");
    s.push_str("    uint8_t sync_units() const { return m_sync_units; }\n\n");
    s.push_str("    /// @brief Apply one smoothed SysTick-reload adjustment on the RT from the\n");
    s.push_str("    ///        skew averaged over the preceding telemetry phase (SYNC_ADJUST on\n");
    s.push_str("    ///        sub-address 1).\n");
    s.push_str("    /// @details Run a batch of sync_once() exchanges (the telemetry phase) to let\n");
    s.push_str("    /// the RT's estimator average the BC-vs-RT skew, then call this once to apply a\n");
    s.push_str("    /// single bounded, partial-step reload change toward the rate that cancels it.\n");
    s.push_str("    /// Repeat (telemetry phase, then sync_adjust) -- the clocks lock over several\n");
    s.push_str("    /// cycles because each step is smoothed and clamped to +/-10% of nominal.\n");
    s.push_str("    /// @param out Optional; receives the averaged skew used and the reload now\n");
    s.push_str("    ///        programmed (also reflected in reload_ticks()).\n");
    s.push_str("    /// @return Status::Ok on success, else the transaction's failing status.\n");
    s.push_str("    Status sync_adjust(SyncAdjustResult* out = nullptr);\n\n");
    s.push_str("    /// @brief Local read: running count of bus faults the BC itself has\n");
    s.push_str("    ///        detected (timeouts + CRC / malformed-frame errors on the BC\n");
    s.push_str("    ///        side). Does not send any bus transaction.\n");
    s.push_str("    uint32_t bc_fault_count() const { return m_bc.fault_count(); }\n\n");

    for slot in slots {
        for m in &slot.members {
            s.push_str(&format!(
                "    /// @brief Sub-address {} ({}).\n",
                slot.sub_address, m.label
            ));
            s.push_str(&format!(
                "    {} {};\n\n",
                client_class_name(slot.kind),
                m.instance_name
            ));
        }
    }

    s.push_str("};\n\n");

    s.push_str("#endif // RT_CLIENT_H\n");
    s
}


fn client_class_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::GpioOutput => "GpioOutputClient",
        DeviceKind::GpioInput  => "GpioInputClient",
        DeviceKind::Adc        => "AdcClient",
        DeviceKind::Dac        => "DacClient",
        DeviceKind::Pwm        => "PwmClient",
        DeviceKind::Qspi       => "QspiClient",
        DeviceKind::Rtc        => "RtcClient",
        DeviceKind::Spi        => "SpiClient",
        DeviceKind::Uart       => "UartClient",
        DeviceKind::I2c        => "I2cClient",
        DeviceKind::Can        => "CanClient",
    }
}


fn render_peripheral_class_decl(kind: DeviceKind) -> String {
    let name = client_class_name(kind);
    let needs_pin_idx = kind_carries_pin_index(kind);
    let mut s = String::new();
    s.push_str(&format!("class {} {{\n", name));
    s.push_str("    public:\n\n");
    if needs_pin_idx {
        s.push_str(&format!(
            "    {}(BusController& bc, IClock& clock, Logger& log, SubAddress sub_address, uint8_t pin_index);\n\n",
            name
        ));
    } else {
        s.push_str(&format!(
            "    {}(BusController& bc, IClock& clock, Logger& log, SubAddress sub_address);\n\n",
            name
        ));
    }
    for sig in method_signatures(kind) {
        s.push_str(&format!("    {};\n", sig));
    }
    s.push('\n');
    s.push_str("    private:\n\n");
    s.push_str("    BusController& m_bc;\n");
    s.push_str("    IClock&        m_clock;\n");
    s.push_str("    Logger&        m_log;\n");
    s.push_str("    SubAddress     m_sa;\n");
    if needs_pin_idx {
        s.push_str("    uint8_t        m_pin_idx;\n");
    }
    s.push_str("};\n");
    s
}


/// True when the client class for `kind` encodes a per-pin index into
/// every outgoing command — currently the two GPIO kinds, whose bank
/// handlers expect `[opcode, pin_idx]` payloads.
fn kind_carries_pin_index(kind: DeviceKind) -> bool {
    matches!(kind, DeviceKind::GpioOutput | DeviceKind::GpioInput)
}


/// Public method signatures (declaration form) for one peripheral kind.
fn method_signatures(kind: DeviceKind) -> Vec<&'static str> {
    match kind {
        DeviceKind::GpioOutput => vec![
            "Status set()",
            "Status clear()",
        ],
        DeviceKind::GpioInput => vec![
            "Status read(bool& value_out)",
        ],
        DeviceKind::Adc => vec![
            "Status sample(uint16_t& counts_out, uint32_t& timestamp_ms_out)",
        ],
        DeviceKind::Dac => vec![
            "Status set_counts(uint16_t counts)",
        ],
        DeviceKind::Pwm => vec![
            "Status set_duty(uint16_t duty_hundredths)",
        ],
        DeviceKind::Spi => vec![
            "Status tx(uint8_t const* data, uint16_t len)",
            "Status rx(uint16_t len, uint8_t* data_out)",
        ],
        DeviceKind::Uart => vec![
            "Status tx(uint8_t const* data, uint16_t len)",
            "Status rx(uint16_t len, uint8_t* data_out)",
        ],
        DeviceKind::I2c => vec![
            "Status write_reg(uint8_t addr7, uint8_t reg, uint8_t const* data, uint8_t len)",
            "Status read_reg(uint8_t addr7, uint8_t reg, uint8_t len, uint8_t* data_out)",
        ],
        DeviceKind::Can => vec![
            "Status tx_frame(uint32_t id, uint8_t dlc, const uint8_t data[8])",
            "Status rx_frame(uint32_t& id_out, uint8_t& dlc_out, uint8_t data_out[8])",
        ],
        DeviceKind::Qspi => vec![
            "Status read(uint8_t instr, uint32_t address, uint8_t dummy_cycles, uint8_t data_lines, uint16_t len, uint8_t* out)",
            "Status program(uint8_t instr, uint32_t address, uint8_t data_lines, uint8_t const* data, uint16_t len)",
            "Status erase(uint8_t instr, uint32_t address)",
            "Status read_status(uint8_t instr, uint8_t& status_out)",
            "Status write_enable(uint8_t instr)",
        ],
        DeviceKind::Rtc => vec![
            "Status set_datetime(uint8_t year_offset, uint8_t month, uint8_t day, uint8_t weekday, uint8_t hours, uint8_t minutes, uint8_t seconds)",
            "Status get_datetime(uint8_t& year_offset_out, uint8_t& month_out, uint8_t& day_out, uint8_t& weekday_out, uint8_t& hours_out, uint8_t& minutes_out, uint8_t& seconds_out)",
        ],
    }
}


/// Opcode → wire name pairs for one command set, sourced from the catalog so
/// the logging names cannot drift from the generated opcodes.
fn opcode_names(set: &commands::DeviceCommandSet) -> Vec<(u8, &str)> {
    set.commands.iter().map(|c| (c.opcode, c.name.as_str())).collect()
}


/// Render one `case <opcode>: return "<name>";` block plus a default.
fn render_opcode_switch(ops: &[(u8, &str)]) -> String {
    let mut s = String::new();
    s.push_str("            switch (opcode) {\n");
    for (op, name) in ops {
        s.push_str(&format!(
            "                case 0x{:02X}: return \"{}\";\n", op, name
        ));
    }
    s.push_str("                default: return \"UNKNOWN\";\n");
    s.push_str("            }\n");
    s
}


/// Emit `op_name()` mapping (sub_address, opcode) to a name. Sub-address 1 is
/// the implicit system handler; every other case comes from a `slots` entry.
/// The names come from the catalog (untrusted opcode byte, so the switch keeps
/// a `uint8_t` argument rather than the typed enum).
fn render_op_name(slots: &[DeviceSlot]) -> String {
    let mut s = String::new();
    s.push_str("char const* op_name(uint8_t sub_address, uint8_t opcode)\n{\n");
    s.push_str("    switch (sub_address) {\n");
    s.push_str("        case 1: // System\n");
    s.push_str(&render_opcode_switch(&opcode_names(commands::system())));
    for slot in slots {
        s.push_str(&format!(
            "        case {}: // {}\n", slot.sub_address, slot.kind.sort_key()
        ));
        s.push_str(&render_opcode_switch(&opcode_names(commands::for_kind(slot.kind))));
    }
    s.push_str("        default: return \"UNKNOWN\";\n");
    s.push_str("    }\n");
    s.push_str("}\n\n");
    s
}


/// Emit `sub_address_name()` mapping a routed sub-address to its instance name
/// for logging. Sub-address 1 is the implicit system handler; every other case
/// comes from a `slots` entry.
fn render_sub_address_name(slots: &[DeviceSlot]) -> String {
    let mut s = String::new();
    s.push_str("char const* sub_address_name(uint8_t sub_address)\n{\n");
    s.push_str("    switch (sub_address) {\n");
    s.push_str("        case 1: return \"system\";\n");
    for slot in slots {
        s.push_str(&format!(
            "        case {}: return \"{}\";\n",
            slot.sub_address, slot.instance_name
        ));
    }
    s.push_str("        default: return \"UNKNOWN\";\n");
    s.push_str("    }\n");
    s.push_str("}\n\n");
    s
}


///////////////////////////////////////////////////////////////////////////////
// rt_client.cpp
///////////////////////////////////////////////////////////////////////////////

fn render_rt_client_source(slots: &[DeviceSlot]) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("/// @file rt_client.cpp\n");
    s.push_str("/// @brief Implementation of the typed BC-side RemoteTerminal client.\n");
    s.push_str("/// @details Auto-generated by Exhibition from the device set.\n");
    s.push_str("///          Regenerated on every run; edit the .ioc and rerun rather\n");
    s.push_str("///          than hand-editing this file.\n");
    s.push('\n');
    s.push_str("#include \"rt_client.h\"\n\n");
    s.push_str("#include <cstdio>\n");
    s.push_str("#include <cstring>\n\n");
    s.push_str("#include \"communication/message_header.h\"\n");
    s.push_str("#include \"host/logger.h\"\n\n");

    s.push_str(BLOCKING_HELPER);

    s.push_str("char const* status_to_string(Status s)\n{\n");
    s.push_str("    switch (s) {\n");
    s.push_str("        case Status::Ok:                 return \"Ok\";\n");
    s.push_str("        case Status::QueueFull:          return \"QueueFull\";\n");
    s.push_str("        case Status::InvalidSubAddress:  return \"InvalidSubAddress\";\n");
    s.push_str("        case Status::PayloadTooLarge:    return \"PayloadTooLarge\";\n");
    s.push_str("        case Status::Timeout:            return \"Timeout\";\n");
    s.push_str("        case Status::RemoteFault:        return \"RemoteFault\";\n");
    s.push_str("        case Status::CrcFailHeader:      return \"CrcFailHeader\";\n");
    s.push_str("        case Status::CrcFailPayload:     return \"CrcFailPayload\";\n");
    s.push_str("        case Status::TransportError:     return \"TransportError\";\n");
    s.push_str("        case Status::HandlerError:       return \"HandlerError\";\n");
    s.push_str("    }\n");
    s.push_str("    return \"Unknown\";\n");
    s.push_str("}\n\n");

    s.push_str("char const* fault_domain_to_string(FaultDomain d)\n{\n");
    s.push_str("    switch (d) {\n");
    s.push_str("        case FaultDomain::Protocol: return \"Protocol\";\n");
    s.push_str("        case FaultDomain::Hal:      return \"Hal\";\n");
    s.push_str("        case FaultDomain::App:      return \"App\";\n");
    s.push_str("    }\n");
    s.push_str("    return \"Unknown\";\n");
    s.push_str("}\n\n");

    s.push_str(&render_op_name(slots));
    s.push_str(&render_sub_address_name(slots));

    // LoggerFaultObserver: turn each retried BC attempt into a WARN line.
    s.push_str("void LoggerFaultObserver::on_fault(uint8_t sub_addr, Status status, bool will_retry)\n");
    s.push_str("{\n");
    s.push_str("    // A terminal failure is already reported as an ERROR by the operation's\n");
    s.push_str("    // completion logging, so only surface the recoverable attempts (the ones\n");
    s.push_str("    // the BC is about to retry) here -- those would otherwise leave no trace.\n");
    s.push_str("    if (!will_retry) {\n");
    s.push_str("        return;\n");
    s.push_str("    }\n");
    s.push_str("    char buf[128];\n");
    s.push_str("    std::snprintf(buf, sizeof(buf), \"sa=%s attempt failed: %s -- retrying\",\n");
    s.push_str("                  sub_address_name(sub_addr),\n");
    s.push_str("                  status_to_string(status));\n");
    s.push_str("    m_log.line(\"WARN\", buf);\n");
    s.push_str("}\n\n");

    // Per-kind class implementations (one set per kind used).
    for kind in kinds_used(slots) {
        s.push_str(&render_peripheral_class_impl(kind));
        s.push('\n');
    }

    // RtClient implementation.
    s.push_str("RtClient::RtClient(IFrameTransport& transport, IClock& clock, Logger& log)\n");
    s.push_str("    : m_fault_observer(log)\n");
    s.push_str("    , m_bc(transport, clock, &m_fault_observer)\n");
    s.push_str("    , m_clock(clock)\n");
    s.push_str("    , m_log(log)\n");
    for slot in slots {
        let sa_member = commands::to_pascal_case(&slot.instance_name);
        for m in &slot.members {
            match m.pin_index {
                Some(pin) => s.push_str(&format!(
                    "    , {}(m_bc, m_clock, m_log, SubAddress::{}, {})\n",
                    m.instance_name, sa_member, pin
                )),
                None => s.push_str(&format!(
                    "    , {}(m_bc, m_clock, m_log, SubAddress::{})\n",
                    m.instance_name, sa_member
                )),
            }
        }
    }
    s.push_str("{\n");
    s.push_str("}\n\n");

    s.push_str(&render_system_handler_impl(slots));
    s.push_str(&render_sync_impl());

    s
}


/// The blocking façade core — included verbatim once per rt_client.cpp.
const BLOCKING_HELPER: &str = r#"namespace {

struct BlockingCtx {
    Status   status;
    bool     done;
    uint8_t* user_out_buf;
    uint32_t user_out_cap;
    uint32_t* user_out_len_opt;
    bool     has_remote;          ///< True if the RT fault frame carried a payload.
    uint8_t  remote_domain;       ///< FaultDomain tag (byte 0 of the fault payload).
    uint8_t  remote_code;         ///< Reason code within that domain (byte 1).
};


void blocking_completion(Status s,
                         const MessageHeader& /*resp_header*/,
                         uint8_t const* payload,
                         uint32_t len,
                         void* user)
{
    BlockingCtx* ctx = static_cast<BlockingCtx*>(user);
    ctx->status = s;
    // RT fault frames are self-describing: byte 0 is a FaultDomain tag and
    // byte 1 the reason code within that domain. Capture both so run_one can
    // decode the remote reason with the right table (Status for Protocol).
    if (s == Status::RemoteFault && payload != nullptr && len >= 1u) {
        ctx->remote_domain = payload[0];
        ctx->remote_code   = (len >= 2u) ? payload[1] : 0u;
        ctx->has_remote    = true;
    }
    if (ctx->user_out_buf != nullptr && payload != nullptr && len > 0u) {
        const uint32_t n = (len < ctx->user_out_cap) ? len : ctx->user_out_cap;
        std::memcpy(ctx->user_out_buf, payload, n);
    }
    if (ctx->user_out_len_opt != nullptr) {
        *ctx->user_out_len_opt = len;
    }
    ctx->done = true;
}


/// @brief Log an operation's outcome: INFO on success, ERROR otherwise.
/// @details When the RT reports a fault its payload names a FaultDomain (byte 0)
///          and a reason code (byte 1). Pass them via has_remote/remote_domain/
///          remote_code to append the decoded remote reason. A Protocol-domain
///          code is rendered as a Status name ("-> RemoteFault (remote:
///          PayloadTooLarge)"); any other domain is rendered as a domain name
///          plus a hex code ("-> RemoteFault (remote: App 0xCA)").
void log_op(Logger& log, const Request& req, Status status,
            bool has_remote = false, uint8_t remote_domain = 0u,
            uint8_t remote_code = 0u, char const* note = nullptr)
{
    uint8_t const* cmd = (req.cmd_len > kInlinePayloadBytes) ? req.cmd_ext
                                                             : req.cmd_inline;
    const uint8_t op = (req.cmd_len > 0u && cmd != nullptr) ? cmd[0] : 0u;
    char buf[160];
    int n;
    if (has_remote) {
        char remote[48];
        if (remote_domain == static_cast<uint8_t>(FaultDomain::Protocol)) {
            std::snprintf(remote, sizeof(remote), "%s",
                          status_to_string(static_cast<Status>(remote_code)));
        } else {
            std::snprintf(remote, sizeof(remote), "%s 0x%02X",
                          fault_domain_to_string(
                              static_cast<FaultDomain>(remote_domain)),
                          static_cast<unsigned>(remote_code));
        }
        n = std::snprintf(buf, sizeof(buf), "op sa=%s %s -> %s (remote: %s)",
                          sub_address_name(req.sub_address),
                          op_name(req.sub_address, op),
                          status_to_string(status),
                          remote);
    } else {
        n = std::snprintf(buf, sizeof(buf), "op sa=%s %s -> %s",
                          sub_address_name(req.sub_address),
                          op_name(req.sub_address, op),
                          status_to_string(status));
    }
    // Optional trailing note (e.g. a host-stall attribution on a timeout).
    if (note != nullptr && n > 0 && static_cast<size_t>(n) < sizeof(buf)) {
        std::snprintf(buf + n, sizeof(buf) - static_cast<size_t>(n), " %s", note);
    }
    log.line(status == Status::Ok ? "INFO" : "ERROR", buf);
}


/// @brief Submit `req` and spin tick() until the completion callback fires.
/// @details
/// The BC self-enforces kFrameTimeoutMs per wire frame; the outer
/// 8x guard is a safety net in case the state machine itself stalls. The
/// final Status is logged on every exit path so detected errors (Timeout,
/// CrcFail*, QueueFull, ...) land in the log even though the wire-level
/// LoggingFrameTransport only sees raw bytes.
Status run_one(BusController& bc,
                  IClock& clock,
                  const Request& req,
                  uint8_t* out_buf,
                  uint32_t out_cap,
                  uint32_t* out_len_opt,
                  Logger& log)
{
    BlockingCtx ctx;
    ctx.status            = Status::Ok;
    ctx.done              = false;
    ctx.user_out_buf      = out_buf;
    ctx.user_out_cap      = out_cap;
    ctx.user_out_len_opt  = out_len_opt;
    ctx.has_remote        = false;
    ctx.remote_domain     = 0u;
    ctx.remote_code       = 0u;

    Status submit_status = Status::Ok;
    if (!bc.submit(req, &blocking_completion, &ctx, submit_status)) {
        log_op(log, req, submit_status);
        return submit_status;
    }

    const uint32_t start = clock.now_ms();
    constexpr uint32_t kOuterTimeoutMs = 8u * kFrameTimeoutMs;
    uint32_t max_tick_ms = 0u;
    while (!ctx.done) {
        const uint32_t t0 = clock.now_ms();
        bc.tick();
        const uint32_t tick_ms = clock.now_ms() - t0;
        if (tick_ms > max_tick_ms) {
            max_tick_ms = tick_ms;
        }
        if ((clock.now_ms() - start) > kOuterTimeoutMs) {
            // tick() never blocks on the transport (non-blocking I/O) and the
            // bus self-times-out every kFrameTimeoutMs, so any single tick that
            // ran longer than a whole frame budget means the host -- not the
            // bus -- stalled the poll loop (a blocked syscall, scheduler
            // preemption, ...). Call that out so the log is unambiguous.
            if (max_tick_ms > kFrameTimeoutMs) {
                char note[64];
                std::snprintf(note, sizeof(note),
                              "(host stall %u ms -- not a bus fault)",
                              static_cast<unsigned>(max_tick_ms));
                log_op(log, req, Status::Timeout, false, 0u, 0u, note);
            } else {
                log_op(log, req, Status::Timeout);
            }
            return Status::Timeout;
        }
    }
    log_op(log, req, ctx.status, ctx.has_remote, ctx.remote_domain,
           ctx.remote_code);
    return ctx.status;
}


/// @brief Stage `actual_len` cmd bytes into req.cmd_inline, zero-pad up to
///        the wire slot, and stamp req.cmd_len / req.cmd_size accordingly.
/// @pre `actual_len <= kInlinePayloadBytes`.
void stage_inline_cmd(Request& req, uint32_t actual_len)
{
    const PayloadSizes ps  = bytes_to_payload_size(actual_len);
    const uint32_t slot    = payload_size_to_bytes(ps);
    if (slot > actual_len) {
        std::memset(req.cmd_inline + actual_len, 0, slot - actual_len);
    }
    req.cmd_len  = slot;
    req.cmd_size = ps;
    req.cmd_ext  = nullptr;
}

} // namespace


"#;


/// System handler methods, appended to the end of rt_client.cpp. The
/// sub-address and opcodes are sourced from the catalog so they stay in sync
/// with the generated `SubAddress` / `SystemCmd` enums.
fn render_system_handler_impl(slots: &[DeviceSlot]) -> String {
    let sa = "static_cast<uint8_t>(SubAddress::System)";
    let fp_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_FINGERPRINT"));
    let sh_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_SERVICE_HASH"));
    let dc_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_DEVICE_COUNT"));
    let st_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_SYS_TICK"));
    let did_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_CORE_DEVICE_ID"));
    let uid_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_CPU_ID"));
    let mfc_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("GET_MESSAGE_FAULT_COUNTER"));
    let sr_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("SOFT_RESET"));
    let rm_op  = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("READ_MEMORY"));
    let mut s = String::new();
    s.push_str("Status RtClient::get_api_fingerprint(uint32_t& fingerprint_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", fp_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 4u) {\n");
    s.push_str("        fingerprint_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::get_service_hash(uint8_t sub_address, uint32_t& hash_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address  = sub_address;\n");
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", sh_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 4u) {\n");
    s.push_str("        hash_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::get_device_count(uint16_t& count_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", dc_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 2u) {\n");
    s.push_str("        count_out = static_cast<uint16_t>(resp[0])\n");
    s.push_str("                  | (static_cast<uint16_t>(resp[1]) << 8);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::get_sys_tick(uint32_t& ticks_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", st_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 4u) {\n");
    s.push_str("        ticks_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::get_core_device_id(uint32_t& devid_out, uint32_t& revid_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", did_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 8u) {\n");
    s.push_str("        devid_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("        revid_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[4]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[5]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[6]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[7]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::get_cpu_id(uint32_t& uid0_out, uint32_t& uid1_out, uint32_t& uid2_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", uid_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 12u) {\n");
    s.push_str("        uid0_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("        uid1_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[4]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[5]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[6]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[7]) << 24);\n");
    s.push_str("        uid2_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[8]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[9])  <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[10]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[11]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::verify_fingerprint()\n");
    s.push_str("{\n");
    s.push_str("    uint32_t got = 0u;\n");
    s.push_str("    const Status s = get_api_fingerprint(got);\n");
    s.push_str("    if (s != Status::Ok) {\n");
    s.push_str("        return s;\n");
    s.push_str("    }\n");
    s.push_str("    return (got == kApiFingerprint) ? Status::Ok : Status::HandlerError;\n");
    s.push_str("}\n");
    s.push_str("\n\n");

    // verify_service_hashes: read each registered sub-address's per-service hash
    // (GET_SERVICE_HASH, opcode 0x00) and compare to the expected compile-time
    // kServiceHash<Kind> constant. System handler first, then each slot in order.
    s.push_str("Status RtClient::verify_service_hashes()\n");
    s.push_str("{\n");
    s.push_str("    Status   result = Status::Ok;\n");
    s.push_str("    uint32_t got    = 0u;\n");
    s.push_str("    Status   s;\n");
    s.push('\n');
    let mut checks: Vec<(String, String)> = Vec::with_capacity(slots.len() + 1);
    checks.push((
        "static_cast<uint8_t>(SubAddress::System)".to_string(),
        commands::service_hash_const_name(commands::system()),
    ));
    for slot in slots {
        checks.push((
            format!(
                "static_cast<uint8_t>(SubAddress::{})",
                commands::to_pascal_case(&slot.instance_name)
            ),
            commands::service_hash_const_name(commands::for_kind(slot.kind)),
        ));
    }
    for (sa_expr, const_name) in &checks {
        s.push_str(&format!("    s = get_service_hash({}, got);\n", sa_expr));
        s.push_str("    if (s != Status::Ok) {\n");
        s.push_str("        return s;\n");
        s.push_str("    }\n");
        s.push_str(&format!("    if (got != {}) {{\n", const_name));
        s.push_str("        char note[160];\n");
        s.push_str("        std::snprintf(note, sizeof(note),\n");
        s.push_str("                      \"service-hash mismatch sa=%u (%s): got=0x%08X expected=0x%08X\",\n");
        s.push_str(&format!("                      static_cast<unsigned>({}),\n", sa_expr));
        s.push_str(&format!("                      sub_address_name({}),\n", sa_expr));
        s.push_str("                      static_cast<unsigned>(got),\n");
        s.push_str(&format!("                      static_cast<unsigned>({}));\n", const_name));
        s.push_str("        m_log.line(\"ERROR\", note);\n");
        s.push_str("        result = Status::HandlerError;\n");
        s.push_str("    }\n");
    }
    s.push_str("    return result;\n");
    s.push_str("}\n");
    s.push_str("\n\n");

    s.push_str("Status RtClient::get_message_fault_counter(uint32_t& count_out)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Low;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", mfc_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 4u) {\n");
    s.push_str("        count_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[0]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[1]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[2]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::soft_reset()\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address  = {};\n", sa));
    s.push_str("    req.priority     = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read      = false;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", sr_op));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s.push_str("\n\n");
    s.push_str("Status RtClient::read_memory(uint32_t address, uint16_t len, uint8_t* data_out)\n");
    s.push_str("{\n");
    s.push_str("    if (data_out == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    if (static_cast<uint32_t>(len) > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str(&format!("    req.sub_address = {};\n", sa));
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", rm_op));
    s.push_str("    req.cmd_inline[1] = static_cast<uint8_t>( address        & 0xFFu);\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((address >>  8)  & 0xFFu);\n");
    s.push_str("    req.cmd_inline[3] = static_cast<uint8_t>((address >> 16)  & 0xFFu);\n");
    s.push_str("    req.cmd_inline[4] = static_cast<uint8_t>((address >> 24)  & 0xFFu);\n");
    s.push_str("    req.cmd_inline[5] = static_cast<uint8_t>( len             & 0xFFu);\n");
    s.push_str("    req.cmd_inline[6] = static_cast<uint8_t>((len >> 8)       & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 7u);\n");
    s.push('\n');
    s.push_str("    const PayloadSizes resp_ps = bytes_to_payload_size(len);\n");
    s.push_str("    const uint32_t resp_slot   = payload_size_to_bytes(resp_ps);\n");
    s.push_str("    req.resp_buf_max = resp_slot;\n");
    s.push('\n');
    s.push_str("    uint8_t inline_resp[kInlinePayloadBytes] = {0};\n");
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    const bool use_ext = (resp_slot > kInlinePayloadBytes);\n");
    s.push_str("    req.resp_ext = use_ext ? scratch : nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t* sink = use_ext ? scratch : inline_resp;\n");
    s.push_str("    uint32_t cap  = use_ext ? sizeof(scratch) : sizeof(inline_resp);\n");
    s.push_str("    uint32_t got  = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, sink, cap, &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && len > 0u) {\n");
    s.push_str("        const uint32_t n = (got < len) ? got : len;\n");
    s.push_str("        std::memcpy(data_out, sink, n);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


/// Time-synchronization façade, appended to rt_client.cpp. Reuses the
/// file-local `run_one` / `stage_inline_cmd` helpers from `BLOCKING_HELPER`.
/// The opcodes come from the catalog so they track the generated `SystemCmd`
/// enum; the formulas and fixed-point decode live in the shared `sync.{h,cpp}`.
fn render_sync_impl() -> String {
    let probe_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("SYNC_PROBE"));
    let fin_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("SYNC_FINAL"));
    let adjust_op = format!("static_cast<uint8_t>({})", sys_cmd_enum_expr("SYNC_ADJUST"));
    let mut s = String::new();

    // File-local helper that builds and runs one sync command.
    s.push_str("namespace {\n\n");
    s.push_str("/// @brief Build and run one [opcode, units, uint32 value] sync command\n");
    s.push_str("///        against the system sub-address, returning its response bytes.\n");
    s.push_str("Status sync_exchange(BusController& bc, IClock& clock, Logger& log,\n");
    s.push_str("                     uint8_t opcode, uint8_t units, uint32_t value,\n");
    s.push_str("                     uint8_t* resp, uint32_t resp_cap, uint32_t& got)\n");
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address  = static_cast<uint8_t>(SubAddress::System);\n");
    s.push_str("    req.priority     = MessagePriorities::High;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str("    req.cmd_inline[0] = opcode;\n");
    s.push_str("    req.cmd_inline[1] = units;\n");
    s.push_str("    emplace_u32_le(req.cmd_inline + 2, value);\n");
    s.push_str("    stage_inline_cmd(req, kSyncCmdBytes);\n");
    s.push_str("    req.resp_buf_max = resp_cap;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    got = 0u;\n");
    s.push_str("    return run_one(bc, clock, req, resp, resp_cap, &got, log);\n");
    s.push_str("}\n\n");
    s.push_str("} // namespace\n\n\n");

    s.push_str("Status RtClient::sync_once(SyncMeasurement* raw_out)\n");
    s.push_str("{\n");
    s.push_str("    const uint8_t units = static_cast<uint8_t>(TimeUnits::Millisecond);\n");
    s.push_str("    uint8_t  resp[kSyncFinalRespBytes] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n\n");

    s.push_str("    // NTP four-timestamp exchange over a single round trip. MIRRORS the\n");
    s.push_str("    // sequence in tests/test_sync_e2e.cpp; keep the two in lockstep.\n");
    s.push_str("    // Step 1 (SYNC_PROBE): T1 is the BC clock at send; the RT returns T2 and\n");
    s.push_str("    // T3 (receive and response-send instants) and caches T1; T4 is the BC\n");
    s.push_str("    // clock when the response arrives. The RT runs the resulting (offset,\n");
    s.push_str("    // delay) through a sliding 8-deep min-delay clock filter before it\n");
    s.push_str("    // disciplines its clock, so this client just drives the exchange.\n");
    s.push_str("    const uint32_t t1 = m_clock.now_ms();\n");
    s.push_str(&format!(
        "    Status s = sync_exchange(m_bc, m_clock, m_log, {}, units, t1,\n",
        probe_op
    ));
    s.push_str("                             resp, sizeof(resp), got);\n");
    s.push_str("    const uint32_t t4 = m_clock.now_ms();\n");
    s.push_str("    if (s != Status::Ok || got < kSyncProbeRespBytes) {\n");
    s.push_str("        // Record which phase failed, the underlying status, and the short byte\n");
    s.push_str("        // count so the persistent log pins the failure mode (complements the\n");
    s.push_str("        // LoggingFrameTransport byte dump and run_one's per-op result line).\n");
    s.push_str("        char note[96];\n");
    s.push_str("        std::snprintf(note, sizeof(note),\n");
    s.push_str("                      \"sync_once SYNC_PROBE failed: status=%s got=%u (need>=%u)\",\n");
    s.push_str("                      status_to_string(s), static_cast<unsigned>(got),\n");
    s.push_str("                      static_cast<unsigned>(kSyncProbeRespBytes));\n");
    s.push_str("        m_log.line(\"ERROR\", note);\n");
    s.push_str("        return (s == Status::Ok) ? Status::CrcFailPayload : s;\n");
    s.push_str("    }\n\n");

    s.push_str("    // Step 2 (SYNC_FINAL): send T4; the RT computes the path delay and the\n");
    s.push_str("    // BC-minus-RT offset from its cached T1, T2, T3 and this T4, runs them\n");
    s.push_str("    // through the min-delay clock filter, disciplines its clock from the\n");
    s.push_str("    // selected sample, and returns the fixed-point result for the BC view.\n");
    s.push_str(&format!(
        "    s = sync_exchange(m_bc, m_clock, m_log, {}, units, t4,\n",
        fin_op
    ));
    s.push_str("                      resp, sizeof(resp), got);\n");
    s.push_str("    if (s != Status::Ok || got < kSyncFinalRespBytes) {\n");
    s.push_str("        char note[96];\n");
    s.push_str("        std::snprintf(note, sizeof(note),\n");
    s.push_str("                      \"sync_once SYNC_FINAL failed: status=%s got=%u (need>=%u)\",\n");
    s.push_str("                      status_to_string(s), static_cast<unsigned>(got),\n");
    s.push_str("                      static_cast<unsigned>(kSyncFinalRespBytes));\n");
    s.push_str("        m_log.line(\"ERROR\", note);\n");
    s.push_str("        return (s == Status::Ok) ? Status::CrcFailPayload : s;\n");
    s.push_str("    }\n\n");

    s.push_str("    const double path_delay = sync_decode_unsigned(\n");
    s.push_str("        extract_u32_le(resp), extract_u32_le(resp + 4));\n");
    s.push_str("    const double offset = sync_decode_signed(\n");
    s.push_str("        static_cast<int32_t>(extract_u32_le(resp + 8)),\n");
    s.push_str("        static_cast<int32_t>(extract_u32_le(resp + 12)));\n\n");

    s.push_str("    // The RT returns its recommended offset (already disciplined on the\n");
    s.push_str("    // RT clock by its own smoother) and the latest measured path delay;\n");
    s.push_str("    // store both directly so this view matches the RT -- no BC-side filter.\n");
    s.push_str("    m_offset       = offset;\n");
    s.push_str("    m_path_delay   = path_delay;\n");
    s.push_str("    m_have_sync    = true;\n");
    s.push_str("    m_sync_units   = units;\n\n");

    s.push_str("    if (raw_out != nullptr) {\n");
    s.push_str("        raw_out->path_delay_units = path_delay;\n");
    s.push_str("        raw_out->offset_units     = offset;\n");
    s.push_str("        raw_out->reload_ticks     = m_reload_ticks;\n");
    s.push_str("        raw_out->units            = units;\n");
    s.push_str("    }\n");
    s.push_str("    return Status::Ok;\n");
    s.push_str("}\n");
    s.push_str("\n\n");

    s.push_str("Status RtClient::sync_adjust(SyncAdjustResult* out)\n");
    s.push_str("{\n");
    s.push_str("    // SYNC_ADJUST is a one-byte trigger: the RT applies a single smoothed, bounded\n");
    s.push_str("    // SysTick-reload step from the skew it averaged over the preceding telemetry\n");
    s.push_str("    // phase, and replies with that skew and the reload it programmed\n");
    s.push_str("    // (kSyncAdjustRespBytes, 8 bytes -> fits the inline response slot).\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address  = static_cast<uint8_t>(SubAddress::System);\n");
    s.push_str("    req.priority     = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read      = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = {};\n", adjust_op));
    s.push_str("    stage_inline_cmd(req, kSyncAdjustCmdBytes);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n\n");

    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s != Status::Ok || got < kSyncAdjustRespBytes) {\n");
    s.push_str("        char note[96];\n");
    s.push_str("        std::snprintf(note, sizeof(note),\n");
    s.push_str("                      \"sync_adjust failed: status=%s got=%u (need>=%u)\",\n");
    s.push_str("                      status_to_string(s), static_cast<unsigned>(got),\n");
    s.push_str("                      static_cast<unsigned>(kSyncAdjustRespBytes));\n");
    s.push_str("        m_log.line(\"ERROR\", note);\n");
    s.push_str("        return (s == Status::Ok) ? Status::CrcFailPayload : s;\n");
    s.push_str("    }\n\n");

    s.push_str("    // Word 0: the averaged skew the RT used (int32 ppb). Word 1: the SysTick reload\n");
    s.push_str("    // it programmed. Mirror the reload into reload_ticks() so the BC view stays\n");
    s.push_str("    // current after an adjust.\n");
    s.push_str("    const int32_t  avg_skew_ppb = static_cast<int32_t>(extract_u32_le(resp));\n");
    s.push_str("    const uint32_t reload       = extract_u32_le(resp + 4);\n");
    s.push_str("    m_reload_ticks = reload;\n");
    s.push_str("    if (out != nullptr) {\n");
    s.push_str("        out->avg_skew_ppb = avg_skew_ppb;\n");
    s.push_str("        out->reload_ticks = reload;\n");
    s.push_str("    }\n");
    s.push_str("    return Status::Ok;\n");
    s.push_str("}\n");
    s
}


fn render_peripheral_class_impl(kind: DeviceKind) -> String {
    let name = client_class_name(kind);
    let mut s = String::new();

    if kind_carries_pin_index(kind) {
        s.push_str(&format!(
            "{0}::{0}(BusController& bc, IClock& clock, Logger& log, SubAddress sub_address, uint8_t pin_index)\n    : m_bc(bc)\n    , m_clock(clock)\n    , m_log(log)\n    , m_sa(sub_address)\n    , m_pin_idx(pin_index)\n{{\n}}\n\n\n",
            name
        ));
    } else {
        s.push_str(&format!(
            "{0}::{0}(BusController& bc, IClock& clock, Logger& log, SubAddress sub_address)\n    : m_bc(bc)\n    , m_clock(clock)\n    , m_log(log)\n    , m_sa(sub_address)\n{{\n}}\n\n\n",
            name
        ));
    }

    for body in method_bodies(kind, name) {
        s.push_str(&body);
        s.push_str("\n\n");
    }
    s
}


/// One full method body per public signature; mirrors method_signatures().
fn method_bodies(kind: DeviceKind, class: &str) -> Vec<String> {
    match kind {
        DeviceKind::GpioOutput => vec![
            simple_write_op(class, "set",   &cmd_enum_expr(DeviceKind::GpioOutput, "SET")),
            simple_write_op(class, "clear", &cmd_enum_expr(DeviceKind::GpioOutput, "CLEAR")),
        ],
        DeviceKind::GpioInput => vec![
            gpio_input_read(class),
        ],
        DeviceKind::Adc => vec![
            adc_sample(class),
        ],
        DeviceKind::Dac => vec![
            dac_set_counts(class),
        ],
        DeviceKind::Pwm => vec![
            pwm_set_duty(class),
        ],
        DeviceKind::Spi => vec![
            variable_tx(class, &cmd_enum_expr(DeviceKind::Spi, "TX")),
            variable_rx(class, &cmd_enum_expr(DeviceKind::Spi, "RX")),
        ],
        DeviceKind::Uart => vec![
            variable_tx(class, &cmd_enum_expr(DeviceKind::Uart, "TX")),
            variable_rx(class, &cmd_enum_expr(DeviceKind::Uart, "RX")),
        ],
        DeviceKind::I2c => vec![
            i2c_write_reg(class),
            i2c_read_reg(class),
        ],
        DeviceKind::Can => vec![
            can_tx_frame(class),
            can_rx_frame(class),
        ],
        DeviceKind::Qspi => vec![
            qspi_read(class),
            qspi_program(class),
            qspi_erase(class),
            qspi_read_status(class),
            qspi_write_enable(class),
        ],
        DeviceKind::Rtc => vec![
            rtc_set_datetime(class),
            rtc_get_datetime(class),
        ],
    }
}


/// GPIO output SET/CLEAR — cmd is `[opcode, pin_idx]`; the bank handler
/// on the RT side decodes the pin index and dispatches to the matching
/// HAL pin. No response payload.
fn simple_write_op(class: &str, method: &str, opcode_expr: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::{}()\n", class, method));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!("    req.cmd_inline[0] = static_cast<uint8_t>({});\n", opcode_expr));
    s.push_str("    req.cmd_inline[1] = m_pin_idx;\n");
    s.push_str("    stage_inline_cmd(req, 2u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn gpio_input_read(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::read(bool& value_out)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::GpioInput, "READ")
    ));
    s.push_str("    req.cmd_inline[1] = m_pin_idx;\n");
    s.push_str("    stage_inline_cmd(req, 2u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 1u) {\n");
    s.push_str("        value_out = (resp[0] != 0u);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


fn adc_sample(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::sample(uint16_t& counts_out, uint32_t& timestamp_ms_out)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Adc, "SAMPLE")
    ));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 6u) {\n");
    s.push_str("        counts_out = static_cast<uint16_t>(resp[0])\n");
    s.push_str("                   | (static_cast<uint16_t>(resp[1]) << 8);\n");
    s.push_str("        timestamp_ms_out =\n");
    s.push_str("              (static_cast<uint32_t>(resp[2]))\n");
    s.push_str("            | (static_cast<uint32_t>(resp[3]) <<  8)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[4]) << 16)\n");
    s.push_str("            | (static_cast<uint32_t>(resp[5]) << 24);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


fn dac_set_counts(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::set_counts(uint16_t counts)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Dac, "SET_COUNTS")
    ));
    s.push_str("    req.cmd_inline[1] = static_cast<uint8_t>(counts & 0xFFu);\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((counts >> 8) & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 3u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn pwm_set_duty(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::set_duty(uint16_t duty_hundredths)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Pwm, "SET_DUTY")
    ));
    s.push_str("    req.cmd_inline[1] = static_cast<uint8_t>(duty_hundredths & 0xFFu);\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((duty_hundredths >> 8) & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 3u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


/// SPI/UART TX: cmd is [op, len_lo, len_hi, data...]. Uses an external
/// scratch buffer when actual_len + 3 > kInlinePayloadBytes.
fn variable_tx(class: &str, opcode_expr: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::tx(uint8_t const* data, uint16_t len)\n", class));
    s.push_str("{\n");
    s.push_str("    if (data == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    const uint32_t actual = static_cast<uint32_t>(len) + 3u;\n");
    s.push_str("    if (actual > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push_str("    const PayloadSizes ps   = bytes_to_payload_size(actual);\n");
    s.push_str("    const uint32_t slot     = payload_size_to_bytes(ps);\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str("    req.cmd_size    = ps;\n");
    s.push_str("    req.cmd_len     = slot;\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    uint8_t* buf = (slot <= kInlinePayloadBytes) ? req.cmd_inline : scratch;\n");
    s.push_str(&format!("    buf[0] = static_cast<uint8_t>({});\n", opcode_expr));
    s.push_str("    buf[1] = static_cast<uint8_t>(len & 0xFFu);\n");
    s.push_str("    buf[2] = static_cast<uint8_t>((len >> 8) & 0xFFu);\n");
    s.push_str("    if (len > 0u) {\n");
    s.push_str("        std::memcpy(buf + 3, data, len);\n");
    s.push_str("    }\n");
    s.push_str("    if (slot > (3u + static_cast<uint32_t>(len))) {\n");
    s.push_str("        std::memset(buf + 3 + len, 0, slot - 3u - len);\n");
    s.push_str("    }\n");
    s.push_str("    if (slot > kInlinePayloadBytes) {\n");
    s.push_str("        req.cmd_ext = scratch;\n");
    s.push_str("    } else {\n");
    s.push_str("        req.cmd_ext = nullptr;\n");
    s.push_str("    }\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


/// SPI/UART RX: cmd is [op, len_lo, len_hi] padded to size_4; response is
/// `len` bytes (padded by RT to the next slot).
fn variable_rx(class: &str, opcode_expr: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::rx(uint16_t len, uint8_t* data_out)\n", class));
    s.push_str("{\n");
    s.push_str("    if (data_out == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    if (static_cast<uint32_t>(len) > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!("    req.cmd_inline[0] = static_cast<uint8_t>({});\n", opcode_expr));
    s.push_str("    req.cmd_inline[1] = static_cast<uint8_t>(len & 0xFFu);\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((len >> 8) & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 3u);\n");
    s.push('\n');
    s.push_str("    const PayloadSizes resp_ps = bytes_to_payload_size(len);\n");
    s.push_str("    const uint32_t resp_slot   = payload_size_to_bytes(resp_ps);\n");
    s.push_str("    req.resp_buf_max = resp_slot;\n");
    s.push('\n');
    s.push_str("    uint8_t inline_resp[kInlinePayloadBytes] = {0};\n");
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    const bool use_ext = (resp_slot > kInlinePayloadBytes);\n");
    s.push_str("    if (use_ext) {\n");
    s.push_str("        req.resp_ext = scratch;\n");
    s.push_str("    } else {\n");
    s.push_str("        req.resp_ext = nullptr;\n");
    s.push_str("    }\n");
    s.push('\n');
    s.push_str("    uint8_t* sink = use_ext ? scratch : inline_resp;\n");
    s.push_str("    uint32_t cap  = use_ext ? sizeof(scratch) : sizeof(inline_resp);\n");
    s.push_str("    uint32_t got  = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, sink, cap, &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && len > 0u) {\n");
    s.push_str("        const uint32_t n = (got < len) ? got : len;\n");
    s.push_str("        std::memcpy(data_out, sink, n);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


fn i2c_write_reg(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::write_reg(uint8_t addr7, uint8_t reg, uint8_t const* data, uint8_t len)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (data == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    const uint32_t actual = 4u + static_cast<uint32_t>(len);\n");
    s.push_str("    if (actual > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push_str("    const PayloadSizes ps = bytes_to_payload_size(actual);\n");
    s.push_str("    const uint32_t slot   = payload_size_to_bytes(ps);\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address  = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority     = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read      = false;\n");
    s.push_str("    req.cmd_size     = ps;\n");
    s.push_str("    req.cmd_len      = slot;\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    uint8_t* buf = (slot <= kInlinePayloadBytes) ? req.cmd_inline : scratch;\n");
    s.push_str(&format!(
        "    buf[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::I2c, "WRITE_REG")
    ));
    s.push_str("    buf[1] = addr7;\n");
    s.push_str("    buf[2] = reg;\n");
    s.push_str("    buf[3] = len;\n");
    s.push_str("    if (len > 0u) {\n");
    s.push_str("        std::memcpy(buf + 4, data, len);\n");
    s.push_str("    }\n");
    s.push_str("    if (slot > actual) {\n");
    s.push_str("        std::memset(buf + actual, 0, slot - actual);\n");
    s.push_str("    }\n");
    s.push_str("    req.cmd_ext = (slot > kInlinePayloadBytes) ? scratch : nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn i2c_read_reg(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::read_reg(uint8_t addr7, uint8_t reg, uint8_t len, uint8_t* data_out)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (data_out == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::I2c, "READ_REG")
    ));
    s.push_str("    req.cmd_inline[1] = addr7;\n");
    s.push_str("    req.cmd_inline[2] = reg;\n");
    s.push_str("    req.cmd_inline[3] = len;\n");
    s.push_str("    stage_inline_cmd(req, 4u);\n");
    s.push('\n');
    s.push_str("    const PayloadSizes resp_ps = bytes_to_payload_size(len);\n");
    s.push_str("    const uint32_t resp_slot   = payload_size_to_bytes(resp_ps);\n");
    s.push_str("    req.resp_buf_max = resp_slot;\n");
    s.push('\n');
    s.push_str("    uint8_t inline_resp[kInlinePayloadBytes] = {0};\n");
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    const bool use_ext = (resp_slot > kInlinePayloadBytes);\n");
    s.push_str("    req.resp_ext = use_ext ? scratch : nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t* sink = use_ext ? scratch : inline_resp;\n");
    s.push_str("    uint32_t cap  = use_ext ? sizeof(scratch) : sizeof(inline_resp);\n");
    s.push_str("    uint32_t got  = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, sink, cap, &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && len > 0u) {\n");
    s.push_str("        const uint32_t n = (got < len) ? got : len;\n");
    s.push_str("        std::memcpy(data_out, sink, n);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


fn can_tx_frame(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::tx_frame(uint32_t id, uint8_t dlc, const uint8_t data[8])\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (dlc > 8u || (data == nullptr && dlc > 0u)) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Can, "TX_FRAME")
    ));
    s.push_str("    // CAN id is big-endian on the wire (matches can_handler.cpp).\n");
    s.push_str("    req.cmd_inline[1] = static_cast<uint8_t>((id >> 24) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((id >> 16) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[3] = static_cast<uint8_t>((id >>  8) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[4] = static_cast<uint8_t>( id        & 0xFFu);\n");
    s.push_str("    req.cmd_inline[5] = dlc;\n");
    s.push_str("    for (uint8_t i = 0; i < dlc; ++i) {\n");
    s.push_str("        req.cmd_inline[6u + i] = data[i];\n");
    s.push_str("    }\n");
    s.push_str("    stage_inline_cmd(req, 6u + static_cast<uint32_t>(dlc));\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn can_rx_frame(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::rx_frame(uint32_t& id_out, uint8_t& dlc_out, uint8_t data_out[8])\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (data_out == nullptr) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Can, "RX_FRAME")
    ));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 13u) {\n");
    s.push_str("        id_out  = (static_cast<uint32_t>(resp[0]) << 24)\n");
    s.push_str("                | (static_cast<uint32_t>(resp[1]) << 16)\n");
    s.push_str("                | (static_cast<uint32_t>(resp[2]) <<  8)\n");
    s.push_str("                |  static_cast<uint32_t>(resp[3]);\n");
    s.push_str("        dlc_out = resp[4];\n");
    s.push_str("        std::memcpy(data_out, resp + 5, 8);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


/// QSPI READ: cmd is [op, instr, addr_be24, dummy, data_lines, len_le16];
/// response is `len` bytes (padded by RT to the next slot).
fn qspi_read(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::read(uint8_t instr, uint32_t address, uint8_t dummy_cycles, uint8_t data_lines, uint16_t len, uint8_t* out)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (out == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    if (static_cast<uint32_t>(len) > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Qspi, "READ")
    ));
    s.push_str("    req.cmd_inline[1] = instr;\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((address >> 16) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[3] = static_cast<uint8_t>((address >> 8) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[4] = static_cast<uint8_t>(address & 0xFFu);\n");
    s.push_str("    req.cmd_inline[5] = dummy_cycles;\n");
    s.push_str("    req.cmd_inline[6] = data_lines;\n");
    s.push_str("    req.cmd_inline[7] = static_cast<uint8_t>(len & 0xFFu);\n");
    s.push_str("    req.cmd_inline[8] = static_cast<uint8_t>((len >> 8) & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 9u);\n");
    s.push('\n');
    s.push_str("    const PayloadSizes resp_ps = bytes_to_payload_size(len);\n");
    s.push_str("    const uint32_t resp_slot   = payload_size_to_bytes(resp_ps);\n");
    s.push_str("    req.resp_buf_max = resp_slot;\n");
    s.push('\n');
    s.push_str("    uint8_t inline_resp[kInlinePayloadBytes] = {0};\n");
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    const bool use_ext = (resp_slot > kInlinePayloadBytes);\n");
    s.push_str("    req.resp_ext = use_ext ? scratch : nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t* sink = use_ext ? scratch : inline_resp;\n");
    s.push_str("    uint32_t cap  = use_ext ? sizeof(scratch) : sizeof(inline_resp);\n");
    s.push_str("    uint32_t got  = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, sink, cap, &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && len > 0u) {\n");
    s.push_str("        const uint32_t n = (got < len) ? got : len;\n");
    s.push_str("        std::memcpy(out, sink, n);\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


/// QSPI PROGRAM: cmd is [op, instr, addr_be24, data_lines, len_le16, data...].
/// Uses an external scratch buffer when the staged command exceeds the inline
/// capacity. No response payload.
fn qspi_program(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::program(uint8_t instr, uint32_t address, uint8_t data_lines, uint8_t const* data, uint16_t len)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    if (data == nullptr && len > 0u) {\n");
    s.push_str("        return Status::HandlerError;\n");
    s.push_str("    }\n");
    s.push_str("    const uint32_t actual = 8u + static_cast<uint32_t>(len);\n");
    s.push_str("    if (actual > kMaxPayloadBytes) {\n");
    s.push_str("        return Status::PayloadTooLarge;\n");
    s.push_str("    }\n");
    s.push_str("    const PayloadSizes ps = bytes_to_payload_size(actual);\n");
    s.push_str("    const uint32_t slot   = payload_size_to_bytes(ps);\n");
    s.push('\n');
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address  = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority     = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read      = false;\n");
    s.push_str("    req.cmd_size     = ps;\n");
    s.push_str("    req.cmd_len      = slot;\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t scratch[kMaxPayloadBytes];\n");
    s.push_str("    uint8_t* buf = (slot <= kInlinePayloadBytes) ? req.cmd_inline : scratch;\n");
    s.push_str(&format!(
        "    buf[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Qspi, "PROGRAM")
    ));
    s.push_str("    buf[1] = instr;\n");
    s.push_str("    buf[2] = static_cast<uint8_t>((address >> 16) & 0xFFu);\n");
    s.push_str("    buf[3] = static_cast<uint8_t>((address >> 8) & 0xFFu);\n");
    s.push_str("    buf[4] = static_cast<uint8_t>(address & 0xFFu);\n");
    s.push_str("    buf[5] = data_lines;\n");
    s.push_str("    buf[6] = static_cast<uint8_t>(len & 0xFFu);\n");
    s.push_str("    buf[7] = static_cast<uint8_t>((len >> 8) & 0xFFu);\n");
    s.push_str("    if (len > 0u) {\n");
    s.push_str("        std::memcpy(buf + 8, data, len);\n");
    s.push_str("    }\n");
    s.push_str("    if (slot > actual) {\n");
    s.push_str("        std::memset(buf + actual, 0, slot - actual);\n");
    s.push_str("    }\n");
    s.push_str("    req.cmd_ext = (slot > kInlinePayloadBytes) ? scratch : nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


/// QSPI ERASE: cmd is [op, instr, addr_be24]; no response payload.
fn qspi_erase(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::erase(uint8_t instr, uint32_t address)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Qspi, "ERASE")
    ));
    s.push_str("    req.cmd_inline[1] = instr;\n");
    s.push_str("    req.cmd_inline[2] = static_cast<uint8_t>((address >> 16) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[3] = static_cast<uint8_t>((address >> 8) & 0xFFu);\n");
    s.push_str("    req.cmd_inline[4] = static_cast<uint8_t>(address & 0xFFu);\n");
    s.push_str("    stage_inline_cmd(req, 5u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


/// QSPI READ_STATUS: cmd is [op, instr]; response is a single status byte.
fn qspi_read_status(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::read_status(uint8_t instr, uint8_t& status_out)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Qspi, "READ_STATUS")
    ));
    s.push_str("    req.cmd_inline[1] = instr;\n");
    s.push_str("    stage_inline_cmd(req, 2u);\n");
    s.push_str("    req.resp_buf_max = 16u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[16] = {0};\n");
    s.push_str("    uint32_t got      = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 1u) {\n");
    s.push_str("        status_out = resp[0];\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


/// QSPI WRITE_ENABLE: cmd is [op, instr]; no response payload.
fn qspi_write_enable(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("Status {}::write_enable(uint8_t instr)\n", class));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Qspi, "WRITE_ENABLE")
    ));
    s.push_str("    req.cmd_inline[1] = instr;\n");
    s.push_str("    stage_inline_cmd(req, 2u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn rtc_set_datetime(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::set_datetime(uint8_t year_offset, uint8_t month, uint8_t day, uint8_t weekday, uint8_t hours, uint8_t minutes, uint8_t seconds)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = false;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Rtc, "SET_DATETIME")
    ));
    s.push_str("    req.cmd_inline[1] = year_offset;\n");
    s.push_str("    req.cmd_inline[2] = month;\n");
    s.push_str("    req.cmd_inline[3] = day;\n");
    s.push_str("    req.cmd_inline[4] = weekday;\n");
    s.push_str("    req.cmd_inline[5] = hours;\n");
    s.push_str("    req.cmd_inline[6] = minutes;\n");
    s.push_str("    req.cmd_inline[7] = seconds;\n");
    s.push_str("    stage_inline_cmd(req, 8u);\n");
    s.push_str("    req.resp_buf_max = 0u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push_str("    return run_one(m_bc, m_clock, req, nullptr, 0u, nullptr, m_log);\n");
    s.push_str("}\n");
    s
}


fn rtc_get_datetime(class: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Status {}::get_datetime(uint8_t& year_offset_out, uint8_t& month_out, uint8_t& day_out, uint8_t& weekday_out, uint8_t& hours_out, uint8_t& minutes_out, uint8_t& seconds_out)\n",
        class
    ));
    s.push_str("{\n");
    s.push_str("    Request req{};\n");
    s.push_str("    req.sub_address = static_cast<uint8_t>(m_sa);\n");
    s.push_str("    req.priority    = MessagePriorities::Medium;\n");
    s.push_str("    req.is_read     = true;\n");
    s.push_str(&format!(
        "    req.cmd_inline[0] = static_cast<uint8_t>({});\n",
        cmd_enum_expr(DeviceKind::Rtc, "GET_DATETIME")
    ));
    s.push_str("    stage_inline_cmd(req, 1u);\n");
    s.push_str("    req.resp_buf_max = 8u;\n");
    s.push_str("    req.resp_ext     = nullptr;\n");
    s.push('\n');
    s.push_str("    uint8_t  resp[8] = {0};\n");
    s.push_str("    uint32_t got     = 0u;\n");
    s.push_str("    const Status s = run_one(m_bc, m_clock, req, resp, sizeof(resp), &got, m_log);\n");
    s.push_str("    if (s == Status::Ok && got >= 7u) {\n");
    s.push_str("        year_offset_out = resp[0];\n");
    s.push_str("        month_out       = resp[1];\n");
    s.push_str("        day_out         = resp[2];\n");
    s.push_str("        weekday_out     = resp[3];\n");
    s.push_str("        hours_out       = resp[4];\n");
    s.push_str("        minutes_out     = resp[5];\n");
    s.push_str("        seconds_out     = resp[6];\n");
    s.push_str("    }\n");
    s.push_str("    return s;\n");
    s.push_str("}\n");
    s
}


///////////////////////////////////////////////////////////////////////////////
// main.cpp
///////////////////////////////////////////////////////////////////////////////

/// Render `main.cpp` (the generated entry point).
///
/// `default_baud` is baked into the `kDefaultBaud` constant and into the
/// `Usage:` example commands. When `emit_unknown_note` is `true` the
/// generator inserts a C++ comment block above `kDefaultBaud` explaining
/// that the value is a fallback (the `.ioc` did not specify a baud rate
/// for the chosen UART), and tags the Usage string with the same caveat
/// so users running `--help` see it too.
///
/// The file carries STM32CubeMX-style `/* USER CODE BEGIN/END */` regions;
/// [`write_text_merging`] preserves their contents across regeneration.
fn render_example_sequence(
    slots: &[DeviceSlot],
    default_baud: u32,
    emit_unknown_note: bool,
) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("/// @file main.cpp\n");
    s.push_str("/// @brief Generated starting point for a BC test sequence.\n");
    s.push_str("/// @details\n");
    s.push_str("/// Opens the serial port to the RT, verifies that the BC and RT\n");
    s.push_str("/// were built from the same .ioc (fingerprint check), and runs one\n");
    s.push_str("/// representative call per peripheral kind. Edit the USER CODE\n");
    s.push_str("/// regions to script your real test sequences; everything you put\n");
    s.push_str("/// between a `USER CODE BEGIN`/`END` pair is preserved when the\n");
    s.push_str("/// project is regenerated. Note: because the example calls live\n");
    s.push_str("/// inside the `Sequence` region, peripherals added to the .ioc\n");
    s.push_str("/// later will not be auto-inserted once you have edited it.\n\n");
    s.push_str("#include <cstdio>\n");
    s.push_str("#include <cstdlib>\n\n");
    s.push_str("#include \"rt_client.h\"\n");
    s.push_str("#include \"host/serial_frame_transport.h\"\n");
    s.push_str("#include \"host/logging_frame_transport.h\"\n");
    s.push_str("#include \"host/logger.h\"\n");
    s.push_str("#include \"host/std_steady_clock.h\"\n");
    s.push_str("#include \"host/delay.h\"\n\n");
    s.push_str("/* USER CODE BEGIN Includes */\n");
    s.push('\n');
    s.push_str("/* USER CODE END Includes */\n\n");

    if emit_unknown_note {
        s.push_str("// NOTE: the .ioc file did not specify a baud rate for the chosen UART.\n");
        s.push_str("//       The value below is a best-guess default — pass the actual\n");
        s.push_str("//       baud as argv[2] if this does not match the firmware.\n");
    }
    s.push_str(&format!(
        "static constexpr uint32_t kDefaultBaud = {}u;\n\n",
        default_baud
    ));

    s.push_str("/* USER CODE BEGIN Decls */\n");
    s.push('\n');
    s.push_str("/* USER CODE END Decls */\n\n");

    s.push_str("int main(int argc, char** argv)\n");
    s.push_str("{\n");
    s.push_str("    if (argc < 2) {\n");
    s.push_str("        std::fprintf(stderr,\n");
    s.push_str("            \"Usage: %s <serial-device> [<baud>]\\n\"\n");
    s.push_str(&format!(
        "            \"  Linux/macOS: %s /dev/ttyUSB0 {baud}\\n\"\n",
        baud = default_baud,
    ));
    if emit_unknown_note {
        s.push_str(&format!(
            "            \"  Windows:     %s COM7 {baud}\\n\"\n",
            baud = default_baud,
        ));
        s.push_str("            \"  (NOTE: baud above is a fallback; .ioc did not specify one)\\n\",\n");
    } else {
        s.push_str(&format!(
            "            \"  Windows:     %s COM7 {baud}\\n\",\n",
            baud = default_baud,
        ));
    }
    s.push_str("            argv[0], argv[0], argv[0]);\n");
    s.push_str("        return 1;\n");
    s.push_str("    }\n");
    s.push_str("    char const* dev = argv[1];\n");
    s.push_str("    uint32_t baud = kDefaultBaud;\n");
    s.push_str("    if (argc >= 3) {\n");
    s.push_str("        baud = static_cast<uint32_t>(std::atoi(argv[2]));\n");
    s.push_str("    }\n\n");
    s.push_str("    // Construct the logger first so even an open() failure is recorded.\n");
    s.push_str("    Logger logger(Logger::default_log_path());\n");
    s.push_str("    logger.line(\"INFO\", \"exhibition_bc starting\");\n");
    s.push_str("    std::printf(\"Logging to %s\\n\", logger.path().c_str());\n\n");
    s.push_str("    SerialFrameTransport serial;\n");
    s.push_str("    if (!serial.open(dev, baud)) {\n");
    s.push_str("        std::fprintf(stderr, \"Failed to open %s at %u baud\\n\", dev, baud);\n");
    s.push_str("        logger.line(\"ERROR\", \"failed to open serial port\");\n");
    s.push_str("        return 2;\n");
    s.push_str("    }\n\n");
    s.push_str("    // Decorate the real transport so all wire traffic is logged.\n");
    s.push_str("    LoggingFrameTransport transport(serial, logger, &op_name, &sub_address_name);\n");
    s.push_str("    StdSteadyClock clock;\n");
    s.push_str("    RtClient client(transport, clock, logger);\n\n");

    s.push_str("    /* USER CODE BEGIN Init */\n");
    s.push('\n');
    s.push_str("    /* USER CODE END Init */\n\n");

    s.push_str("    const Status fps = client.verify_fingerprint();\n");
    s.push_str("    if (fps != Status::Ok) {\n");
    s.push_str("        std::fprintf(stderr,\n");
    s.push_str("            \"Fingerprint check failed: %s (expected 0x%08X)\\n\",\n");
    s.push_str("            status_to_string(fps),\n");
    s.push_str("            static_cast<unsigned>(kApiFingerprint));\n");
    s.push_str("        // Localize the divergence: per-service hashes pinpoint which\n");
    s.push_str("        // sub-address(es) differ (each mismatch is logged as ERROR).\n");
    s.push_str("        client.verify_service_hashes();\n");
    s.push_str("        return 3;\n");
    s.push_str("    }\n");
    s.push_str("    std::printf(\"Fingerprint OK (0x%08X)\\n\",\n");
    s.push_str("                static_cast<unsigned>(kApiFingerprint));\n\n");
    s.push_str("    uint16_t count = 0;\n");
    s.push_str("    if (client.get_device_count(count) == Status::Ok) {\n");
    s.push_str("        std::printf(\"RT reports %u registered devices.\\n\", count);\n");
    s.push_str("    }\n\n");

    s.push_str("    // PTP-style time sync: run the exchange a few times to let the RT's\n");
    s.push_str("    // offset filter settle. The offset is the RT systick relative to this\n");
    s.push_str("    // host's clock; add it to an RT timestamp to express it here.\n");
    s.push_str("    // Note 32 is the smoother window size.\n");
    s.push_str("    for (int i = 0; i < 32; ++i) {\n");
    s.push_str("        SyncMeasurement m;\n");
    s.push_str("        if (client.sync_once(&m) != Status::Ok) {\n");
    s.push_str("            break;\n");
    s.push_str("        }\n");
    s.push_str("        delay(50);\n");
    s.push_str("    }\n");
    s.push_str("    if (client.has_sync()) {\n");
    s.push_str("        std::printf(\"Time sync: offset = %.3f, path delay = %.3f (units 1e-%u s), \"\n");
    s.push_str("                    \"systick reload = %u\\n\",\n");
    s.push_str("                    client.clock_offset_units(), client.path_delay_units(),\n");
    s.push_str("                    static_cast<unsigned>(client.sync_units()),\n");
    s.push_str("                    static_cast<unsigned>(client.reload_ticks()));\n");
    s.push_str("    }\n\n");

    // Per-IOC stamped examples: one per kind seen, against the first
    // member of the first slot of that kind. Iterating members (not
    // slots) lets a GPIO bank slot stamp `client.gpio_PA5.set()` rather
    // than the bank-level handler name a user never types directly.
    //
    // The examples live inside the `Sequence` USER CODE region so a user's
    // edits to them survive regeneration (CubeMX-strict): once edited, the
    // user's version wins and newly added peripherals are not re-injected.
    s.push_str("    /* USER CODE BEGIN Sequence */\n");
    let mut seen: Vec<DeviceKind> = Vec::new();
    for slot in slots {
        if seen.contains(&slot.kind) {
            continue;
        }
        let Some(member) = slot.members.first() else { continue };
        seen.push(slot.kind);
        s.push_str(&render_example_for_slot(slot, member));
        s.push('\n');
    }
    s.push_str("    /* USER CODE END Sequence */\n\n");

    s.push_str("    /* USER CODE BEGIN PreReturn */\n");
    s.push('\n');
    s.push_str("    /* USER CODE END PreReturn */\n\n");

    s.push_str("    return 0;\n");
    s.push_str("}\n");
    s
}


fn render_example_for_slot(slot: &DeviceSlot, member: &SlotMember) -> String {
    let inst = &member.instance_name;
    let label = &member.label;
    let mut s = String::new();
    match slot.kind {
        DeviceKind::GpioOutput => {
            s.push_str(&format!("    // GPIO output {} ({}): on, wait 500 ms, off.\n", label, inst));
            s.push_str(&format!("    client.{}.set();\n", inst));
            s.push_str("    delay(500u);   // half-second blocking delay\n");
            s.push_str(&format!("    client.{}.clear();\n", inst));
        }
        DeviceKind::GpioInput => {
            s.push_str(&format!("    // GPIO input {} ({}): single read.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        bool pin_value = false;\n");
            s.push_str(&format!(
                "        if (client.{}.read(pin_value) == Status::Ok) {{\n", inst));
            s.push_str(&format!(
                "            std::printf(\"{} = %d\\n\", pin_value ? 1 : 0);\n", label));
            s.push_str("        }\n");
            s.push_str("    }\n");
        }
        DeviceKind::Adc => {
            s.push_str(&format!("    // ADC {} ({}): single sample.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        uint16_t adc_counts = 0;\n");
            s.push_str("        uint32_t adc_timestamp_ms = 0;\n");
            s.push_str(&format!(
                "        if (client.{}.sample(adc_counts, adc_timestamp_ms) == Status::Ok) {{\n", inst));
            s.push_str(&format!(
                "            std::printf(\"{} = %u counts @ %u ms\\n\", adc_counts, adc_timestamp_ms);\n", label));
            s.push_str("        }\n");
            s.push_str("    }\n");
        }
        DeviceKind::Dac => {
            s.push_str(&format!("    // DAC {} ({}): mid-scale.\n", label, inst));
            s.push_str(&format!("    client.{}.set_counts(2048u);\n", inst));
        }
        DeviceKind::Pwm => {
            s.push_str(&format!("    // PWM {} ({}): 50%% duty (5000 = 50.00%%).\n", label, inst));
            s.push_str(&format!("    client.{}.set_duty(5000u);\n", inst));
        }
        DeviceKind::Spi => {
            s.push_str(&format!(
                "    // SPI {} ({}): TX two bytes, then RX two bytes.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        uint8_t tx[2] = {0xDE, 0xAD};\n");
            s.push_str(&format!("        client.{}.tx(tx, 2u);\n", inst));
            s.push_str("        uint8_t rx[2] = {0};\n");
            s.push_str(&format!("        client.{}.rx(2u, rx);\n", inst));
            s.push_str("    }\n");
        }
        DeviceKind::Uart => {
            s.push_str(&format!(
                "    // UART {} ({}): TX a short string, then RX one byte.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        const uint8_t msg[] = {'H','I','\\n'};\n");
            s.push_str(&format!("        client.{}.tx(msg, 3u);\n", inst));
            s.push_str("        uint8_t one = 0;\n");
            s.push_str(&format!("        client.{}.rx(1u, &one);\n", inst));
            s.push_str("    }\n");
        }
        DeviceKind::I2c => {
            s.push_str(&format!(
                "    // I2C {} ({}): WHO_AM_I-style read at addr 0x68, reg 0x75.\n",
                label, inst));
            s.push_str("    {\n");
            s.push_str("        uint8_t who = 0;\n");
            s.push_str(&format!("        client.{}.read_reg(0x68u, 0x75u, 1u, &who);\n", inst));
            s.push_str(&format!(
                "        std::printf(\"{} WHO_AM_I = 0x%02X\\n\", who);\n", label));
            s.push_str("    }\n");
        }
        DeviceKind::Can => {
            s.push_str(&format!(
                "    // CAN {} ({}): receive one frame if any are queued.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        uint32_t id = 0;\n");
            s.push_str("        uint8_t  dlc = 0;\n");
            s.push_str("        uint8_t  data[8] = {0};\n");
            s.push_str(&format!(
                "        if (client.{}.rx_frame(id, dlc, data) == Status::Ok) {{\n", inst));
            s.push_str(&format!(
                "            std::printf(\"{} rx id=0x%X dlc=%u\\n\", id, dlc);\n", label));
            s.push_str("        }\n");
            s.push_str("    }\n");
        }
        DeviceKind::Qspi => {
            s.push_str(&format!(
                "    // QSPI {} ({}): read status (instr 0x05), then read 16 bytes\n", label, inst));
            s.push_str("    // from address 0 (instr 0x03, single-line, no dummy cycles).\n");
            s.push_str("    {\n");
            s.push_str("        uint8_t status = 0;\n");
            s.push_str(&format!(
                "        if (client.{}.read_status(0x05u, status) == Status::Ok) {{\n", inst));
            s.push_str(&format!(
                "            std::printf(\"{} status = 0x%02X\\n\", status);\n", label));
            s.push_str("        }\n");
            s.push_str("        uint8_t buf[16] = {0};\n");
            s.push_str(&format!(
                "        client.{}.read(0x03u, 0x000000u, 0u, 1u, sizeof(buf), buf);\n", inst));
            s.push_str("    }\n");
        }
        DeviceKind::Rtc => {
            s.push_str(&format!(
                "    // RTC {} ({}): read the current date and time.\n", label, inst));
            s.push_str("    {\n");
            s.push_str("        uint8_t yr = 0, mo = 0, dy = 0, wd = 0, hr = 0, mn = 0, sc = 0;\n");
            s.push_str(&format!(
                "        if (client.{}.get_datetime(yr, mo, dy, wd, hr, mn, sc) == Status::Ok) {{\n", inst));
            s.push_str(&format!(
                "            std::printf(\"{} 20%02u-%02u-%02u %02u:%02u:%02u (weekday %u)\\n\", yr, mo, dy, hr, mn, sc, wd);\n", label));
            s.push_str("        }\n");
            s.push_str("    }\n");
        }
    }
    s
}


///////////////////////////////////////////////////////////////////////////////
// CMake build
///////////////////////////////////////////////////////////////////////////////

fn render_cmakelists() -> String {
    let mut s = String::new();
    s.push_str("# Generated by exhibition; rebuild by regenerating this project.\n");
    s.push_str("#\n");
    s.push_str("# Portable CMake build (Linux, macOS, and Windows via MSVC or MinGW),\n");
    s.push_str("# matching the RT side. No platform-specific link libraries are needed:\n");
    s.push_str("# termios is in libc and ReadFile/WriteFile in kernel32; threads come\n");
    s.push_str("# from the imported Threads::Threads target.\n");
    s.push_str("#\n");
    s.push_str("# Configure and build with the bundled Ninja presets:\n");
    s.push_str("#   cmake --preset Debug\n");
    s.push_str("#   cmake --build --preset Debug\n");
    s.push_str(&format!(
        "# The binary lands at build/Debug/{}[.exe].\n",
        BC_TARGET_NAME
    ));
    s.push_str("#\n");
    s.push_str("# Sources are auto-discovered with file(GLOB), so hand-written code\n");
    s.push_str("# dropped into user/ (or alongside the generated files) is built without\n");
    s.push_str("# editing this file and survives regeneration. CONFIGURE_DEPENDS re-globs\n");
    s.push_str("# on build so a newly added source is picked up without re-configuring.\n\n");
    s.push_str("cmake_minimum_required(VERSION 3.15)\n");
    s.push_str(&format!("project({} CXX)\n\n", BC_TARGET_NAME));
    s.push_str("set(CMAKE_CXX_STANDARD 17)\n");
    s.push_str("set(CMAKE_CXX_STANDARD_REQUIRED ON)\n");
    s.push_str("set(CMAKE_CXX_EXTENSIONS OFF)\n\n");
    // Auto-discover every .cpp in each source directory (non-recursive). This
    // picks up the generated sources plus anything the user drops in user/,
    // so new sources build without editing this file. A missing user/
    // directory contributes nothing.
    s.push_str("file(GLOB SOURCES CONFIGURE_DEPENDS\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/*.cpp\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/common/*.cpp\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/communication/*.cpp\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/host/*.cpp\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/user/*.cpp\")\n\n");
    s.push_str(&format!("add_executable({} ${{SOURCES}})\n\n", BC_TARGET_NAME));
    s.push_str(&format!(
        "target_include_directories({} PRIVATE\n",
        BC_TARGET_NAME
    ));
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/common\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/communication\"\n");
    s.push_str("    \"${CMAKE_CURRENT_SOURCE_DIR}/host\")\n\n");
    s.push_str("# The host Logger runs its file writer on a std::thread.\n");
    s.push_str("find_package(Threads REQUIRED)\n");
    s.push_str(&format!(
        "target_link_libraries({} PRIVATE Threads::Threads)\n\n",
        BC_TARGET_NAME
    ));
    s.push_str("if(MSVC)\n");
    s.push_str(&format!(
        "    target_compile_options({} PRIVATE /W3)\n",
        BC_TARGET_NAME
    ));
    s.push_str("else()\n");
    s.push_str(&format!(
        "    target_compile_options({} PRIVATE -Wall -Wextra -Wpedantic)\n",
        BC_TARGET_NAME
    ));
    s.push_str("endif()\n");
    s
}


/// CMake presets for the BC project. A Ninja Debug/Release pair mirroring the
/// RT side's `CMakePresets.json`, so the documented build command is identical:
/// `cmake --preset Debug && cmake --build --preset Debug`.
fn render_cmake_presets() -> String {
    r#"{
    "version": 3,
    "configurePresets": [
        {
            "name": "default",
            "hidden": true,
            "generator": "Ninja",
            "binaryDir": "${sourceDir}/build/${presetName}"
        },
        {
            "name": "Debug",
            "inherits": "default",
            "cacheVariables": {
                "CMAKE_BUILD_TYPE": "Debug"
            }
        },
        {
            "name": "Release",
            "inherits": "default",
            "cacheVariables": {
                "CMAKE_BUILD_TYPE": "Release"
            }
        }
    ],
    "buildPresets": [
        {
            "name": "Debug",
            "configurePreset": "Debug"
        },
        {
            "name": "Release",
            "configurePreset": "Release"
        }
    ]
}
"#
    .to_string()
}


#[cfg(test)]
mod tests;
