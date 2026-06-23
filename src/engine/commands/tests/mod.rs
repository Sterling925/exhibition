// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the embedded command catalog and identifier conversion.

use crate::engine::addresses::DeviceKind;
use crate::engine::commands::{
    self, validate_description, to_pascal_case, Catalog, DeviceCommandSet,
};


/// The embedded catalog parses and passes every validation rule. Forces the
/// parse eagerly (instead of waiting for the `LazyLock`) so a malformed or
/// inconsistent catalog fails the build here with a readable message.
#[test]
fn t1() {
    let catalog = Catalog::parse_embedded().expect("embedded catalog must parse and validate");
    drop(catalog);
}


/// `to_pascal_case` capitalizes each underscore segment, preserves digit runs,
/// and folds already-mixed tokens to first-upper-rest-lower — always yielding a
/// valid C++ identifier.
#[test]
fn t2() {
    assert_eq!(to_pascal_case("GET_FINGERPRINT"), "GetFingerprint");
    assert_eq!(to_pascal_case("GET_DEVICE_COUNT"), "GetDeviceCount");
    assert_eq!(to_pascal_case("SET"), "Set");
    assert_eq!(to_pascal_case("CLEAR"), "Clear");
    assert_eq!(to_pascal_case("SET_COUNTS"), "SetCounts");
    assert_eq!(to_pascal_case("TX"), "Tx");
    assert_eq!(to_pascal_case("RX_FRAME"), "RxFrame");
    assert_eq!(to_pascal_case("WRITE_REG"), "WriteReg");
    assert_eq!(to_pascal_case("adc_hadc1_ch0"), "AdcHadc1Ch0");
    assert_eq!(to_pascal_case("gpio_out_bank_A"), "GpioOutBankA");
    assert_eq!(to_pascal_case("uart_huart2"), "UartHuart2");
    assert_eq!(to_pascal_case("gpio_PA5"), "GpioPa5");
}


/// Every [`DeviceKind`] resolves to a command set, and `system()` resolves to
/// the system set. `all()` yields exactly the embedded sets with `system`
/// first.
#[test]
fn t3() {
    let kinds = [
        DeviceKind::GpioOutput,
        DeviceKind::GpioInput,
        DeviceKind::Adc,
        DeviceKind::Dac,
        DeviceKind::Pwm,
        DeviceKind::Qspi,
        DeviceKind::Rtc,
        DeviceKind::Spi,
        DeviceKind::Uart,
        DeviceKind::I2c,
        DeviceKind::Can,
    ];
    for k in kinds {
        let set = commands::for_kind(k);
        assert_eq!(set.kind, k.sort_key());
        assert!(!set.commands.is_empty());
    }

    assert_eq!(commands::system().cpp_enum, "SystemCmd");

    let collected: Vec<&str> = commands::all().map(|s| s.kind.as_str()).collect();
    assert_eq!(collected.len(), kinds.len() + 1);
    assert_eq!(collected[0], "system");
}


/// The catalog reproduces the legacy hardcoded opcode/name vocabulary, guarding
/// against accidental drift when the data moved from Rust into YAML.
#[test]
fn t4() {
    // The universal GET_SERVICE_HASH (opcode 0x00) is the first command of
    // every set, so the kind-specific vocabulary starts at index 1.
    let gpio_out = commands::for_kind(DeviceKind::GpioOutput);
    assert_eq!(gpio_out.cpp_enum, "GpioOutputCmd");
    assert_eq!(gpio_out.commands[0].name, "GET_SERVICE_HASH");
    assert_eq!(gpio_out.commands[0].opcode, 0x00);
    assert_eq!(gpio_out.commands[1].name, "SET");
    assert_eq!(gpio_out.commands[1].opcode, 0x01);
    assert_eq!(gpio_out.commands[2].name, "CLEAR");
    assert_eq!(gpio_out.commands[2].opcode, 0x02);

    let sys = commands::system();
    assert_eq!(sys.commands[0].name, "GET_SERVICE_HASH");
    assert_eq!(sys.commands[0].opcode, 0x00);
    assert_eq!(sys.commands[1].name, "GET_FINGERPRINT");
    assert_eq!(sys.commands[1].opcode, 0x01);
    assert_eq!(sys.commands[2].name, "GET_DEVICE_COUNT");
    assert_eq!(sys.commands[2].opcode, 0x02);

    let can = commands::for_kind(DeviceKind::Can);
    assert_eq!(can.commands[2].name, "RX_FRAME");
    assert_eq!(can.commands[2].resp_payload.bytes, "13 (padded to 14)");
}


/// Structured payloads parse from the embedded catalog: a multi-field response
/// keeps its fields in order with their types, and a zero-byte payload has a
/// `bytes` size with no fields.
#[test]
fn t6() {
    let sys = commands::system();
    let get_cpu = sys
        .commands
        .iter()
        .find(|c| c.name == "GET_CPU_ID")
        .expect("GET_CPU_ID present");

    assert_eq!(get_cpu.resp_payload.bytes, "12");
    let fields: Vec<(&str, &str)> = get_cpu
        .resp_payload
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.ty.as_str()))
        .collect();
    assert_eq!(
        fields,
        [("uid0", "uint32_t"), ("uid1", "uint32_t"), ("uid2", "uint32_t")]
    );

    // A zero-byte command payload carries a size but no field breakdown.
    assert_eq!(get_cpu.cmd_payload.bytes, "0");
    assert!(get_cpu.cmd_payload.fields.is_empty());
}


/// A payload field is a single-key `name: type` mapping. The custom
/// deserializer accepts exactly one key and rejects an entry with two keys.
#[test]
fn t7() {
    let ok = "\
kind: t
cpp_enum: TCmd
commands:
  - name: FOO
    opcode: \"0x01\"
    cmd_payload:
      bytes: \"1\"
      fields:
        - flag: uint8_t
    resp_payload:
      bytes: \"0\"
";
    let set: DeviceCommandSet = serde_yaml::from_str(ok).expect("single-key field parses");
    assert_eq!(set.commands[0].cmd_payload.fields[0].name, "flag");
    assert_eq!(set.commands[0].cmd_payload.fields[0].ty, "uint8_t");

    let bad = "\
kind: t
cpp_enum: TCmd
commands:
  - name: FOO
    opcode: \"0x01\"
    cmd_payload:
      bytes: \"2\"
      fields:
        - a: uint8_t
          b: uint8_t
    resp_payload:
      bytes: \"0\"
";
    assert!(
        serde_yaml::from_str::<DeviceCommandSet>(bad).is_err(),
        "a two-key field entry must be rejected"
    );
}


/// `validate_description` accepts the plain single-line text the catalog uses
/// (including parentheses and slashes, which are inert inline in YAML) and
/// rejects anything that would break the unquoted, single-line invariant the
/// manifest emitter and `canonicalize` depend on: a colon-space, an inline
/// comment marker, embedded newlines, surrounding whitespace, or emptiness.
#[test]
fn t8() {
    assert!(validate_description("Set GPIO pin (high)").is_ok());
    assert!(validate_description("Read/clear the flag").is_ok());

    assert!(validate_description("").is_err(), "empty rejected");
    assert!(validate_description(" leading").is_err(), "leading space rejected");
    assert!(validate_description("trailing ").is_err(), "trailing space rejected");
    assert!(validate_description("two\nlines").is_err(), "newline rejected");
    assert!(
        validate_description("key: value").is_err(),
        "colon-space rejected (would start a nested mapping)"
    );
    assert!(
        validate_description("text #comment").is_err(),
        "space-hash rejected (parsed as a YAML comment)"
    );
}


/// The checked-in golden `command_ids.h` matches the generator output byte for
/// byte. The golden is what the in-tree handler templates `#include`, so this
/// keeps it in sync; if it fails, copy the generator output into the golden.
#[test]
fn t5() {
    let generated = commands::render_command_ids_header();
    let golden = include_str!("../../autogen_content/cpp/generated/command_ids.h");
    assert_eq!(generated, golden, "golden command_ids.h is stale");
}


/// The checked-in golden `service_hashes.h` matches the generator output byte
/// for byte. If it fails, copy the generator output into the golden.
#[test]
fn t9() {
    let generated = commands::render_service_hashes_header();
    let golden = include_str!("../../autogen_content/cpp/generated/service_hashes.h");
    assert_eq!(generated, golden, "golden service_hashes.h is stale");
}


/// The per-service hash depends only on the command vocabulary (opcode + name +
/// payload shapes): editing descriptions leaves it unchanged, while editing an
/// opcode or a payload changes it. This is the same stability the global
/// fingerprint relies on, applied to one command set.
#[test]
fn t10() {
    use crate::engine::commands::{service_hash_of_set, CommandDef, PayloadDef, PayloadField};

    let base = DeviceCommandSet {
        kind: "t".to_string(),
        cpp_enum: "TCmd".to_string(),
        commands: vec![CommandDef {
            name: "GET_SERVICE_HASH".to_string(),
            opcode: 0x00,
            description: Some("original".to_string()),
            cmd_payload: PayloadDef::default(),
            resp_payload: PayloadDef {
                bytes: "4".to_string(),
                fields: vec![PayloadField { name: "service_hash".to_string(), ty: "uint32_t".to_string() }],
            },
        }],
    };
    let h0 = service_hash_of_set(&base);

    // Description change (and a None) must NOT change the hash.
    let mut desc_changed = base.clone();
    desc_changed.commands[0].description = None;
    assert_eq!(service_hash_of_set(&desc_changed), h0, "descriptions must not affect the hash");

    // kind / cpp_enum are not part of the vocabulary, so they must not matter.
    let mut renamed = base.clone();
    renamed.kind = "other".to_string();
    renamed.cpp_enum = "OtherCmd".to_string();
    assert_eq!(service_hash_of_set(&renamed), h0, "kind/cpp_enum must not affect the hash");

    // An opcode change MUST change the hash.
    let mut op_changed = base.clone();
    op_changed.commands[0].opcode = 0x01;
    assert_ne!(service_hash_of_set(&op_changed), h0, "an opcode change must change the hash");

    // A payload-shape change MUST change the hash.
    let mut payload_changed = base.clone();
    payload_changed.commands[0].resp_payload.bytes = "8".to_string();
    assert_ne!(service_hash_of_set(&payload_changed), h0, "a payload change must change the hash");
}


/// Catalog kinds with distinct command vocabularies hash differently, while two
/// kinds that expose a byte-identical vocabulary (SPI and UART) intentionally
/// share a hash — the per-service hash captures the command/response surface,
/// not the peripheral name. (The BC compares each sub-address against its own
/// kind's constant, so the shared value is harmless.)
#[test]
fn t11() {
    use crate::engine::commands::service_hash_of_set;

    // Distinct vocabularies → distinct hashes.
    let distinct = [
        DeviceKind::Adc,
        DeviceKind::Can,
        DeviceKind::Dac,
        DeviceKind::GpioInput,
        DeviceKind::GpioOutput,
        DeviceKind::I2c,
        DeviceKind::Pwm,
        DeviceKind::Qspi,
        DeviceKind::Rtc,
        DeviceKind::Spi, // stands in for the SPI/UART vocabulary
    ];
    let mut hashes: Vec<u32> = distinct.iter().map(|&k| service_hash_of_set(commands::for_kind(k))).collect();
    hashes.push(service_hash_of_set(commands::system()));
    let unique: std::collections::HashSet<u32> = hashes.iter().copied().collect();
    assert_eq!(unique.len(), hashes.len(), "distinct command vocabularies must hash distinctly");

    // Identical vocabularies (SPI and UART) → identical hash, by design.
    assert_eq!(
        service_hash_of_set(commands::for_kind(DeviceKind::Spi)),
        service_hash_of_set(commands::for_kind(DeviceKind::Uart)),
        "SPI and UART share a command vocabulary, so they must share a service hash"
    );
}


/// Every command set — the system handler and every device kind — must expose
/// the universal `GET_SERVICE_HASH` command at opcode `0x00`. The whole
/// per-service-hash feature assumes this (the RT answers `0x00` framework-side
/// for every sub-address, and `service_hash_of_set` folds the universal command
/// into each set's hash). Catalog validation only binds the *name* to opcode
/// `0x00` and back; it never checks *presence*, so a newly added catalog could
/// silently omit it. This guard locks the invariant.
#[test]
fn t12() {
    for set in commands::all() {
        assert!(
            set.commands
                .iter()
                .any(|c| c.name == "GET_SERVICE_HASH" && c.opcode == 0x00),
            "catalog '{}' is missing the universal GET_SERVICE_HASH@0x00 command",
            set.kind
        );
    }
}
