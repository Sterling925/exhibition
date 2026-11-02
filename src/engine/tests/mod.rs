// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`crate::engine`] code generator.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");

mod spdx_headers;


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
use crate::device_definitions::qspi::Qspi;
use crate::device_definitions::rtc::Rtc;
use crate::engine::addresses::{
    assign_sub_addresses, DeviceKind, FIRST_DEVICE_SUB_ADDRESS,
};
use crate::engine::codegen::{
    layout_is_modeled, source_dir_rel, INC_DIR_REL, SRC_DIR_REL,
};
use crate::engine::commands::to_pascal_case;
use crate::engine::manifest::{
    build_manifest_files, canonical_set, canonicalize, fingerprint_of_canonical,
    stamp_fingerprint_in_set, ManifestFile,
};
use crate::engine::transport_select::{
    select_transport_with_io, TransportChoice, UsbSpeed,
};
use crate::engine::{build_artifacts, generate_rt};
use crate::ioc_parser::{DeviceSet, ProjectLayout};


/// Build a minimal but representative `DeviceSet` for tests. Includes a
/// GPIO output, a GPIO input, an ADC channel, a SPI bus, and a UART bus
/// (the latter so we can pick it as the RT transport).
fn fixture_set() -> DeviceSet {
    let led = Gpio::new("LED".to_string(), "PA5".to_string(), true);
    let button = Gpio::new("BTN".to_string(), "PC13".to_string(), false);
    let adc_pin = Gpio::new("AIN".to_string(), "PA0".to_string(), false);
    let adc = Adc::new(adc_pin, 0, "AIN".to_string(), "hadc1".to_string());

    let sck = Gpio::new("SCK".to_string(), "PB3".to_string(), true);
    let miso = Gpio::new("MISO".to_string(), "PB4".to_string(), false);
    let mosi = Gpio::new("MOSI".to_string(), "PB5".to_string(), true);
    let spi = BusSpi::new(Vec::new(), Some(miso), Some(mosi), sck,
                          "SPI1".to_string(), "hspi1".to_string());

    let tx = Gpio::new("TX".to_string(), "PA9".to_string(), true);
    let rx = Gpio::new("RX".to_string(), "PA10".to_string(), false);
    let uart = BusUart::new(Some(tx), Some(rx),
                            "USART1".to_string(), "huart1".to_string(),
                            None);

    DeviceSet {
        core: None,
        gpios: vec![led, button],
        spi_buses: vec![spi],
        uart_buses: vec![uart],
        i2c_buses: Vec::<BusI2c>::new(),
        can_buses: Vec::<BusCan>::new(),
        usb_buses: Vec::<BusUsb>::new(),
        adcs: vec![adc],
        dacs: Vec::<Dac>::new(),
        pwms: Vec::<Pwm>::new(),
        qspis: vec![],
        rtcs: vec![],
        layout: ProjectLayout::default(),
    }
}


/// t1
///
/// Verify that `assign_sub_addresses` produces a deterministic, ordered
/// slot table excluding the UART chosen as the RT transport. Expected
/// ordering (by `"{kind_sort_key}::{key}"`): adc::hadc1_ch0,
/// gpio_in::bank_C, gpio_out::bank_A, spi::hspi1. Sub-addresses start at
/// `FIRST_DEVICE_SUB_ADDRESS`. GPIOs are bank-grouped, so the lone PA5
/// output and lone PC13 input collapse into one slot apiece, each
/// carrying a single `SlotMember` for the actual pin.
#[test]
fn t1() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1"))
        .expect("assignment should succeed");

    assert_eq!(slots.len(), 4, "USART1 should be excluded as transport");

    assert_eq!(slots[0].sub_address, FIRST_DEVICE_SUB_ADDRESS);
    assert_eq!(slots[0].kind, DeviceKind::Adc);
    assert_eq!(slots[0].sort_key, "adc::hadc1_ch0");

    assert_eq!(slots[1].sub_address, FIRST_DEVICE_SUB_ADDRESS + 1);
    assert_eq!(slots[1].kind, DeviceKind::GpioInput);
    assert_eq!(slots[1].sort_key, "gpio_in::bank_C");
    assert_eq!(slots[1].instance_name, "gpio_in_bank_C");
    assert_eq!(slots[1].members.len(), 1);
    assert_eq!(slots[1].members[0].instance_name, "gpio_PC13");
    assert_eq!(slots[1].members[0].pin_index, Some(13));

    assert_eq!(slots[2].sub_address, FIRST_DEVICE_SUB_ADDRESS + 2);
    assert_eq!(slots[2].kind, DeviceKind::GpioOutput);
    assert_eq!(slots[2].sort_key, "gpio_out::bank_A");
    assert_eq!(slots[2].instance_name, "gpio_out_bank_A");
    assert_eq!(slots[2].members.len(), 1);
    assert_eq!(slots[2].members[0].instance_name, "gpio_PA5");
    assert_eq!(slots[2].members[0].pin_index, Some(5));

    assert_eq!(slots[3].sub_address, FIRST_DEVICE_SUB_ADDRESS + 3);
    assert_eq!(slots[3].kind, DeviceKind::Spi);
    assert_eq!(slots[3].sort_key, "spi::hspi1");
}


/// t2
///
/// Verify that building the YAML and fingerprinting it twice on the same
/// inputs yields the same fingerprint (deterministic / stable hash).
#[test]
fn t2() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let files1 = build_manifest_files(&slots);
    let files2 = build_manifest_files(&slots);
    let fp1 = fingerprint_of_canonical(&canonical_set(&files1));
    let fp2 = fingerprint_of_canonical(&canonical_set(&files2));
    assert_eq!(fp1, fp2);
    assert_ne!(fp1, 0, "fingerprint should not be the seed value");
}


/// t3
///
/// Verify that adding a device to the set changes the fingerprint
/// (sensitivity check on the canonicalized hash).
#[test]
fn t3() {
    let devices = fixture_set();
    let slots_a = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let fp_a = fingerprint_of_canonical(&canonical_set(&build_manifest_files(&slots_a)));

    let mut devices_b = fixture_set();
    let extra = Gpio::new("EXTRA".to_string(), "PB0".to_string(), true);
    devices_b.gpios.push(extra);
    let slots_b = assign_sub_addresses(&devices_b, Some("USART1")).unwrap();
    let fp_b = fingerprint_of_canonical(&canonical_set(&build_manifest_files(&slots_b)));

    assert_ne!(fp_a, fp_b, "extra device should change the fingerprint");
}


/// t4
///
/// Verify that `engine::generate_rt` writes every expected file under a
/// temporary directory. The exact file set we look for is enough to catch
/// regressions in the template-embedding list, the registry generator, and
/// the manifest writer.
#[test]
fn t4() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t4");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).expect("build_artifacts");
    generate_rt(&art, &transport, &tmp).expect("generate_rt should succeed");

    let inc_root = tmp.join(INC_DIR_REL);
    let src_root = tmp.join(SRC_DIR_REL);
    let must_exist_inc = [
        "communication/message_header.h",
        "communication/scheduler.h",
        "communication/i_sub_address_handler.h",
        "communication/phase_anchor.h",
        "common/crc.h",
        "system/system_handler.h",
        "devices/gpio_output_handler.h",
        "devices/gpio_input_handler.h",
        "devices/adc_handler.h",
        "devices/spi_handler.h",
        "transports/sys_tick_clock.h",
        "transports/uart_frame_transport.h",
        "device_registry.h",
        "api_fingerprint.h",
        "command_ids.h",
        "sub_addresses.h",
    ];
    for rel in must_exist_inc.iter() {
        let p = inc_root.join(rel);
        assert!(p.exists(), "expected '{}' to exist", p.display());
    }
    let must_exist_src = [
        "common/crc.cpp",
        "transports/uart_frame_transport.cpp",
        "device_registry.cpp",
        "api/01_system.yaml",
    ];
    for rel in must_exist_src.iter() {
        let p = src_root.join(rel);
        assert!(p.exists(), "expected '{}' to exist", p.display());
    }

    // The manifest is split per sub-address; concatenate the set for the
    // content checks so they stay decoupled from sub-address numbering.
    let mut all_yaml = String::new();
    for entry in std::fs::read_dir(src_root.join("api")).unwrap() {
        all_yaml.push_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    assert!(all_yaml.contains("fingerprint: 0x"), "stamped fingerprint");
    assert!(all_yaml.contains("name: gpio_PA5"), "named GPIO output member");
    assert!(all_yaml.contains("name: system"), "system handler entry");

    let cmd_ids = std::fs::read_to_string(inc_root.join("command_ids.h")).unwrap();
    assert!(cmd_ids.contains("enum class GpioOutputCmd : uint8_t"),
            "command_ids.h defines the per-kind enums");

    let fp_header = std::fs::read_to_string(inc_root.join("api_fingerprint.h")).unwrap();
    assert!(fp_header.contains("constexpr uint32_t kApiFingerprint"));
    let registry = std::fs::read_to_string(src_root.join("device_registry.cpp")).unwrap();
    assert!(registry.contains("UartFrameTransport g_transport(&huart1)"),
            "registry uses chosen transport");
    assert!(registry.contains(
                "register_handler(static_cast<uint8_t>(SubAddress::System), &g_system_handler,"),
            "registry binds system to the System sub-address (with its service hash)");
    assert!(registry.contains("g_system_handler.set_rx_time_source(&g_rt);"),
            "registry wires the RT header arrival time as the sync T2 source");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t5
///
/// Verify that running `engine::generate_rt` twice into the same directory
/// produces byte-identical output (idempotency). Catches non-deterministic
/// ordering and timestamping bugs.
#[test]
fn t5() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t5");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();
    let yaml1 = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("api/01_system.yaml")).unwrap();
    let registry1 = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("device_registry.cpp")).unwrap();

    generate_rt(&art, &transport, &tmp).unwrap();
    let yaml2 = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("api/01_system.yaml")).unwrap();
    let registry2 = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("device_registry.cpp")).unwrap();

    assert_eq!(yaml1, yaml2);
    assert_eq!(registry1, registry2);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t6
///
/// Verify that `select_transport_with_io` rejects an empty transport list
/// and accepts a valid 1-based selection. Covers both error paths in one
/// test.
#[test]
fn t6() {
    let mut empty = fixture_set();
    empty.uart_buses.clear();
    empty.usb_buses.clear();
    let err = select_transport_with_io(&empty, b"1\n".as_ref(), Vec::new())
        .expect_err("no eligible buses should be an error");
    assert!(err.contains("No eligible transport"), "got: {}", err);

    let devices = fixture_set();
    let mut out = Vec::new();
    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), &mut out)
        .expect("should accept choice 1");
    assert_eq!(choice.bus_name(), "USART1");
    assert_eq!(choice.hal_handle(), Some("huart1"));
    assert_eq!(choice.kind(), "uart");
    let out_s = String::from_utf8(out).unwrap();
    assert!(out_s.contains("USART1"));
}


/// t7
///
/// Verify that fingerprint stamping replaces only the placeholder line (in the
/// system file) and that the canonical form of the whole set is invariant
/// under the fingerprint value — the canonicalizer drops the `fingerprint:`
/// line so the hash does not depend on itself.
#[test]
fn t7() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let mut files = build_manifest_files(&slots);

    let canon_before = canonical_set(&files);
    let fp = fingerprint_of_canonical(&canon_before);
    stamp_fingerprint_in_set(&mut files, fp);

    let sys = files.iter().find(|f| f.file_name == "01_system.yaml").unwrap();
    assert!(sys.content.contains(&format!("fingerprint: 0x{:08X}", fp)));
    assert!(!sys.content.contains("fingerprint: 0x00000000"));

    let canon_after = canonical_set(&files);
    assert_eq!(canon_before, canon_after,
        "canonical form should be invariant under fingerprint value");
}


/// t8
///
/// Verify that exceeding 30 peripherals returns an error rather than
/// silently overflowing into reserved sub-addresses. Uses PWMs as the
/// inflater because GPIO bank grouping caps GPIO slot count at
/// `banks * directions` (max 22), so GPIOs alone cannot overflow.
#[test]
fn t8() {
    let mut devices = fixture_set();
    // Drop the UART so we don't run the transport-exclusion path; we want
    // to test the count check directly.
    devices.uart_buses.clear();
    devices.gpios.clear();
    devices.adcs.clear();
    devices.spi_buses.clear();
    devices.pwms.clear();
    for i in 0..31 {
        let pin = Gpio::new(format!("PWM_PIN_{}", i), "PA0".to_string(), true);
        // Unique hal_handle / channel pairs so each PWM gets its own slot.
        let pwm = Pwm::new(pin, 1, format!("PWM_{}", i), format!("htim{}", i));
        devices.pwms.push(pwm);
    }
    let err = assign_sub_addresses(&devices, None)
        .expect_err("31 peripherals should overflow the 30-slot range");
    assert!(err.contains("Too many"), "got: {}", err);
}


/// t9
///
/// Verify that `generate_rt` emits the auto-generated `INTEGRATION_GUIDE.md`
/// inside `exhibition_rt/` and substitutes the chosen transport handle, the
/// transport bus name, the registered GPIO output label, and the sub-address
/// table. Also confirm `communication/README.md` is shipped alongside it.
#[test]
fn t9() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t9");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let src_root = tmp.join(SRC_DIR_REL);

    // Integration guide is present and tailored to the chosen transport.
    let guide_path = src_root.join("INTEGRATION_GUIDE.md");
    let guide = std::fs::read_to_string(&guide_path)
        .expect("INTEGRATION_GUIDE.md should have been written");
    assert!(guide.contains("huart1"), "guide should name the HAL handle");
    assert!(guide.contains("USART1"), "guide should name the transport bus");
    // Fixture has a GPIO output labelled "LED" — the LED-toggle section
    // must reference it concretely.
    assert!(guide.contains("LED"), "guide should reference the LED label");
    assert!(guide.contains("set()") && guide.contains("clear()"),
        "guide should show the BC set()/clear() example");
    // Sub-address table reserves 1 for the system handler.
    assert!(guide.contains("| 1 | System"), "guide should list the system row");
    // Sanity: it includes the init_rt/tick_rt code block.
    assert!(guide.contains("init_rt()") && guide.contains("tick_rt()"),
        "guide should show both entry-point calls");
    // The RT uses `if constexpr`, so the guide must tell the user to build as C++17.
    assert!(guide.contains("C++17"),
        "guide should instruct building as C++17");

    // The protocol deep-dive is shipped too.
    let proto_spec = src_root.join("communication").join("protocol_t1.md");
    assert!(proto_spec.exists(),
        "communication/protocol_t1.md should be copied into the generated tree");
    let proto_t2 = src_root.join("communication").join("protocol_t2.md");
    assert!(proto_t2.exists(),
        "communication/protocol_t2.md should be copied into the generated tree");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t10
///
/// Verify that multiple GPIO pins on the same bank collapse into one
/// slot whose `pin_mask` is the bitwise OR of each pin's bit, and whose
/// `members` are sorted by pin index. Mixing two GPIO outputs on bank A
/// (PA5, PA7) plus one input on bank A (PA3) must yield two slots
/// (output bank A and input bank A) — one per direction — not three.
#[test]
fn t10() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.spi_buses.clear();
    devices.adcs.clear();
    devices.gpios.clear();
    // Bank A: two outputs and one input.
    devices.gpios.push(Gpio::new("OUT_A5".to_string(), "PA5".to_string(), true));
    devices.gpios.push(Gpio::new("OUT_A7".to_string(), "PA7".to_string(), true));
    devices.gpios.push(Gpio::new("IN_A3".to_string(),  "PA3".to_string(), false));

    let slots = assign_sub_addresses(&devices, None).unwrap();
    assert_eq!(slots.len(), 2, "one slot per (direction, bank)");

    let out_slot = slots.iter().find(|s| s.kind == DeviceKind::GpioOutput)
        .expect("output bank A slot");
    let in_slot = slots.iter().find(|s| s.kind == DeviceKind::GpioInput)
        .expect("input bank A slot");

    // Pin mask = (1<<5) | (1<<7) = 0x00A0 for outputs; 1<<3 = 0x0008 for input.
    let out_mask = out_slot.details.iter()
        .find(|(k, _)| k == "pin_mask").map(|(_, v)| v.as_str()).unwrap();
    let in_mask = in_slot.details.iter()
        .find(|(k, _)| k == "pin_mask").map(|(_, v)| v.as_str()).unwrap();
    assert_eq!(out_mask, "0x00A0");
    assert_eq!(in_mask,  "0x0008");

    // Members are sorted by pin index (5 then 7).
    assert_eq!(out_slot.members.len(), 2);
    assert_eq!(out_slot.members[0].pin_index, Some(5));
    assert_eq!(out_slot.members[1].pin_index, Some(7));
    assert_eq!(out_slot.members[0].instance_name, "gpio_PA5");
    assert_eq!(out_slot.members[1].instance_name, "gpio_PA7");
    assert_eq!(in_slot.members.len(), 1);
    assert_eq!(in_slot.members[0].pin_index, Some(3));
}


/// t11
///
/// Verify that GPIO outputs spread across multiple banks produce one
/// slot per bank in deterministic, lexicographic bank order (A before B
/// before C).
#[test]
fn t11() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.spi_buses.clear();
    devices.adcs.clear();
    devices.gpios.clear();
    // Intentionally pushed out of order; the sort step should normalize.
    devices.gpios.push(Gpio::new("LED_C0".to_string(), "PC0".to_string(), true));
    devices.gpios.push(Gpio::new("LED_A0".to_string(), "PA0".to_string(), true));
    devices.gpios.push(Gpio::new("LED_B0".to_string(), "PB0".to_string(), true));

    let slots = assign_sub_addresses(&devices, None).unwrap();
    let banks: Vec<&str> = slots.iter()
        .filter(|s| s.kind == DeviceKind::GpioOutput)
        .map(|s| s.instance_name.as_str())
        .collect();
    assert_eq!(banks, vec!["gpio_out_bank_A", "gpio_out_bank_B", "gpio_out_bank_C"]);
}


/// t12
///
/// Verify that two runs of the generator over identical inputs produce
/// the same slot vector, including the same `members` order within each
/// slot. Catches non-determinism that would surface as a churning
/// fingerprint between runs.
#[test]
fn t12() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.spi_buses.clear();
    devices.adcs.clear();
    devices.gpios.clear();
    for (name, id, out) in [
        ("LED1", "PA5", true), ("LED2", "PA1", true), ("LED3", "PA10", true),
        ("BTN1", "PB0", false), ("BTN2", "PB12", false),
    ] {
        devices.gpios.push(Gpio::new(name.to_string(), id.to_string(), out));
    }
    let s1 = assign_sub_addresses(&devices, None).unwrap();
    let s2 = assign_sub_addresses(&devices, None).unwrap();
    assert_eq!(s1.len(), s2.len());
    for (a, b) in s1.iter().zip(s2.iter()) {
        assert_eq!(a.sub_address,   b.sub_address);
        assert_eq!(a.sort_key,      b.sort_key);
        assert_eq!(a.instance_name, b.instance_name);
        assert_eq!(a.members.len(), b.members.len());
        for (ma, mb) in a.members.iter().zip(b.members.iter()) {
            assert_eq!(ma.instance_name, mb.instance_name);
            assert_eq!(ma.pin_index,     mb.pin_index);
        }
    }
}


/// t13
///
/// Verify the YAML manifest emits per-pin entries under each GPIO bank
/// slot. The `name: gpio_PA5` substring must still be present (it's how
/// the registry / docs reference an individual pin), even though the
/// slot's own name is now bank-level.
#[test]
fn t13() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let all: String = build_manifest_files(&slots)
        .iter()
        .map(|f| f.content.clone())
        .collect();
    assert!(all.contains("name: gpio_out_bank_A"),
        "bank-level slot name missing");
    assert!(all.contains("name: gpio_PA5"),
        "per-pin member name should appear inside the bank's pins: list");
    assert!(all.contains("pin_idx: 5"),
        "per-pin index should be emitted");
    assert!(all.contains("pin_mask: 0x0020"),
        "per-bank pin_mask should reflect just PA5 (bit 5 = 0x0020)");
}


/// t14
///
/// Verify the generated `device_registry.cpp` declares one bank handler
/// per (direction, bank) with the correct port and `pin_mask` literal,
/// and that `init_rt` registers the bank handler at the bank's
/// sub-address.
#[test]
fn t14() {
    let mut devices = fixture_set();
    // Add a second output on bank A so we exercise mask aggregation.
    devices.gpios.push(Gpio::new("LED2".to_string(), "PA9".to_string(), true));
    // Also a second input on bank C so both directions on one bank show up.
    devices.gpios.push(Gpio::new("BTN2".to_string(), "PC14".to_string(), false));

    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t14");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let registry = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("device_registry.cpp")
    ).unwrap();

    // PA5 = bit 5 (0x20), PA9 = bit 9 (0x200). OR = 0x0220.
    assert!(registry.contains(
        "static GpioOutputBankHandler g_gpio_out_bank_A(GPIOA, 0x0220, g_clock);"),
        "missing GPIOA output bank with mask 0x0220; registry was:\n{}", registry);
    // PC13 = bit 13 (0x2000), PC14 = bit 14 (0x4000). OR = 0x6000.
    assert!(registry.contains(
        "static GpioInputBankHandler g_gpio_in_bank_C(GPIOC, 0x6000, g_clock);"),
        "missing GPIOC input bank with mask 0x6000; registry was:\n{}", registry);

    // Bank handlers must be registered via their named SubAddress enumerator.
    // The enumerator name is derived from the slot's instance name, so the
    // assertion stays decoupled from the concrete sub-address numbering.
    assert!(registry.contains(&format!(
        "g_rt.register_handler(static_cast<uint8_t>(SubAddress::{}), &g_gpio_out_bank_A,",
        to_pascal_case("gpio_out_bank_A"))));
    assert!(registry.contains(&format!(
        "g_rt.register_handler(static_cast<uint8_t>(SubAddress::{}), &g_gpio_in_bank_C,",
        to_pascal_case("gpio_in_bank_C"))));

    let _ = std::fs::remove_dir_all(&tmp);
}


/// Build a `BusUsb` suitable for picker / codegen tests. `name` should
/// match what the parser would emit (`USB_OTG_HS`, `USB_OTG_FS`, or `USB`)
/// so the speed-derivation logic exercises the real string path.
fn make_usb(name: &str, id: &str) -> BusUsb {
    let dp = Gpio::new(format!("{}_DP", name), "PA12".to_string(), true);
    let dm = Gpio::new(format!("{}_DM", name), "PA11".to_string(), true);
    BusUsb::new(dp, dm, None, None, name.to_string(), id.to_string())
}


/// t15
///
/// Verify that `select_transport_with_io` exposes USB buses as eligible
/// transports and returns a `TransportChoice::UsbCdc` for an `USB_OTG_HS`
/// peripheral, with `speed = Hs` derived from the bus name. Also confirms
/// the menu wording mentions the speed suffix so the user can tell HS from
/// FS at the picker prompt.
#[test]
fn t15() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();   // only USB should be offered
    devices.usb_buses.push(make_usb("USB_OTG_HS", "husb_otg_hs"));

    let mut out = Vec::new();
    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), &mut out)
        .expect("USB bus should be an eligible transport");
    match &choice {
        TransportChoice::UsbCdc { bus_name, speed } => {
            assert_eq!(bus_name, "USB_OTG_HS");
            assert_eq!(*speed, UsbSpeed::Hs);
        }
        other => panic!("expected UsbCdc variant, got {:?}", other),
    }
    assert_eq!(choice.kind(), "usb_cdc");
    assert_eq!(choice.hal_handle(), None, "USB transport has no HAL handle");
    assert_eq!(
        choice.transmit_fn_name().as_deref(),
        Some("CDC_Transmit_HS"),
        "HS speed must map to the _HS transmit function",
    );

    let out_s = String::from_utf8(out).unwrap();
    assert!(out_s.contains("USB_OTG_HS"));
    assert!(out_s.contains("USB CDC HS"), "menu should mention the speed; got: {}", out_s);
}


/// t16
///
/// Verify that a `DeviceSet` containing both a UART and a USB bus lists
/// UARTs first (entry 1) and USBs second (entry 2). Selecting "2" must
/// return the `UsbCdc` variant; selecting "1" must return the `Uart`
/// variant. The picker's ordering is stable, so re-runs are deterministic.
#[test]
fn t16() {
    let mut devices = fixture_set();   // already has USART1
    devices.usb_buses.push(make_usb("USB_OTG_HS", "husb_otg_hs"));

    // Entry 1 should be the UART.
    let mut out1 = Vec::new();
    let c1 = select_transport_with_io(&devices, b"1\n".as_ref(), &mut out1).unwrap();
    assert!(matches!(c1, TransportChoice::Uart { .. }), "entry 1 must be UART");
    assert_eq!(c1.bus_name(), "USART1");

    // Entry 2 should be the USB.
    let mut out2 = Vec::new();
    let c2 = select_transport_with_io(&devices, b"2\n".as_ref(), &mut out2).unwrap();
    assert!(matches!(c2, TransportChoice::UsbCdc { .. }), "entry 2 must be USB");
    assert_eq!(c2.bus_name(), "USB_OTG_HS");
}


/// t17
///
/// Verify that a plain `USB` peripheral (no `_HS` / `_FS` qualifier in the
/// bus name — typical for smaller STM32s that only have one USB block)
/// resolves to `UsbSpeed::Fs` and emits `CDC_Transmit_FS`. Default-to-FS is
/// the safe behavior because every CubeMX-supported USB peripheral has at
/// least an FS endpoint.
#[test]
fn t17() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.usb_buses.push(make_usb("USB", "husb"));

    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), Vec::new())
        .expect("USB should be eligible");
    match &choice {
        TransportChoice::UsbCdc { bus_name, speed } => {
            assert_eq!(bus_name, "USB");
            assert_eq!(*speed, UsbSpeed::Fs, "plain USB must default to FS");
        }
        other => panic!("expected UsbCdc variant, got {:?}", other),
    }
    assert_eq!(choice.transmit_fn_name().as_deref(), Some("CDC_Transmit_FS"));
}


/// t18
///
/// Verify that `generate_rt` with a USB CDC HS transport writes the
/// USB-specific transport files, emits the correct `extern "C"` glue in
/// `device_registry.cpp` (`CDC_Transmit_HS` declaration, transport
/// instantiation, and the `exhibition_usb_rx` forwarder), and does *not*
/// emit the UART transport files, exercising the per-transport conditional
/// emission in the codegen branch.
#[test]
fn t18() {
    let devices = fixture_set();   // GPIO + ADC + SPI + USART1 left in
    let transport = TransportChoice::UsbCdc {
        bus_name: "USB_OTG_HS".to_string(),
        speed: UsbSpeed::Hs,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t18");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).expect("build_artifacts");
    generate_rt(&art, &transport, &tmp).expect("generate_rt should succeed");

    let inc_root = tmp.join(INC_DIR_REL);
    let src_root = tmp.join(SRC_DIR_REL);

    // USB transport files written, UART transport files absent.
    assert!(inc_root.join("transports/usb_cdc_frame_transport.h").exists());
    assert!(src_root.join("transports/usb_cdc_frame_transport.cpp").exists());
    assert!(!inc_root.join("transports/uart_frame_transport.h").exists(),
        "UART transport files should not appear when USB is selected");
    assert!(!src_root.join("transports/uart_frame_transport.cpp").exists());

    let registry = std::fs::read_to_string(src_root.join("device_registry.cpp")).unwrap();
    assert!(
        registry.contains("#include \"rt/transports/usb_cdc_frame_transport.h\""),
        "registry must include the USB transport header; got:\n{}", registry,
    );
    assert!(
        registry.contains("extern \"C\" uint8_t CDC_Transmit_HS(uint8_t* buf, uint16_t len);"),
        "registry must declare CDC_Transmit_HS; got:\n{}", registry,
    );
    assert!(
        registry.contains("static UsbCdcFrameTransport g_transport(CDC_Transmit_HS);"),
        "registry must instantiate the USB transport with the HS transmit fn; got:\n{}", registry,
    );
    assert!(
        registry.contains("extern \"C\" void exhibition_usb_rx"),
        "registry must export the RX forwarder; got:\n{}", registry,
    );
    assert!(
        registry.contains("g_transport.deposit_rx(buf, len);"),
        "RX forwarder must call deposit_rx; got:\n{}", registry,
    );

    // Selecting USB as the transport must NOT shadow the USART1 peripheral
    // — USB isn't in the peripheral assignment loop, so USART1 still gets
    // a sub-address as a regular UART device.
    assert!(art.slots.iter().any(|s| s.instance_name.starts_with("uart_huart1")),
        "USART1 should still be assigned as a peripheral when USB is the transport");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t19
///
/// Verify the FS speed path: a `UsbSpeed::Fs` transport emits
/// `CDC_Transmit_FS` (not `_HS`) in the registry. Together with t18 this
/// covers both speed branches of the codegen.
#[test]
fn t19() {
    let devices = fixture_set();
    let transport = TransportChoice::UsbCdc {
        bus_name: "USB".to_string(),
        speed: UsbSpeed::Fs,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t19");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let registry = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("device_registry.cpp")
    ).unwrap();
    assert!(registry.contains("CDC_Transmit_FS"),
        "FS speed must emit CDC_Transmit_FS; got:\n{}", registry);
    assert!(!registry.contains("CDC_Transmit_HS"),
        "FS speed must not emit CDC_Transmit_HS; got:\n{}", registry);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t20
///
/// Verify the USB-specific sections of `INTEGRATION_GUIDE.md`: it must
/// mention USB CDC, name the bus and speed, include the `usbd_cdc_if.c`
/// hook snippet (`extern void exhibition_usb_rx` and the `CDC_Receive_HS`
/// function it goes inside), and reference `/dev/ttyACM0` as the BC port.
/// It must NOT reference `MX_<bus>_UART_Init()` since the transport is USB.
#[test]
fn t20() {
    let devices = fixture_set();
    let transport = TransportChoice::UsbCdc {
        bus_name: "USB_OTG_HS".to_string(),
        speed: UsbSpeed::Hs,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t20");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let guide = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("INTEGRATION_GUIDE.md")
    ).unwrap();

    assert!(guide.contains("USB_OTG_HS"), "guide must name the USB bus");
    assert!(guide.contains("USB CDC"), "guide must mention USB CDC");
    assert!(guide.contains("MX_USB_DEVICE_Init()"),
        "guide must instruct calling MX_USB_DEVICE_Init()");
    assert!(!guide.contains("MX_USB_OTG_HS_UART_Init"),
        "guide must not reference a UART init for a USB transport");
    assert!(guide.contains("CDC_Receive_HS"),
        "guide must reference the HS receive callback");
    assert!(guide.contains("exhibition_usb_rx"),
        "guide must reference the C hook");
    assert!(guide.contains("/dev/ttyACM0"),
        "guide must reference the Linux CDC device path");
    assert!(guide.contains("usbd_cdc_if.c"),
        "guide must point at the file to edit");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t21
///
/// Verify that the picker propagates a UART bus's `BusUart::baud_rate`
/// into the returned `TransportChoice::Uart::baud_rate` so downstream
/// codegen can read it without re-querying the DeviceSet.
#[test]
fn t21() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    let tx = Gpio::new("TX".to_string(),  "PA9".to_string(),  true);
    let rx = Gpio::new("RX".to_string(),  "PA10".to_string(), false);
    devices.uart_buses.push(BusUart::new(
        Some(tx), Some(rx),
        "USART1".to_string(), "huart1".to_string(),
        Some(57600),
    ));

    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), Vec::new())
        .expect("should accept choice 1");
    match &choice {
        TransportChoice::Uart { baud_rate, .. } => {
            assert_eq!(*baud_rate, Some(57600));
        }
        other => panic!("expected Uart variant, got {:?}", other),
    }
    assert_eq!(choice.baud_rate(), Some(57600));
}


/// t22
///
/// Verify that a UART with no `BusUart::baud_rate` (e.g. the IOC omitted
/// `<PERIPH>.BaudRate`) propagates as `None` through the picker, so
/// downstream codegen can decide whether to emit the "fallback baud" note.
#[test]
fn t22() {
    let devices = fixture_set();   // fixture's UART is built with None
    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), Vec::new())
        .expect("should accept choice 1");
    assert_eq!(choice.baud_rate(), None);
}


/// t23
///
/// Verify that `generate_rt` substitutes the known baud rate into
/// `INTEGRATION_GUIDE.md`'s verification commands and does not emit the
/// "did not specify a baud rate" fallback note.
#[test]
fn t23() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    let tx = Gpio::new("TX".to_string(),  "PA9".to_string(),  true);
    let rx = Gpio::new("RX".to_string(),  "PA10".to_string(), false);
    devices.uart_buses.push(BusUart::new(
        Some(tx), Some(rx),
        "USART1".to_string(), "huart1".to_string(),
        Some(38400),
    ));
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: Some(38400),
    };
    let tmp = unique_tmp_dir("exhibition_engine_t23");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let guide = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("INTEGRATION_GUIDE.md")
    ).unwrap();

    assert!(guide.contains("/dev/ttyUSB0 38400"),
        "guide must show real baud in Linux command; got:\n{}", guide);
    assert!(guide.contains("COM7 38400"),
        "guide must show real baud in Windows command; got:\n{}", guide);
    assert!(!guide.contains("did not specify a baud rate"),
        "guide must not emit fallback note when baud is known");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t24
///
/// Verify that `generate_rt` falls back to 115200 in
/// `INTEGRATION_GUIDE.md` AND emits the "did not specify a baud rate"
/// blockquote when the chosen UART has no baud rate configured.
#[test]
fn t24() {
    let devices = fixture_set();   // fixture UART has baud_rate = None
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t24");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let guide = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("INTEGRATION_GUIDE.md")
    ).unwrap();

    assert!(guide.contains("/dev/ttyUSB0 115200"),
        "guide must show fallback baud when none configured; got:\n{}", guide);
    assert!(guide.contains("did not specify a baud rate"),
        "guide must emit fallback note when baud is unknown; got:\n{}", guide);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t25
///
/// Verify the BC `main.cpp` uses the configured baud as
/// `kDefaultBaud` and in the Usage strings when the UART's baud rate is
/// known, and does NOT include the unknown-baud comment block.
#[test]
fn t25() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    let tx = Gpio::new("TX".to_string(),  "PA9".to_string(),  true);
    let rx = Gpio::new("RX".to_string(),  "PA10".to_string(), false);
    devices.uart_buses.push(BusUart::new(
        Some(tx), Some(rx),
        "USART1".to_string(), "huart1".to_string(),
        Some(38400),
    ));
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: Some(38400),
    };
    let tmp = unique_tmp_dir("exhibition_engine_t25");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    crate::engine::generate_bc(&art, &transport, &tmp).unwrap();

    let ex = std::fs::read_to_string(
        tmp.join(crate::engine::bc_codegen::BC_OUTPUT_DIR_NAME)
            .join("main.cpp")
    ).unwrap();
    assert!(ex.contains("kDefaultBaud = 38400u"),
        "kDefaultBaud must use the real baud; got:\n{}", ex);
    assert!(ex.contains("/dev/ttyUSB0 38400"),
        "Usage string must show real baud; got:\n{}", ex);
    assert!(!ex.contains("did not specify a baud rate"),
        "no fallback NOTE when baud is known; got:\n{}", ex);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t26
///
/// Verify the BC `main.cpp` falls back to 115200 AND emits
/// the "did not specify a baud rate" NOTE comment block when a UART
/// transport has no baud configured.
#[test]
fn t26() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t26");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    crate::engine::generate_bc(&art, &transport, &tmp).unwrap();

    let ex = std::fs::read_to_string(
        tmp.join(crate::engine::bc_codegen::BC_OUTPUT_DIR_NAME)
            .join("main.cpp")
    ).unwrap();
    assert!(ex.contains("kDefaultBaud = 115200u"),
        "kDefaultBaud must fall back to 115200 when unknown; got:\n{}", ex);
    assert!(ex.contains("did not specify a baud rate"),
        "NOTE block must be present when baud unknown; got:\n{}", ex);
    assert!(ex.contains("baud above is a fallback"),
        "Usage string must include fallback caveat; got:\n{}", ex);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t27
///
/// Verify that a USB CDC transport does NOT emit the unknown-baud NOTE
/// in `main.cpp` even though `transport.baud_rate() == None`,
/// since USB CDC ignores baud anyway. The INTEGRATION_GUIDE already
/// explains this in its USB-specific verify section.
#[test]
fn t27() {
    let devices = fixture_set();
    let transport = TransportChoice::UsbCdc {
        bus_name: "USB_OTG_HS".to_string(),
        speed: UsbSpeed::Hs,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t27");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    crate::engine::generate_bc(&art, &transport, &tmp).unwrap();

    let ex = std::fs::read_to_string(
        tmp.join(crate::engine::bc_codegen::BC_OUTPUT_DIR_NAME)
            .join("main.cpp")
    ).unwrap();
    assert!(ex.contains("kDefaultBaud = 115200u"),
        "USB CDC must use the 115200 default; got:\n{}", ex);
    assert!(!ex.contains("did not specify a baud rate"),
        "USB CDC must not emit the unknown-baud NOTE block; got:\n{}", ex);

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t28
///
/// Verify that `device_registry.cpp` `#include`s the CubeMX peripheral
/// headers that declare the HAL handles it instantiates (issue 1). A set
/// using I2C, SPI, ADC and a UART *device* — driven over a UART transport —
/// must pull in `i2c.h`, `spi.h`, `adc.h`, and `usart.h`, but not headers for
/// kinds it does not use (`dac.h`, `tim.h`, `can.h`) nor `gpio.h` (GPIO
/// symbols arrive via `main.h`).
#[test]
fn t28() {
    let mut devices = fixture_set();   // gpio out/in, adc, spi, USART1
    let sda = Gpio::new("SDA".to_string(), "PB7".to_string(), true);
    let scl = Gpio::new("SCL".to_string(), "PB6".to_string(), true);
    devices.i2c_buses.push(BusI2c::new(sda, scl, "I2C1".to_string(), "hi2c1".to_string()));
    // Second UART becomes a peripheral device (USART1 is the transport).
    let tx2 = Gpio::new("TX2".to_string(), "PC10".to_string(), true);
    let rx2 = Gpio::new("RX2".to_string(), "PC11".to_string(), false);
    devices.uart_buses.push(BusUart::new(
        Some(tx2), Some(rx2), "USART2".to_string(), "huart2".to_string(), None));

    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t28");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();
    let registry = std::fs::read_to_string(
        tmp.join(SRC_DIR_REL).join("device_registry.cpp")).unwrap();

    for needed in ["#include \"i2c.h\"", "#include \"spi.h\"",
                   "#include \"adc.h\"", "#include \"usart.h\""] {
        assert!(registry.contains(needed),
            "registry must contain {}; got:\n{}", needed, registry);
    }
    for absent in ["#include \"gpio.h\"", "#include \"dac.h\"",
                   "#include \"tim.h\"", "#include \"can.h\""] {
        assert!(!registry.contains(absent),
            "registry must not contain {}; got:\n{}", absent, registry);
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t29
///
/// Verify that only the handler source files for kinds actually used are
/// emitted (issue 2). A GPIO + I2C set over a UART transport must write the
/// GPIO and I2C handlers but NOT the adc/dac/pwm/spi/uart/can handlers, whose
/// `.cpp`s would otherwise reference HAL symbols the `.ioc` never enabled and
/// break the link.
#[test]
fn t29() {
    let mut devices = fixture_set();
    devices.adcs.clear();
    devices.spi_buses.clear();          // leaves: gpio out/in + USART1 (transport)
    let sda = Gpio::new("SDA".to_string(), "PB7".to_string(), true);
    let scl = Gpio::new("SCL".to_string(), "PB6".to_string(), true);
    devices.i2c_buses.push(BusI2c::new(sda, scl, "I2C1".to_string(), "hi2c1".to_string()));

    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t29");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let inc_root = tmp.join(INC_DIR_REL);
    let src_root = tmp.join(SRC_DIR_REL);

    // Used kinds: their handlers must be present.
    for present in ["devices/gpio_output_handler.cpp",
                    "devices/gpio_input_handler.cpp",
                    "devices/i2c_handler.cpp"] {
        assert!(src_root.join(present).exists(),
            "expected used handler '{}' to be written", present);
    }
    // Unused kinds: neither the .cpp nor the .h should appear.
    for kind in ["adc", "dac", "pwm", "spi", "uart", "can"] {
        let cpp = src_root.join(format!("devices/{}_handler.cpp", kind));
        let hdr = inc_root.join(format!("devices/{}_handler.h", kind));
        assert!(!cpp.exists(), "unused handler '{}' must not be written", cpp.display());
        assert!(!hdr.exists(), "unused header '{}' must not be written", hdr.display());
    }

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t30
///
/// Verify that a host-mode USB block is excluded from the transport picker
/// (issue 3). With a UART present alongside a host-mode USB, only the UART is
/// offered and selecting it returns the UART. With *only* a host-mode USB, the
/// picker reports no eligible transport rather than offering the unusable USB.
#[test]
fn t30() {
    // UART + host-mode USB: USB must not appear in the menu.
    let mut devices = fixture_set();   // has USART1
    devices.usb_buses.push(make_usb("USB_OTG_FS", "husb_otg_fs").with_cdc_device(false));

    let mut out = Vec::new();
    let choice = select_transport_with_io(&devices, b"1\n".as_ref(), &mut out)
        .expect("the UART should be eligible");
    assert!(matches!(choice, TransportChoice::Uart { .. }));
    assert_eq!(choice.bus_name(), "USART1");
    let menu = String::from_utf8(out).unwrap();
    assert!(!menu.contains("USB_OTG_FS"),
        "host-mode USB must not be listed; menu was:\n{}", menu);

    // Only a host-mode USB: no eligible transport at all.
    let mut only_host = fixture_set();
    only_host.uart_buses.clear();
    only_host.usb_buses.push(make_usb("USB_OTG_FS", "husb_otg_fs").with_cdc_device(false));
    let err = select_transport_with_io(&only_host, b"1\n".as_ref(), Vec::new())
        .expect_err("host-only USB must leave no eligible transport");
    assert!(err.contains("No eligible transport"), "got: {}", err);
}


/// t31
///
/// Verify that an `UnderRoot=false` STM32CubeIDE project routes generated
/// sources into the toolchain's build folder
/// (`STM32CubeIDE/Application/User/Core/rt`) and *not* into the root
/// `Core/Src/rt` (issue 4), while headers still land under `Core/Inc/rt`.
/// This is what prevents the two-locations duplication.
#[test]
fn t31() {
    let mut devices = fixture_set();
    devices.layout = ProjectLayout {
        under_root: false,
        toolchain: "STM32CubeIDE".to_string(),
    };
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t31");
    std::fs::create_dir_all(&tmp).expect("mkdir");

    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    assert!(tmp.join("STM32CubeIDE/Application/User/Core/rt/device_registry.cpp").exists(),
        "sources must land in the toolchain build folder for UnderRoot=false");
    assert!(!tmp.join("Core/Src/rt/device_registry.cpp").exists(),
        "sources must NOT also be written to the root Core/Src/rt");
    assert!(tmp.join(INC_DIR_REL).join("device_registry.h").exists(),
        "headers always land under Core/Inc/rt");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t32
///
/// Unit-level checks on the layout resolver (issue 4): the root layout maps
/// to `Core/Src/rt`; an `UnderRoot=false` STM32CubeIDE project maps to the
/// toolchain folder; an unknown toolchain falls back to `Core/Src/rt` and is
/// reported as not modeled so the CLI can warn.
#[test]
fn t32() {
    let root = ProjectLayout::default();
    assert_eq!(source_dir_rel(&root), "Core/Src/rt");
    assert!(layout_is_modeled(&root));

    let cube = ProjectLayout { under_root: false, toolchain: "STM32CubeIDE".to_string() };
    assert_eq!(source_dir_rel(&cube), "STM32CubeIDE/Application/User/Core/rt");
    assert!(layout_is_modeled(&cube));

    let unknown = ProjectLayout { under_root: false, toolchain: "Makefile".to_string() };
    assert_eq!(source_dir_rel(&unknown), "Core/Src/rt");
    assert!(!layout_is_modeled(&unknown));
}


/// t33
///
/// Pin the FNV-1a-32 constants (offset basis `0x811c_9dc5`, prime
/// `0x0100_0193`) against the canonical published test vectors by hashing raw
/// strings through `fingerprint_of_canonical` (which hashes the bytes
/// directly). A transposed seed/prime that still hashed *deterministically*
/// would satisfy t2/t3 but is caught here.
#[test]
fn t33() {
    assert_eq!(fingerprint_of_canonical(""), 0x811C_9DC5,
        "empty input must hash to the FNV-1a offset basis");
    assert_eq!(fingerprint_of_canonical("a"), 0xE40C_292C);
    assert_eq!(fingerprint_of_canonical("foobar"), 0xBF9C_F968);
}


/// t34
///
/// Verify `canonicalize` drops comment lines, blank/whitespace-only lines, any
/// `fingerprint:` line, and every `description:` line (device- and
/// command-level, even when indented), trims surrounding whitespace from the
/// lines it keeps, and preserves their relative order. The result is the stable
/// input the fingerprint is computed over — so editing a description must not
/// perturb it.
#[test]
fn t34() {
    let yaml = "\
# a leading comment
version: 1

  fingerprint: 0xDEADBEEF
description: System handler label
sub_address: 1
  name: system
    description: TODO
";
    let canon = canonicalize(yaml);
    assert_eq!(canon, "version: 1\nsub_address: 1\nname: system\n");
    assert!(!canon.contains("fingerprint"), "fingerprint line must be stripped");
    assert!(!canon.contains("description"), "description lines must be stripped");
    assert!(!canon.contains('#'), "comment line must be stripped");
}


/// t35
///
/// Verify `stamp_fingerprint_in_set` substitutes the placeholder only in the
/// file that carries it (the system file), writes the value as upper-case
/// zero-padded `0x%08X`, and leaves every other file byte-identical.
#[test]
fn t35() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let before = build_manifest_files(&slots);
    let mut after = build_manifest_files(&slots);
    stamp_fingerprint_in_set(&mut after, 0x0A0B_0C0D);

    for (b, a) in before.iter().zip(after.iter()) {
        if a.file_name == "01_system.yaml" {
            assert!(a.content.contains("fingerprint: 0x0A0B0C0D"),
                "system file must carry the stamped, zero-padded value");
            assert!(!a.content.contains("0x00000000"),
                "placeholder must be gone from the system file");
        } else {
            assert_eq!(b.content, a.content,
                "non-system file '{}' must be untouched by stamping", a.file_name);
        }
    }
}


/// t36
///
/// Verify `canonical_set` folds each file's *name* into the canonical text, so
/// a sub-address reassignment (which renames files) perturbs the fingerprint
/// even when every file's content is unchanged.
#[test]
fn t36() {
    let a = vec![ManifestFile {
        file_name: "02_foo.yaml".to_string(),
        content: "name: foo\n".to_string(),
    }];
    let b = vec![ManifestFile {
        file_name: "03_foo.yaml".to_string(),
        content: "name: foo\n".to_string(),
    }];
    let fp_a = fingerprint_of_canonical(&canonical_set(&a));
    let fp_b = fingerprint_of_canonical(&canonical_set(&b));
    assert_ne!(fp_a, fp_b, "file name must contribute to the fingerprint");
}


/// t37
///
/// Verify the RT include-rewriter (`rewrite_includes_for_rt`, exercised here
/// through `generate_rt`) leaves no pre-rewrite `#include` form in any emitted
/// file and produces the `rt/<area>/<file>.h` forms instead. Catches a missed
/// rewrite rule that would leave the generated project with unresolvable
/// includes.
#[test]
fn t37() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t37");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    // Concatenate every emitted .h/.cpp under the generated tree.
    let mut all = String::new();
    let mut stack = vec![tmp.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("h") | Some("cpp")
            ) {
                all.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
    }

    // A representative spread of pre-rewrite forms (bare, one-up, two-up) must
    // not survive anywhere in the tree.
    for stale in [
        "#include \"message_header.h\"",
        "#include \"i_sub_address_handler.h\"",
        "#include \"crc.h\"",
        "#include \"../common/crc.h\"",
        "#include \"../api_fingerprint.h\"",
        "#include \"../../communication/i_sub_address_handler.h\"",
    ] {
        assert!(!all.contains(stale),
            "stale include '{}' should have been rewritten to an rt/ path", stale);
    }
    // Generic guard: no relative-traversal include may survive at all. Sources
    // live under Core/Src/rt/ while headers live under Core/Inc/rt/, so a
    // `../` include that resolves in the template tree breaks the RT build.
    for line in all.lines().filter(|l| l.trim_start().starts_with("#include \"../")) {
        panic!("relative include '{}' should have been rewritten to an rt/ path", line.trim());
    }
    // And the rewritten forms are present.
    assert!(all.contains("#include \"rt/communication/message_header.h\""),
        "rewritten message_header include missing");
    assert!(all.contains("#include \"rt/common/crc.h\""),
        "rewritten crc include missing");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t38
///
/// Verify `render_fingerprint_header` (via `generate_rt`) emits
/// `api_fingerprint.h` with the include guard and a `constexpr uint32_t`
/// whose literal is the artifact's fingerprint, always formatted as exactly
/// eight upper-case hex digits with the `u` suffix (zero-padding invariant).
#[test]
fn t38() {
    let devices = fixture_set();
    let transport = TransportChoice::Uart {
        bus_name: "USART1".to_string(),
        hal_handle: "huart1".to_string(),
        baud_rate: None,
    };
    let tmp = unique_tmp_dir("exhibition_engine_t38");
    std::fs::create_dir_all(&tmp).expect("mkdir");
    let art = build_artifacts(&devices, &transport).unwrap();
    generate_rt(&art, &transport, &tmp).unwrap();

    let header = std::fs::read_to_string(
        tmp.join(INC_DIR_REL).join("api_fingerprint.h")
    ).unwrap();

    assert!(header.contains("#ifndef API_FINGERPRINT_H"), "include guard missing");
    let expected = format!("constexpr uint32_t kApiFingerprint = 0x{:08X}u;", art.fingerprint);
    assert!(header.contains(&expected),
        "header literal must match the artifact fingerprint; expected '{}' in:\n{}",
        expected, header);

    // The literal is always 8 hex digits + 'u' regardless of leading zeros.
    let literal = format!("0x{:08X}u", art.fingerprint);
    assert_eq!(literal.len(), 11, "0x + 8 hex digits + u");

    let _ = std::fs::remove_dir_all(&tmp);
}


/// t39
///
/// Verify that malformed or out-of-range GPIO pins are silently dropped by
/// `slots_for_gpio_banks` (via `assign_sub_addresses`): a pin whose identifier
/// has no extractable bank produces no slot at all, and a pin whose index is
/// above 15 is excluded from its bank's members — while a well-formed pin on
/// the same bank still yields its slot and member.
#[test]
fn t39() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.spi_buses.clear();
    devices.adcs.clear();
    devices.gpios.clear();
    // Well-formed output on bank A.
    devices.gpios.push(Gpio::new("LED".to_string(), "PA5".to_string(), true));
    // Same bank A, but pin index 99 > 15 -> dropped from members.
    devices.gpios.push(Gpio::new("BAD_HIGH".to_string(), "PA99".to_string(), true));
    // No extractable bank ('X' is not 'P...') -> never grouped, no slot.
    devices.gpios.push(Gpio::new("BAD_ID".to_string(), "XYZ".to_string(), true));

    let slots = assign_sub_addresses(&devices, None).unwrap();

    assert_eq!(slots.len(), 1, "only the valid bank-A output slot should exist");
    let slot = &slots[0];
    assert_eq!(slot.kind, DeviceKind::GpioOutput);
    assert_eq!(slot.instance_name, "gpio_out_bank_A");
    assert_eq!(slot.members.len(), 1, "the >15 pin must not become a member");
    assert_eq!(slot.members[0].pin_index, Some(5));
    let mask = slot.details.iter()
        .find(|(k, _)| k == "pin_mask").map(|(_, v)| v.as_str()).unwrap();
    assert_eq!(mask, "0x0020", "mask reflects only PA5 (bit 5)");
}


/// t40
///
/// Verify the exact 30/31-slot boundary of `assign_sub_addresses`: exactly 30
/// peripheral slots succeed and occupy sub-addresses
/// `FIRST_DEVICE_SUB_ADDRESS..=SUB_ADDRESS_LIMIT-1`, while a 31st slot
/// overflows the usable range and returns an error. (t8 covers only the
/// failing side.)
#[test]
fn t40() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.gpios.clear();
    devices.adcs.clear();
    devices.spi_buses.clear();
    devices.pwms.clear();
    for i in 0..30 {
        let pin = Gpio::new(format!("PWM_PIN_{}", i), "PA0".to_string(), true);
        devices.pwms.push(Pwm::new(pin, 1, format!("PWM_{}", i), format!("htim{}", i)));
    }

    let slots = assign_sub_addresses(&devices, None)
        .expect("exactly 30 peripherals must fit");
    assert_eq!(slots.len(), 30);
    assert_eq!(slots[0].sub_address, FIRST_DEVICE_SUB_ADDRESS);
    assert_eq!(slots[29].sub_address, FIRST_DEVICE_SUB_ADDRESS + 29,
        "the 30th slot lands on the last usable sub-address");

    // One more peripheral tips it over the edge.
    let pin = Gpio::new("PWM_PIN_30".to_string(), "PA0".to_string(), true);
    devices.pwms.push(Pwm::new(pin, 1, "PWM_30".to_string(), "htim30".to_string()));
    let err = assign_sub_addresses(&devices, None)
        .expect_err("31 peripherals must overflow the 30-slot range");
    assert!(err.contains("Too many"), "got: {}", err);
}


/// t41
///
/// A QUADSPI peripheral is assigned a sub-address ordered lexicographically
/// between PWM (`pwm`) and RTC (`rtc`), and its slot carries the expected
/// kind, sort key, instance name, and HAL handle detail.
#[test]
fn t41() {
    let mut devices = fixture_set();
    devices.uart_buses.clear();
    devices.gpios.clear();
    devices.adcs.clear();
    devices.spi_buses.clear();

    let pwm_pin = Gpio::new("PWM_PIN".to_string(), "PA0".to_string(), true);
    devices.pwms = vec![Pwm::new(pwm_pin, 1, "PWM".to_string(), "htim2".to_string())];
    devices.qspis = vec![Qspi::new("hqspi".to_string())];
    devices.rtcs = vec![Rtc::new("hrtc".to_string())];

    let slots = assign_sub_addresses(&devices, None)
        .expect("assignment should succeed");

    assert_eq!(slots.len(), 3);
    // Lexicographic order of sort keys: pwm < qspi < rtc.
    assert_eq!(slots[0].kind, DeviceKind::Pwm);
    assert_eq!(slots[1].kind, DeviceKind::Qspi);
    assert_eq!(slots[2].kind, DeviceKind::Rtc);

    let qspi = &slots[1];
    assert_eq!(qspi.sub_address, FIRST_DEVICE_SUB_ADDRESS + 1);
    assert_eq!(qspi.sort_key, "qspi::hqspi");
    assert_eq!(qspi.instance_name, "qspi");
    assert_eq!(qspi.label, "QSPI");
    assert_eq!(qspi.members.len(), 1);
    assert!(qspi.details.iter().any(|(k, v)| k == "hal_handle" && v == "&hqspi"));
}


/// t42
///
/// The system manifest renders each command in the structured format — its
/// build-time catalog description plus nested `cmd_payload`/`resp_payload`
/// mappings with a `bytes` size and an ordered `fields` list — and the whole
/// file is valid YAML.
#[test]
fn t42() {
    // No peripheral slots are needed: the system file is rendered unconditionally.
    let files = build_manifest_files(&[]);
    let sys = &files[0];
    assert_eq!(sys.file_name, "01_system.yaml");

    let expected = "\
  - opcode: 0x05
    name: GET_CPU_ID
    description: Get the unique CPU ID
    cmd_payload:
      bytes: 0
    resp_payload:
      bytes: 12
      fields:
        - uid0: uint32_t
        - uid1: uint32_t
        - uid2: uint32_t
";
    assert!(
        sys.content.contains(expected),
        "GET_CPU_ID should render in the structured format; got:\n{}",
        sys.content
    );

    // A zero-field, zero-byte payload still emits a `bytes:` line and no fields.
    assert!(sys.content.contains("    cmd_payload:\n      bytes: 0\n    resp_payload:"));

    // The generated manifest must be valid YAML (it is opened/edited by users).
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&sys.content).expect("generated system manifest must be valid YAML");
    assert!(parsed.get("commands").is_some(), "parsed manifest has a commands list");
}


/// t43
///
/// Every generated per-sub-address manifest is valid YAML, including the
/// peripheral files whose payloads use the trickier scalar forms — array types
/// (`data: uint8_t[]`), open-ended sizes (`bytes: 3+`, `bytes: N`) — and GPIO
/// files that also carry a `pins:` list.
#[test]
fn t43() {
    let devices = fixture_set();
    let slots = assign_sub_addresses(&devices, Some("USART1")).unwrap();
    let files = build_manifest_files(&slots);

    for f in &files {
        serde_yaml::from_str::<serde_yaml::Value>(&f.content)
            .unwrap_or_else(|e| panic!("manifest '{}' is not valid YAML: {e}\n{}", f.file_name, f.content));
    }

    // Spot-check that the SPI file exercised the array-type / open-ended forms.
    let spi = files
        .iter()
        .find(|f| f.content.contains("type: spi"))
        .expect("fixture has an SPI bus");
    assert!(spi.content.contains("- data: uint8_t[]"), "array field type rendered");
    assert!(spi.content.contains("bytes: 3+"), "open-ended size rendered");
}


/// Build a fresh temporary directory under `/media/ramdisk` (per the
/// user's CLAUDE.md preference for ramdisk-backed temporary files), or
/// fall back to `std::env::temp_dir` if the ramdisk is unavailable.
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
