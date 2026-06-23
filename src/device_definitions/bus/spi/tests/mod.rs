// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`BusSpi`](super::BusSpi) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::BusSpi;
use super::Gpio;

/// Helper to build a valid output Gpio for use as a chip-select, clock, or MOSI pin.
fn output_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// Helper to build a valid input Gpio (for MISO).
fn input_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), false)
}

/// t1
///
/// Verify that the simple accessors `cs()`, `clk()`, `name()`, and `id()`
/// return exactly the values supplied to `BusSpi::new`.
#[test]
fn t1() {
    let cs_list = vec![output_pin("CS0", "PA4"), output_pin("CS1", "PA5")];
    let clk = output_pin("SCK", "PB3");
    let miso = input_pin("MISO", "PB4");
    let mosi = output_pin("MOSI", "PB5");

    let bus = BusSpi::new(
        cs_list.clone(),
        Some(miso.clone()),
        Some(mosi.clone()),
        clk.clone(),
        "THERMAL_CYCLER_BUS".to_string(),
        "hspi1".to_string(),
    );

    assert_eq!(bus.cs().len(), 2);
    assert!(bus.cs()[0] == cs_list[0]);
    assert!(bus.cs()[1] == cs_list[1]);
    assert!(bus.clk() == &clk);
    assert_eq!(bus.name(), "THERMAL_CYCLER_BUS");
    assert_eq!(bus.id(), "hspi1");
}

/// t2
///
/// Verify that `miso()` and `mosi()` return clones of the supplied pins when
/// present, and `None` when not supplied.
#[test]
fn t2() {
    let clk = output_pin("SCK", "PB3");
    let miso = input_pin("MISO", "PB4");
    let mosi = output_pin("MOSI", "PB5");

    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(miso.clone()),
        Some(mosi.clone()),
        clk.clone(),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus.miso() == Some(miso));
    assert!(bus.mosi() == Some(mosi));

    let bus_no_miso = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        None,
        Some(output_pin("MOSI", "PB5")),
        clk.clone(),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus_no_miso.miso().is_none());
    assert!(bus_no_miso.mosi().is_some());

    let bus_no_mosi = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        None,
        clk,
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus_no_mosi.miso().is_some());
    assert!(bus_no_mosi.mosi().is_none());
}

/// t3
///
/// Verify that `is_valid()` accepts a fully populated bus definition with at
/// least one of MISO or MOSI present, a valid clock pin, and non-empty
/// `name` and `id` fields.
#[test]
fn t3() {
    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        Some(output_pin("MOSI", "PB5")),
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus.is_valid().is_ok());

    // Receive-only bus (MISO only) is acceptable.
    let rx_only = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        None,
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(rx_only.is_valid().is_ok());

    // Transmit-only bus (MOSI only) is acceptable.
    let tx_only = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        None,
        Some(output_pin("MOSI", "PB5")),
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(tx_only.is_valid().is_ok());
}

/// t4
///
/// Verify that `is_valid()` rejects a bus definition that has neither MISO
/// nor MOSI defined, since such a bus could neither transmit nor receive.
#[test]
fn t4() {
    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        None,
        None,
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a bus definition whose clock pin is not
/// itself a valid Gpio definition.
#[test]
fn t5() {
    let bad_clk = output_pin("SCK", "QQ99");
    assert!(bad_clk.is_valid().is_err(), "test precondition: clock pin must be invalid");

    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        Some(output_pin("MOSI", "PB5")),
        bad_clk,
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a bus definition whose `name` field is
/// the empty string.
#[test]
fn t6() {
    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        Some(output_pin("MOSI", "PB5")),
        output_pin("SCK", "PB3"),
        String::new(),
        "hspi1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t7
///
/// Verify that `is_valid()` rejects a bus definition whose `id` field is
/// the empty string.
#[test]
fn t7() {
    let bus = BusSpi::new(
        vec![output_pin("CS0", "PA4")],
        Some(input_pin("MISO", "PB4")),
        Some(output_pin("MOSI", "PB5")),
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        String::new(),
    );
    assert!(bus.is_valid().is_err());
}

/// t8
///
/// Verify that `cs()` returns an empty slice when the bus is constructed with
/// no chip-select pins, and the accessor still behaves correctly.
#[test]
fn t8() {
    let bus = BusSpi::new(
        Vec::new(),
        Some(input_pin("MISO", "PB4")),
        Some(output_pin("MOSI", "PB5")),
        output_pin("SCK", "PB3"),
        "BUS".to_string(),
        "hspi1".to_string(),
    );
    assert!(bus.cs().is_empty());
}
