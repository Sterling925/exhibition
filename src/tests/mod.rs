// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! End-to-end acceptance tests for the full Exhibition workflow.
//!
//! These tests treat the sibling CubeMX project in this directory
//! (`discovery_f4`) as a fixture. Each test copies the project to a
//! throw-away directory (preferring `/media/ramdisk`), runs the generator
//! into the copy exactly as the CLI would, performs the small `main.c`
//! integration a user would do by hand, and then checks the result:
//!
//! - [`t1`] verifies the generated RT files are present and correctly wired
//!   (always runs).
//! - [`t2`] cross-compiles the integrated RT project for ARM, and skips
//!   itself when the embedded toolchain (`arm-none-eabi-g++` + `ninja` +
//!   `cmake`) is not installed.
//! - [`t3`] generates the companion host-side BC project and builds it with
//!   the host toolchain, skipping the compile when `cmake`/`ninja` are absent.
//!
//! Choosing the transport interactively is out of scope; the tests build
//! the [`TransportChoice`] directly from the parsed device set.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::engine::bc_codegen::BC_OUTPUT_DIR_NAME;
use crate::engine::codegen::{INC_DIR_REL, SRC_DIR_REL};
use crate::engine::{
    build_artifacts, build_artifacts_logged, generate_bc, generate_rt_logged, TransportChoice,
};
use crate::gen_log::GenLog;
use crate::ioc_parser;

/// Absolute path to the fixture CubeMX project (this directory).
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/discovery_f4")
}

/// Build a fresh temporary directory under `/media/ramdisk` (per the
/// user's preference for ramdisk-backed temporary files), or fall back to
/// `std::env::temp_dir` if the ramdisk is unavailable.
fn unique_tmp_dir(prefix: &str) -> PathBuf {
    let ramdisk = Path::new("/media/ramdisk");
    let root: PathBuf = if ramdisk.is_dir() {
        ramdisk.to_path_buf()
    } else {
        std::env::temp_dir()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    root.join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
}

/// Recursively copy the CubeMX project from `src` into `dst`, producing a
/// clean C/C++ project. Skips this test module's own `.rs` files, any
/// `build` output directory, and `.git` so the copy is exactly what a user
/// would hand to CMake.
fn copy_project(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create destination dir");
    for entry in std::fs::read_dir(src).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let file_type = entry.file_type().expect("file type");

        if file_type.is_dir() {
            if name_str == "build" || name_str == ".git" {
                continue;
            }
            copy_project(&entry.path(), &dst.join(&name));
        } else {
            // Leave Rust sources (this module) out of the generated project.
            if name_str.ends_with(".rs") {
                continue;
            }
            std::fs::copy(entry.path(), dst.join(&name)).expect("copy file");
        }
    }
}

/// Perform the hand integration the INTEGRATION_GUIDE describes: include
/// the registry header and call `init_rt()` / `tick_rt()` at the CubeMX
/// `USER CODE` markers. `main.c` stays C — the generated header now carries
/// an `extern "C"` guard, so the C caller links against the C++ definitions.
fn patch_main_c(core_src_dir: &Path) {
    let main_c = core_src_dir.join("main.c");
    let original = std::fs::read_to_string(&main_c).expect("read main.c");

    let patched = original
        .replacen(
            "/* USER CODE BEGIN Includes */",
            "/* USER CODE BEGIN Includes */\n#include \"rt/device_registry.h\"",
            1,
        )
        .replacen(
            "/* USER CODE BEGIN 2 */",
            "/* USER CODE BEGIN 2 */\n  init_rt();",
            1,
        )
        .replacen(
            "/* USER CODE BEGIN 3 */",
            "/* USER CODE BEGIN 3 */\n    tick_rt();",
            1,
        );

    assert_ne!(
        patched, original,
        "main.c USER CODE markers were not found; integration would be a no-op"
    );
    std::fs::write(&main_c, patched).expect("write patched main.c");
}

/// Parse the fixture `.ioc` already copied into `root`, generate the RT into
/// it, and apply the `main.c` integration. Returns the shared artifacts.
fn generate_into(root: &Path) -> crate::engine::GeneratedArtifacts {
    let mut log = GenLog::new();
    let devices = ioc_parser::parse_path_logged(&root.join("discovery_f4.ioc"), &mut log)
        .expect("parse discovery_f4.ioc");

    let transport = uart_transport(&devices);
    let art = build_artifacts_logged(&devices, &transport, &mut log).expect("build_artifacts");
    generate_rt_logged(&art, &transport, root, &mut log).expect("generate_rt");
    patch_main_c(&root.join("Core/Src"));
    art
}

/// Build the RT transport directly from the fixture's USART2 bus. The
/// fixture's USB is host-mode, so UART is the only eligible transport;
/// selecting it interactively is out of scope for these tests.
fn uart_transport(devices: &crate::ioc_parser::DeviceSet) -> TransportChoice {
    let uart = devices
        .uart_buses
        .iter()
        .find(|b| b.name() == "USART2")
        .expect("fixture should expose USART2");
    TransportChoice::Uart {
        bus_name: uart.name().to_string(),
        hal_handle: uart.id().to_string(),
        baud_rate: uart.baud_rate(),
    }
}

/// Assert a path exists, with a message naming the missing file.
fn assert_exists(p: PathBuf) {
    assert!(p.exists(), "expected generated file to exist: {}", p.display());
}

/// Return true when `cmd --version` runs successfully (tool is on PATH).
fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/*
 * t1: Run the generator against the real discovery_f4 fixture and verify
 * the workflow output. Expects every always-present RT file to be written
 * under Core/Inc/rt and Core/Src/rt, the registry to be wired with an
 * extern "C" guard plus one handler instance per parsed device, the RT-side
 * fingerprint header to match the artifact fingerprint, and the patched
 * main.c to carry the include and both entry-point calls. Always runs (no
 * toolchain required).
 */
#[test]
fn t1() {
    let tmp = unique_tmp_dir("exhibition_accept_t1");
    copy_project(&fixture_root(), &tmp);

    let art = generate_into(&tmp);

    let inc = tmp.join(INC_DIR_REL);
    let src = tmp.join(SRC_DIR_REL);

    // Always-present generated files.
    assert_exists(inc.join("device_registry.h"));
    assert_exists(inc.join("api_fingerprint.h"));
    assert_exists(src.join("device_registry.cpp"));
    assert_exists(src.join("api/01_system.yaml"));
    assert_exists(inc.join("command_ids.h"));
    assert_exists(inc.join("sub_addresses.h"));
    assert_exists(src.join("INTEGRATION_GUIDE.md"));
    assert_exists(inc.join("communication/scheduler.h"));
    assert_exists(src.join("communication/scheduler.cpp"));
    assert_exists(inc.join("system/system_handler.h"));
    assert_exists(src.join("system/system_handler.cpp"));
    assert_exists(inc.join("transports/uart_frame_transport.h"));
    assert_exists(src.join("transports/uart_frame_transport.cpp"));
    assert_exists(inc.join("common/crc.h"));
    assert_exists(src.join("common/crc.cpp"));

    // The registry header must declare its entry points with C linkage so a
    // C main.c links against them.
    let header = std::fs::read_to_string(inc.join("device_registry.h")).unwrap();
    assert!(
        header.contains("extern \"C\""),
        "device_registry.h must wrap init_rt/tick_rt in extern \"C\""
    );

    // The registry source must register the reserved system handler and wire
    // one instance per parsed device slot.
    let registry = std::fs::read_to_string(src.join("device_registry.cpp")).unwrap();
    assert!(
        registry.contains("register_handler(static_cast<uint8_t>(SubAddress::System),"),
        "registry must register the system handler at the System sub-address"
    );
    assert!(!art.slots.is_empty(), "fixture should yield at least one device slot");
    for slot in &art.slots {
        let inst = format!("g_{}", slot.instance_name);
        assert!(
            registry.contains(&inst),
            "registry should instantiate {} for slot {}",
            inst, slot.sub_address
        );
    }

    // The RT-side fingerprint header must match the artifact fingerprint.
    let fp_header = std::fs::read_to_string(inc.join("api_fingerprint.h")).unwrap();
    let expected_fp = format!("0x{:08X}", art.fingerprint);
    assert!(
        fp_header.contains(&expected_fp),
        "api_fingerprint.h should contain {}", expected_fp
    );

    // The hand integration landed in main.c.
    let main_c = std::fs::read_to_string(tmp.join("Core/Src/main.c")).unwrap();
    assert!(main_c.contains("#include \"rt/device_registry.h\""));
    assert!(main_c.contains("init_rt();"));
    assert!(main_c.contains("tick_rt();"));

    // The generation report is written alongside the YAML manifests and
    // carries the same fingerprint plus the traceability sections.
    let report = std::fs::read_to_string(src.join("api/generation_report.md"))
        .expect("generation_report.md should exist");
    assert!(report.contains("# Exhibition RT Generation Report"));
    assert!(report.contains("## API fingerprint"));
    assert!(
        report.contains(&format!("0x{:08x}", art.fingerprint)),
        "report should embed the artifact fingerprint"
    );
    assert!(report.contains("## Devices enabled (and why)"));
    assert!(report.contains("## Code-generation decisions"));

    let _ = std::fs::remove_dir_all(&tmp);
}

/*
 * t2: Cross-compile the fully integrated project for ARM. Generates and
 * integrates as in t1, then runs `cmake --preset Debug` and
 * `cmake --build --preset Debug` and asserts the .elf is produced. Skips
 * itself (passing) when arm-none-eabi-g++, ninja, or cmake is not on PATH,
 * so `cargo test` stays green on machines without the embedded toolchain.
 */
#[test]
fn t2() {
    if !have("arm-none-eabi-g++") || !have("ninja") || !have("cmake") {
        eprintln!(
            "t2 skipped: ARM toolchain not found \
             (need arm-none-eabi-g++, ninja, and cmake on PATH)"
        );
        return;
    }

    let tmp = unique_tmp_dir("exhibition_accept_t2");
    copy_project(&fixture_root(), &tmp);
    generate_into(&tmp);

    let configure = Command::new("cmake")
        .current_dir(&tmp)
        .args(["--preset", "Debug"])
        .output()
        .expect("run cmake configure");
    assert!(
        configure.status.success(),
        "cmake configure failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&configure.stdout),
        String::from_utf8_lossy(&configure.stderr),
    );

    let build = Command::new("cmake")
        .current_dir(&tmp)
        .args(["--build", "--preset", "Debug"])
        .output()
        .expect("run cmake build");
    assert!(
        build.status.success(),
        "cmake build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    assert_exists(tmp.join("build/Debug/discovery_f4.elf"));

    let _ = std::fs::remove_dir_all(&tmp);
}

/*
 * t3: Generate the companion BC (host-side) project from the same fixture
 * .ioc and verify it builds. Expects the standalone exhibition_bc/ tree
 * (rt_client, main.cpp, CMakeLists.txt, CMakePresets.json, shared
 * communication/host/common sources) plus a BC api_fingerprint.h whose
 * fingerprint matches the shared artifact fingerprint, so the BC and RT agree
 * on the API. When the host toolchain (cmake + ninja) is present, configures
 * and builds with the Debug preset and asserts the exhibition_bc binary is
 * produced; otherwise skips the compile (passing) after the file/fingerprint
 * checks.
 */
#[test]
fn t3() {
    let tmp = unique_tmp_dir("exhibition_accept_t3");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    // BC is a standalone host project, so parse the fixture .ioc in place —
    // no need to copy the CubeMX tree.
    let devices = ioc_parser::parse_path(&fixture_root().join("discovery_f4.ioc"))
        .expect("parse discovery_f4.ioc");
    let transport = uart_transport(&devices);
    let art = build_artifacts(&devices, &transport).expect("build_artifacts");
    generate_bc(&art, &transport, &tmp).expect("generate_bc");

    let bc = tmp.join(BC_OUTPUT_DIR_NAME);

    // Expected BC project files (mirrors the CMakeLists.txt's globbed sources).
    assert_exists(bc.join("rt_client.h"));
    assert_exists(bc.join("rt_client.cpp"));
    assert_exists(bc.join("main.cpp"));
    assert_exists(bc.join("CMakeLists.txt"));
    assert_exists(bc.join("CMakePresets.json"));
    assert_exists(bc.join("api_fingerprint.h"));
    assert_exists(bc.join("common/crc.cpp"));
    assert_exists(bc.join("communication/message_header.cpp"));
    assert_exists(bc.join("communication/scheduler.cpp"));
    assert_exists(bc.join("host/std_steady_clock.cpp"));
    assert_exists(bc.join("host/serial_frame_transport.cpp"));
    assert_exists(bc.join("host/logger.cpp"));
    assert_exists(bc.join("host/logging_frame_transport.cpp"));

    // The BC fingerprint must match the shared artifact fingerprint so the
    // two sides reject a mismatched build at runtime.
    let fp = std::fs::read_to_string(bc.join("api_fingerprint.h")).unwrap();
    let expected_fp = format!("0x{:08X}", art.fingerprint);
    assert!(
        fp.contains(&expected_fp),
        "BC api_fingerprint.h should contain {}", expected_fp
    );

    // Build with the host toolchain when available.
    if !have("cmake") || !have("ninja") {
        eprintln!(
            "t3 compile skipped: host toolchain not found (need cmake and ninja on PATH)"
        );
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }

    let configure = Command::new("cmake")
        .current_dir(&bc)
        .args(["--preset", "Debug"])
        .output()
        .expect("run cmake configure");
    assert!(
        configure.status.success(),
        "BC cmake configure failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&configure.stdout),
        String::from_utf8_lossy(&configure.stderr),
    );

    let build = Command::new("cmake")
        .current_dir(&bc)
        .args(["--build", "--preset", "Debug"])
        .output()
        .expect("run cmake build");
    assert!(
        build.status.success(),
        "BC cmake build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    // Ninja single-config build writes the binary into the preset's binaryDir
    // (build/Debug); CMake appends the platform executable suffix on Windows.
    let bc_bin = bc.join("build/Debug").join(format!("exhibition_bc{}", std::env::consts::EXE_SUFFIX));
    assert_exists(bc_bin.clone());

    // Run the binary against a device that cannot open: it must fail to
    // connect yet still leave a timestamped log file (the Logger is built
    // before the port is opened), proving logging works end to end.
    //
    // Pin the log directory via the documented EXHIBITION_LOG_DIR override rather
    // than assuming where the logger writes: by default it prefers a ramdisk
    // (/media/ramdisk/exhibition_bc/logs) when one is present and only falls back to
    // <exe_dir>/logs otherwise, so the on-disk location is environment-dependent.
    let logs_dir = bc.join("test_logs");
    // A device path that cannot open, chosen per-platform so the open simply
    // fails (the test only asserts the failure path + that a log is written).
    let bogus_device = if cfg!(windows) {
        r"\\.\COM_exhibition_does_not_exist"
    } else {
        "/dev/this_device_does_not_exist_exhibition"
    };
    let run = Command::new(&bc_bin)
        .current_dir(bc.join("build/Debug"))
        .env("EXHIBITION_LOG_DIR", &logs_dir)
        .arg(bogus_device)
        .output()
        .expect("run exhibition_bc");
    assert!(
        !run.status.success(),
        "exhibition_bc should fail to open a nonexistent device"
    );
    let log_file = std::fs::read_dir(&logs_dir)
        .unwrap_or_else(|e| panic!("logs dir '{}' missing: {}", logs_dir.display(), e))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("exhibition_bc_") && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .expect("a timestamped exhibition_bc_*.log should exist");
    let log_text = std::fs::read_to_string(&log_file).unwrap();
    assert!(
        log_text.contains("exhibition_bc starting"),
        "log should contain the startup line; got:\n{}", log_text
    );
    assert!(
        log_text.contains("failed to open serial port"),
        "log should record the open failure; got:\n{}", log_text
    );

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t4
///
/// Unit-test `find_ioc_in_project` across its outcomes using throw-away
/// directories: zero `.ioc` files, exactly one (returned), a case-insensitive
/// `.IOC` match, two `.ioc` files (ambiguous -> None), a *directory* named like
/// an `.ioc` (must not count as a file match), and a non-existent path. Only
/// the exactly-one case yields `Some`.
#[test]
fn t4() {
    let base = unique_tmp_dir("exhibition_find_ioc");
    std::fs::create_dir_all(&base).expect("mkdir base");

    // Zero .ioc files -> None.
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(crate::find_ioc_in_project(&empty).is_none(), "empty dir must yield None");

    // Exactly one .ioc -> Some(that path); unrelated files are ignored.
    let one = base.join("one");
    std::fs::create_dir_all(&one).unwrap();
    let ioc = one.join("board.ioc");
    std::fs::write(&ioc, "x").unwrap();
    std::fs::write(one.join("readme.txt"), "x").unwrap();
    assert_eq!(crate::find_ioc_in_project(&one), Some(ioc));

    // Case-insensitive extension match.
    let upper = base.join("upper");
    std::fs::create_dir_all(&upper).unwrap();
    let upper_ioc = upper.join("board.IOC");
    std::fs::write(&upper_ioc, "x").unwrap();
    assert_eq!(crate::find_ioc_in_project(&upper), Some(upper_ioc));

    // Two .ioc files -> ambiguous -> None.
    let two = base.join("two");
    std::fs::create_dir_all(&two).unwrap();
    std::fs::write(two.join("a.ioc"), "x").unwrap();
    std::fs::write(two.join("b.ioc"), "x").unwrap();
    assert!(crate::find_ioc_in_project(&two).is_none(),
        "two .ioc files are ambiguous and must yield None");

    // A *directory* whose name ends in .ioc must not count as a file match.
    let dir_named = base.join("dir_named");
    std::fs::create_dir_all(&dir_named).unwrap();
    std::fs::create_dir_all(dir_named.join("nested.ioc")).unwrap();
    assert!(crate::find_ioc_in_project(&dir_named).is_none(),
        "a directory ending in .ioc is not a file match");

    // Non-existent path -> unreadable dir -> None.
    assert!(crate::find_ioc_in_project(&base.join("does_not_exist")).is_none(),
        "an unreadable path must yield None, not panic");

    let _ = std::fs::remove_dir_all(&base);
}


/// Recursively collect every file under `dir` whose extension marks it as a
/// source-like artifact we license (C/C++, Markdown, YAML). Used by [`t5`] to
/// sweep a generated tree for the SPDX header.
fn collect_licensed_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_licensed_files(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "h" | "hpp" | "c" | "cpp" | "md" | "yaml" | "yml") {
                out.push(path);
            }
        }
    }
}

/*
 * t5: Every Exhibition-generated source-like file carries the project SPDX
 * license header. Sweeps both generated trees — the RT files under Core/Inc/rt
 * and Core/Src/rt (rendered headers, copied templates, the YAML manifests, and
 * the Markdown guide/report) and the standalone exhibition_bc/ tree — and
 * asserts each C/C++/Markdown/YAML file contains the SPDX-License-Identifier
 * line. This guards against a future `render_*` function shipping output
 * without the header. Always runs (no toolchain required).
 */
#[test]
fn t5() {
    const SPDX: &str = "SPDX-License-Identifier: MIT OR Apache-2.0";

    // ---- RT tree ----
    let rt_tmp = unique_tmp_dir("exhibition_spdx_rt");
    copy_project(&fixture_root(), &rt_tmp);
    let _ = generate_into(&rt_tmp);

    let mut files = Vec::new();
    collect_licensed_files(&rt_tmp.join(INC_DIR_REL), &mut files);
    collect_licensed_files(&rt_tmp.join(SRC_DIR_REL), &mut files);

    // ---- BC tree ----
    let bc_tmp = unique_tmp_dir("exhibition_spdx_bc");
    std::fs::create_dir_all(&bc_tmp).expect("mkdir");
    let devices = ioc_parser::parse_path(&fixture_root().join("discovery_f4.ioc"))
        .expect("parse discovery_f4.ioc");
    let transport = uart_transport(&devices);
    let art = build_artifacts(&devices, &transport).expect("build_artifacts");
    generate_bc(&art, &transport, &bc_tmp).expect("generate_bc");
    collect_licensed_files(&bc_tmp.join(BC_OUTPUT_DIR_NAME), &mut files);

    assert!(!files.is_empty(), "expected to find generated source files to check");
    let missing: Vec<String> = files
        .iter()
        .filter(|p| !std::fs::read_to_string(p).map(|c| c.contains(SPDX)).unwrap_or(false))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "generated files missing the SPDX header:\n{}",
        missing.join("\n")
    );

    let _ = std::fs::remove_dir_all(&rt_tmp);
    let _ = std::fs::remove_dir_all(&bc_tmp);
}
