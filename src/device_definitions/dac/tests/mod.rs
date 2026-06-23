// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`Dac`](super::Dac) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::Dac;
use super::Gpio;

/// Helper to build a valid Gpio for use as a DAC output. The DAC drives the
/// pin from the analog peripheral; the `output` flag of the Gpio is not
/// meaningful here, so we use `true` to mirror the typical CubeMX export.
fn analog_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// t1
///
/// Verify that the accessors `pin()`, `name()`, and `id()` return exactly
/// the values supplied to `Dac::new`.
#[test]
fn t1() {
    let pin = analog_pin("VREF_OUT", "PA4");
    let dac = Dac::new(pin.clone(), 1, "REF_VOLTAGE_OUT".to_string(), "hdac1".to_string());

    assert!(dac.pin() == &pin);
    assert_eq!(dac.name(), "REF_VOLTAGE_OUT");
    assert_eq!(dac.id(), "hdac1");
}

/// t2
///
/// Verify that `is_valid()` accepts a fully populated definition with a
/// valid pin and non-empty `name` and `id`.
#[test]
fn t2() {
    let dac = Dac::new(
        analog_pin("VREF_OUT", "PA4"),
        1,
        "REF_VOLTAGE_OUT".to_string(),
        "hdac1".to_string(),
    );
    assert!(dac.is_valid().is_ok());
}

/// t3
///
/// Verify that `is_valid()` rejects a definition whose pin is not itself
/// a valid Gpio definition.
#[test]
fn t3() {
    let bad_pin = analog_pin("VREF_OUT", "QQ99");
    assert!(bad_pin.is_valid().is_err(), "test precondition: pin must be invalid");

    let dac = Dac::new(bad_pin, 1, "REF_VOLTAGE_OUT".to_string(), "hdac1".to_string());
    assert!(dac.is_valid().is_err());
}

/// t4
///
/// Verify that `is_valid()` rejects a definition whose `name` field is
/// the empty string.
#[test]
fn t4() {
    let dac = Dac::new(
        analog_pin("VREF_OUT", "PA4"),
        1,
        String::new(),
        "hdac1".to_string(),
    );
    assert!(dac.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a definition whose `id` field is
/// the empty string.
#[test]
fn t5() {
    let dac = Dac::new(
        analog_pin("VREF_OUT", "PA4"),
        1,
        "REF_VOLTAGE_OUT".to_string(),
        String::new(),
    );
    assert!(dac.is_valid().is_err());
}

/// t6
///
/// Verify that the `channel()` accessor returns exactly the value supplied
/// to `Dac::new`.
#[test]
fn t6() {
    let dac = Dac::new(
        analog_pin("VREF_OUT", "PA4"),
        2,
        "REF_VOLTAGE_OUT".to_string(),
        "hdac1".to_string(),
    );
    assert_eq!(dac.channel(), 2);
}
