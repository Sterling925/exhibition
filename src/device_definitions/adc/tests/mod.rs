// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`Adc`](super::Adc) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::Adc;
use super::Gpio;

/// Helper to build a valid input Gpio for use as an ADC channel input.
fn input_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), false)
}

/// t1
///
/// Verify that the accessors `pin()`, `name()`, and `id()` return exactly
/// the values supplied to `Adc::new`.
#[test]
fn t1() {
    let pin = input_pin("BAT_SENSE", "PA0");
    let adc = Adc::new(pin.clone(), 0, "BATTERY_VOLTAGE".to_string(), "hadc1".to_string());

    assert!(adc.pin() == &pin);
    assert_eq!(adc.name(), "BATTERY_VOLTAGE");
    assert_eq!(adc.id(), "hadc1");
}

/// t2
///
/// Verify that `is_valid()` accepts a fully populated definition with a
/// valid pin and non-empty `name` and `id`.
#[test]
fn t2() {
    let adc = Adc::new(
        input_pin("BAT_SENSE", "PA0"),
        0,
        "BATTERY_VOLTAGE".to_string(),
        "hadc1".to_string(),
    );
    assert!(adc.is_valid().is_ok());
}

/// t3
///
/// Verify that `is_valid()` rejects a definition whose pin is not itself
/// a valid Gpio definition.
#[test]
fn t3() {
    let bad_pin = input_pin("BAT_SENSE", "QQ99");
    assert!(bad_pin.is_valid().is_err(), "test precondition: pin must be invalid");

    let adc = Adc::new(bad_pin, 0, "BATTERY_VOLTAGE".to_string(), "hadc1".to_string());
    assert!(adc.is_valid().is_err());
}

/// t4
///
/// Verify that `is_valid()` rejects a definition whose `name` field is
/// the empty string.
#[test]
fn t4() {
    let adc = Adc::new(
        input_pin("BAT_SENSE", "PA0"),
        0,
        String::new(),
        "hadc1".to_string(),
    );
    assert!(adc.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a definition whose `id` field is
/// the empty string.
#[test]
fn t5() {
    let adc = Adc::new(
        input_pin("BAT_SENSE", "PA0"),
        0,
        "BATTERY_VOLTAGE".to_string(),
        String::new(),
    );
    assert!(adc.is_valid().is_err());
}

/// t6
///
/// Verify that the `channel()` accessor returns exactly the value supplied
/// to `Adc::new`.
#[test]
fn t6() {
    let adc = Adc::new(
        input_pin("BAT_SENSE", "PA0"),
        7,
        "BATTERY_VOLTAGE".to_string(),
        "hadc1".to_string(),
    );
    assert_eq!(adc.channel(), 7);
}
