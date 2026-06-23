// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`crate::engine::bc_codegen`] generator.

#[cfg(not(test))]
compile_error!("This file is only to be used in test build");

use std::path::PathBuf;

use crate::device_definitions::adc::Adc;
use crate::device_definitions::bus::can::BusCan;
use crate::device_definitions::bus::i2c::BusI2c;
use crate::device_definitions::bus::spi::BusSpi;
use crate::device_definitions::bus::uart::BusUart;
use crate::device_definitions::bus::usb::BusUsb;
use crate::device_definitions::dac::Dac;
use crate::device_definitions::gpio::Gpio;
use crate::device_definitions::pwm::Pwm;
use crate::engine::bc_codegen::{write_bc_project, BC_OUTPUT_DIR_NAME};
use crate::engine::codegen::{INC_DIR_REL, SRC_DIR_REL};
use crate::engine::transport_select::TransportChoice;
use crate::engine::{build_artifacts, generate_rt};
use crate::ioc_parser::{DeviceSet, ProjectLayout};


/// Wide-coverage fixture — at least one slot per peripheral kind so the
/// generator stamps every method body.
fn rich_fixture() -> DeviceSet {
    let led    = Gpio::new("LED".to_string(),    "PA5".to_string(),  true);
    let button = Gpio::new("BTN".to_string(),    "PC13".to_string(), false);
    let adc_pin = Gpio::new("AIN".to_string(),   "PA0".to_string(),  false);
    let adc = Adc::new(adc_pin, 0, "AIN".to_string(), "hadc1".to_string());

    let dac_pin = Gpio::new("DOUT".to_string(),  "PA4".to_string(),  true);
    let dac = Dac::new(dac_pin, 1, "DOUT".to_string(), "hdac1".to_string());

    let pwm_pin = Gpio::new("PWM_OUT".to_string(), "PA6".to_string(), true);
    let pwm = Pwm::new(pwm_pin, 1, "PWM_OUT".to_string(), "htim3".to_string());

    let sck  = Gpio::new("SCK".to_string(),  "PB3".to_string(), true);
    let miso = Gpio::new("MISO".to_string(), "PB4".to_string(), false);
    let mosi = Gpio::new("MOSI".to_string(), "PB5".to_string(), true);
    let spi  = BusSpi::new(Vec::new(), Some(miso), Some(mosi), sck,
                           "SPI1".to_string(), "hspi1".to_string());

    let tx = Gpio::new("TX".to_string(),  "PA9".to_string(),  true);
    let rx = Gpio::new("RX".to_string(),  "PA10".to_string(), false);
    let uart_xport = BusUart::new(Some(tx), Some(rx),
                                  "USART1".to_string(), "huart1".to_string(),
                                  None);

    // Second UART — this one becomes a peripheral (not the transport).
    let tx2 = Gpio::new("TX2".to_string(), "PC10".to_string(), true);
    let rx2 = Gpio::new("RX2".to_string(), "PC11".to_string(), false);
    let uart_periph = BusUart::new(Some(tx2), Some(rx2),
                                   "USART3".to_string(), "huart3".to_string(),
                                   None);

    let sda = Gpio::new("SDA".to_string(), "PB7".to_string(), true);
    let scl = Gpio::new("SCL".to_string(), "PB6".to_string(), true);
    let i2c = BusI2c::new(sda, scl, "I2C1".to_string(), "hi2c1".to_string());

    let can_tx = Gpio::new("CAN_TX".to_string(), "PB9".to_string(),  true);
    let can_rx = Gpio::new("CAN_RX".to_string(), "PB8".to_string(),  false);
    let can = BusCan::new(can_tx, can_rx,
                          "CAN1".to_string(), "hcan1".to_string());

    DeviceSet {
        core: None,
        gpios: vec![led, button],
        spi_buses: vec![spi],
        uart_buses: vec![uart_xport, uart_periph],
        i2c_buses: vec![i2c],
        can_buses: vec![can],
        usb_buses: Vec::<BusUsb>::new(),
        adcs: vec![adc],
        dacs: vec![dac],
        pwms: vec![pwm],
        qspis: vec![],
        rtcs: vec![],
        layout: ProjectLayout::default(),
    }
}


fn rt_transport() -> TransportChoice {
    TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    }
}


/// Mirror of engine::tests::unique_tmp_dir. Kept local so the bc_codegen
/// tests don't reach into the parent tests module.
fn unique_tmp_dir(prefix: &str) -> PathBuf {
    let ramdisk = std::path::Path::new("/media/ramdisk");
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


/// t1
///
/// Verify `write_bc_project` writes every expected file (static + generated)
/// under the output root. Mirrors engine::tests::t4 but for the BC tree.
#[test]
fn t1() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t1");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .expect("write_bc_project");

    let project = tmp.join(BC_OUTPUT_DIR_NAME);
    let must_exist = [
        "CMakeLists.txt",
        "CMakePresets.json",
        "rt_client.h",
        "rt_client.cpp",
        "main.cpp",
        "api_fingerprint.h",
        "command_ids.h",
        "sub_addresses.h",
        "api/01_system.yaml",
        "common/crc.h",
        "common/crc.cpp",
        "communication/i_clock.h",
        "communication/i_frame_transport.h",
        "communication/i_sub_address_handler.h",
        "communication/message_header.h",
        "communication/message_header.cpp",
        "communication/scheduler.h",
        "communication/scheduler.cpp",
        "host/std_steady_clock.h",
        "host/std_steady_clock.cpp",
        "host/serial_frame_transport.h",
        "host/serial_frame_transport.cpp",
        "host/logger.h",
        "host/logger.cpp",
        "host/logging_frame_transport.h",
        "host/logging_frame_transport.cpp",
    ];
    for rel in must_exist.iter() {
        let p = project.join(rel);
        assert!(p.exists(), "expected '{}' to exist", p.display());
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t2
///
/// Verify idempotency: generating twice produces byte-identical
/// rt_client.{h,cpp}, main.cpp, and CMakeLists.txt. Catches any
/// non-deterministic iteration in the generator.
#[test]
fn t2() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t2");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();
    let h1   = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.h")).unwrap();
    let cpp1 = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.cpp")).unwrap();
    let ex1  = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("main.cpp")).unwrap();
    let mk1  = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("CMakeLists.txt")).unwrap();

    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();
    let h2   = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.h")).unwrap();
    let cpp2 = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.cpp")).unwrap();
    let ex2  = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("main.cpp")).unwrap();
    let mk2  = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("CMakeLists.txt")).unwrap();

    assert_eq!(h1,   h2);
    assert_eq!(cpp1, cpp2);
    assert_eq!(ex1,  ex2);
    assert_eq!(mk1,  mk2);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t3
///
/// Verify rt_client.h contains the expected method signatures for every
/// peripheral kind present in the fixture. Catches signature drift between
/// the plan and the generator.
#[test]
fn t3() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t3");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let h = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.h")).unwrap();
    let expected = [
        // GPIO output
        "Status set()",
        "Status clear()",
        // GPIO input
        "Status read(bool& value_out)",
        // ADC
        "Status sample(uint16_t& counts_out, uint32_t& timestamp_ms_out)",
        // DAC
        "Status set_counts(uint16_t counts)",
        // PWM
        "Status set_duty(uint16_t duty_hundredths)",
        // SPI / UART
        "Status tx(uint8_t const* data, uint16_t len)",
        "Status rx(uint16_t len, uint8_t* data_out)",
        // I2C
        "Status write_reg(uint8_t addr7, uint8_t reg, uint8_t const* data, uint8_t len)",
        "Status read_reg(uint8_t addr7, uint8_t reg, uint8_t len, uint8_t* data_out)",
        // CAN
        "Status tx_frame(uint32_t id, uint8_t dlc, const uint8_t data[8])",
        "Status rx_frame(uint32_t& id_out, uint8_t& dlc_out, uint8_t data_out[8])",
        // System handler (top-level)
        "Status get_api_fingerprint(uint32_t& fingerprint_out)",
        "Status get_device_count(uint16_t& count_out)",
        "Status get_sys_tick(uint32_t& ticks_out)",
        "Status get_core_device_id(uint32_t& devid_out, uint32_t& revid_out)",
        "Status get_cpu_id(uint32_t& uid0_out, uint32_t& uid1_out, uint32_t& uid2_out)",
        "Status read_memory(uint32_t address, uint16_t len, uint8_t* data_out)",
        "Status verify_fingerprint()",
    ];
    for sig in expected.iter() {
        assert!(h.contains(sig), "rt_client.h missing signature: {}", sig);
    }

    assert!(h.contains("char const* sub_address_name(uint8_t sub_address);"),
        "rt_client.h must declare sub_address_name for log resolution");

    let cpp = std::fs::read_to_string(
        tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.cpp")).unwrap();
    assert!(cpp.contains("char const* sub_address_name(uint8_t sub_address)\n{"),
        "rt_client.cpp must define sub_address_name");
    assert!(cpp.contains("return \"gpio_out_bank_A\";"),
        "sub_address_name must map a representative slot to its instance name");
    assert!(cpp.contains("\"op sa=%s %s -> %s\""),
        "log_op must format the sub-address by name");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t4
///
/// Verify rt_client.h declares one nested member per `SlotMember` (so
/// each GPIO pin in a bank slot still gets its own per-pin field) and
/// that the constructor initializer list in rt_client.cpp binds each one
/// to its slot's sub-address — with the extra `pin_index` argument for
/// GPIO members.
#[test]
fn t4() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t4");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let h   = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.h")).unwrap();
    let cpp = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.cpp")).unwrap();

    for slot in &art.slots {
        let sa_member = crate::engine::commands::to_pascal_case(&slot.instance_name);
        for m in &slot.members {
            assert!(h.contains(&format!(" {};", m.instance_name)),
                    "rt_client.h missing nested member '{}'", m.instance_name);
            let init_frag = match m.pin_index {
                Some(pin) => format!(
                    "{}(m_bc, m_clock, m_log, SubAddress::{}, {})",
                    m.instance_name, sa_member, pin),
                None => format!(
                    "{}(m_bc, m_clock, m_log, SubAddress::{})",
                    m.instance_name, sa_member),
            };
            assert!(cpp.contains(&init_frag),
                    "rt_client.cpp constructor missing init '{}'", init_frag);
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t5
///
/// Verify the BC tree's api_fingerprint.h, the per-sub-address manifest set,
/// and the generated id headers are byte-identical to the RT tree's copies
/// when both come from the same GeneratedArtifacts. This is the load-bearing
/// invariant for verify_fingerprint() to work.
#[test]
fn t5() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t5");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    generate_rt(&art, &transport, &tmp).unwrap();
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint).unwrap();

    let rt_fp = std::fs::read_to_string(tmp.join(INC_DIR_REL).join("api_fingerprint.h")).unwrap();
    let bc_fp = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join("api_fingerprint.h")).unwrap();

    // The wording in the comment block differs intentionally (RT says
    // "regenerated each time", BC says "stamped here at generation time"),
    // so we only compare the constant line.
    let rt_const = rt_fp.lines().find(|l| l.contains("kApiFingerprint")).unwrap();
    let bc_const = bc_fp.lines().find(|l| l.contains("kApiFingerprint")).unwrap();
    assert_eq!(rt_const, bc_const,
        "RT and BC must declare the same kApiFingerprint value");

    // The per-sub-address manifest set must match file-for-file.
    let rt_api = tmp.join(SRC_DIR_REL).join("api");
    let bc_api = tmp.join(BC_OUTPUT_DIR_NAME).join("api");
    let mut rt_names: Vec<_> = std::fs::read_dir(&rt_api).unwrap()
        .map(|e| e.unwrap().file_name()).collect();
    let mut bc_names: Vec<_> = std::fs::read_dir(&bc_api).unwrap()
        .map(|e| e.unwrap().file_name()).collect();
    rt_names.sort();
    bc_names.sort();
    assert_eq!(rt_names, bc_names, "RT and BC api/ file sets must match");
    for name in &rt_names {
        let rt_c = std::fs::read_to_string(rt_api.join(name)).unwrap();
        let bc_c = std::fs::read_to_string(bc_api.join(name)).unwrap();
        assert_eq!(rt_c, bc_c, "RT and BC api/{:?} must be byte-identical", name);
    }

    // The generated id headers are shared verbatim between the trees.
    for h in ["command_ids.h", "sub_addresses.h"] {
        let rt_h = std::fs::read_to_string(tmp.join(INC_DIR_REL).join(h)).unwrap();
        let bc_h = std::fs::read_to_string(tmp.join(BC_OUTPUT_DIR_NAME).join(h)).unwrap();
        assert_eq!(rt_h, bc_h, "RT and BC {} must be byte-identical", h);
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t6
///
/// Verify the generated CMakeLists.txt auto-discovers sources with file(GLOB)
/// over every BC source directory (including user/), and that each source
/// the project is expected to contain actually exists on disk.
#[test]
fn t6() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t6");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let project = tmp.join(BC_OUTPUT_DIR_NAME);
    let mk = std::fs::read_to_string(project.join("CMakeLists.txt")).unwrap();

    // Sources are globbed per directory, not enumerated. Confirm the file(GLOB)
    // discovery covers the project root plus each source subdirectory, so any
    // file dropped in (notably user/*.cpp) is built automatically.
    let expected_globs = [
        "${CMAKE_CURRENT_SOURCE_DIR}/*.cpp",
        "${CMAKE_CURRENT_SOURCE_DIR}/common/*.cpp",
        "${CMAKE_CURRENT_SOURCE_DIR}/communication/*.cpp",
        "${CMAKE_CURRENT_SOURCE_DIR}/host/*.cpp",
        "${CMAKE_CURRENT_SOURCE_DIR}/user/*.cpp",
    ];
    for glob in expected_globs.iter() {
        assert!(mk.contains(glob), "CMakeLists.txt missing source glob '{}'", glob);
    }

    // The globs only build what is on disk; confirm the generated/static
    // sources the wildcards are meant to pick up were actually written.
    let expected_sources = [
        "rt_client.cpp",
        "main.cpp",
        "common/crc.cpp",
        "communication/message_header.cpp",
        "communication/scheduler.cpp",
        "host/delay.cpp",
        "host/std_steady_clock.cpp",
        "host/serial_frame_transport.cpp",
        "host/logger.cpp",
        "host/logging_frame_transport.cpp",
    ];
    for src in expected_sources.iter() {
        assert!(project.join(src).exists(), "expected source '{}' not on disk", src);
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// smoke (ignored)
///
/// Manual smoke target: generates the full rich-fixture BC tree to a
/// stable path under /media/ramdisk and does NOT clean it up, so you can
/// `cd /media/ramdisk/exh_smoke_compile/exhibition_bc && cmake --preset Debug
/// && cmake --build --preset Debug` to confirm the C++ side actually builds.
/// Run with:
///
///     cargo test bc_codegen::tests::smoke -- --ignored --nocapture
#[test]
#[ignore]
fn smoke() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = PathBuf::from("/media/ramdisk/exh_smoke_compile");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("mkdir");
    generate_rt(&art, &transport, &tmp).unwrap();
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();
    println!("Wrote tree to {}", tmp.display());
}


/// t7
///
/// Verify main.cpp contains one example call per DeviceKind
/// present in the fixture. This is the user's copy-paste starting point —
/// if a kind is silently omitted, the user won't learn how to call it.
#[test]
fn t7() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t7");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let ex = std::fs::read_to_string(
        tmp.join(BC_OUTPUT_DIR_NAME).join("main.cpp")).unwrap();

    // Each fingerprint string is something only the per-kind example block
    // emits — picking them keeps the asserts tight.
    assert!(ex.contains(".set();"),          "GpioOutput example missing");
    assert!(ex.contains("delay(500u)"),      "LED delay example missing");
    assert!(ex.contains("#include \"host/delay.h\""),
        "example must include host/delay.h");
    assert!(ex.contains(".read(pin_value)"), "GpioInput example missing");
    assert!(ex.contains(".sample(adc_counts, adc_timestamp_ms)"), "Adc example missing");
    assert!(ex.contains(".set_counts(2048u)"),  "Dac example missing");
    assert!(ex.contains(".set_duty(5000u)"),    "Pwm example missing");
    assert!(ex.contains(".tx(tx, 2u)"),         "Spi tx example missing");
    assert!(ex.contains(".tx(msg, 3u)"),        "Uart tx example missing");
    assert!(ex.contains(".read_reg(0x68u"),     "I2c example missing");
    assert!(ex.contains(".rx_frame(id, dlc"),   "Can example missing");

    // The CLI-arg parsing skeleton must be intact.
    assert!(ex.contains("verify_fingerprint()"),
        "example must call verify_fingerprint()");
    assert!(ex.contains("SerialFrameTransport serial"),
        "example must instantiate SerialFrameTransport");
    assert!(ex.contains(
                "LoggingFrameTransport transport(serial, logger, &op_name, &sub_address_name)"),
        "example must wrap the transport in LoggingFrameTransport with both name resolvers");
    assert!(ex.contains("Logger logger(Logger::default_log_path())"),
        "example must construct a Logger");
    assert!(ex.contains("StdSteadyClock clock"),
        "example must instantiate StdSteadyClock");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t8
///
/// Verify sync_once() stores the RT's recommended offset directly and does no
/// BC-side smoothing of its own. The RT is the single source of truth for the
/// offset, so the generated client must assign it as-is (`m_offset = offset;`)
/// and must NOT apply a BC-side exponential filter such as `(old * 5 + new) / 6`.
#[test]
fn t8() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t8");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let cpp = std::fs::read_to_string(
        tmp.join(BC_OUTPUT_DIR_NAME).join("rt_client.cpp")).unwrap();

    assert!(cpp.contains("m_offset       = offset;"),
        "sync_once must store the RT's recommended offset directly");
    assert!(cpp.contains("m_path_delay   = path_delay;"),
        "sync_once must store the measured path delay directly");
    assert!(!cpp.contains("* 5.0"),
        "sync_once must not run a BC-side exponential filter");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t9
///
/// Verify the USER CODE preservation wiring end to end: after a user edits a
/// `USER CODE` region in the generated `main.cpp` (and adds a region the
/// template does not know about), regenerating the project keeps the edit,
/// keeps the generated framing/scaffolding, and relocates the unknown region to
/// the ORPHANED trailer instead of discarding it.
#[test]
fn t9() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t9");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let main_path = tmp.join(BC_OUTPUT_DIR_NAME).join("main.cpp");
    let generated = std::fs::read_to_string(&main_path).unwrap();

    // Simulate a user: drop a sentinel into the (empty) Init region and append
    // a region whose tag the template will never emit.
    let edited = generated.replace(
        "    /* USER CODE BEGIN Init */\n\n    /* USER CODE END Init */",
        "    /* USER CODE BEGIN Init */\n    sentinel_user_init();\n    /* USER CODE END Init */",
    );
    assert!(edited.contains("sentinel_user_init();"), "test setup: Init region not found");
    let edited = format!(
        "{edited}\n/* USER CODE BEGIN Bogus */\nsentinel_orphan();\n/* USER CODE END Bogus */\n"
    );
    std::fs::write(&main_path, &edited).unwrap();

    // Regenerate over the edited file.
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();
    let merged = std::fs::read_to_string(&main_path).unwrap();

    // User edit survived, inside intact generated framing.
    assert!(merged.contains("sentinel_user_init();"),
        "user edit in the Init region must survive regeneration");
    assert!(merged.contains("/* USER CODE BEGIN Init */"),
        "generated Init markers must remain");
    // Scaffolding outside the regions was refreshed (still present).
    assert!(merged.contains("verify_fingerprint()"),
        "generated scaffolding must be regenerated");
    // The unknown region was preserved in the trailer, not dropped.
    assert!(merged.contains("ORPHANED USER CODE"),
        "an unknown USER CODE region must be moved to the ORPHANED trailer");
    assert!(merged.contains("sentinel_orphan();"),
        "orphaned user content must be preserved, not discarded");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t10
///
/// Regression guard for the "regeneration must not erase user files" rule.
/// Generate the project, add a user/ source and a stray top-level file, then
/// regenerate: the user-owned files must survive untouched, and the
/// CMakeLists.txt must glob user/*.cpp so the added source is actually built.
#[test]
fn t10() {
    let devices = rich_fixture();
    let transport = rt_transport();
    let art = build_artifacts(&devices, &transport).unwrap();

    let tmp = unique_tmp_dir("exhibition_bc_t10");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    let project = tmp.join(BC_OUTPUT_DIR_NAME);

    // Stand in for hand-written code Exhibition does not produce: a source in a
    // user/ folder and an unrelated file at the project root.
    let user_src = project.join("user").join("sentinel.cpp");
    std::fs::create_dir_all(user_src.parent().unwrap()).unwrap();
    std::fs::write(&user_src, "void sentinel_user_code() {}\n").unwrap();
    let stray = project.join("KEEP_ME.txt");
    std::fs::write(&stray, "do not delete me\n").unwrap();

    // Regenerate over the existing project.
    write_bc_project(&tmp, &art.slots, &transport, &art.manifest_files, art.fingerprint)
        .unwrap();

    // Neither user-owned file may be removed or altered by regeneration.
    assert_eq!(std::fs::read_to_string(&user_src).unwrap(), "void sentinel_user_code() {}\n",
        "user/sentinel.cpp must survive regeneration unchanged");
    assert_eq!(std::fs::read_to_string(&stray).unwrap(), "do not delete me\n",
        "a stray user file must survive regeneration unchanged");

    // The regenerated CMakeLists.txt must auto-discover the user source.
    let mk = std::fs::read_to_string(project.join("CMakeLists.txt")).unwrap();
    assert!(mk.contains("${CMAKE_CURRENT_SOURCE_DIR}/user/*.cpp"),
        "CMakeLists.txt must glob user/*.cpp so user sources are built");

    let _ = std::fs::remove_dir_all(&tmp);
}
