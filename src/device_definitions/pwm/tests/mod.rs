// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`Pwm`](super::Pwm) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::Pwm;
use super::Gpio;

/// Helper to build a valid output Gpio for use as a PWM output.
fn output_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// Helper to build a valid input Gpio (for the negative `output` test).
fn input_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), false)
}

/// t1
///
/// Verify that the accessors `pin()`, `name()`, and `id()` return exactly
/// the values supplied to `Pwm::new`.
#[test]
fn t1() {
    let pin = output_pin("MOTOR", "PA0");
    let pwm = Pwm::new(pin.clone(), 1, "MOTOR_PWM".to_string(), "htim2".to_string());

    assert!(pwm.pin() == &pin);
    assert_eq!(pwm.name(), "MOTOR_PWM");
    assert_eq!(pwm.id(), "htim2");
}

/// t2
///
/// Verify that `is_valid()` accepts a fully populated definition with a
/// valid output pin and non-empty `name` and `id`.
#[test]
fn t2() {
    let pwm = Pwm::new(
        output_pin("MOTOR", "PA0"),
        1,
        "MOTOR_PWM".to_string(),
        "htim2".to_string(),
    );
    assert!(pwm.is_valid().is_ok());
}

/// t3
///
/// Verify that `is_valid()` rejects a definition whose pin is not itself
/// a valid Gpio definition.
#[test]
fn t3() {
    let bad_pin = output_pin("MOTOR", "QQ99");
    assert!(bad_pin.is_valid().is_err(), "test precondition: pin must be invalid");

    let pwm = Pwm::new(bad_pin, 1, "MOTOR_PWM".to_string(), "htim2".to_string());
    assert!(pwm.is_valid().is_err());
}

/// t4
///
/// Verify that `is_valid()` rejects a definition whose pin is configured
/// as an input. PWM channels must drive their pin.
#[test]
fn t4() {
    let pwm = Pwm::new(
        input_pin("MOTOR", "PA0"),
        1,
        "MOTOR_PWM".to_string(),
        "htim2".to_string(),
    );
    assert!(pwm.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a definition whose `name` field is
/// the empty string.
#[test]
fn t5() {
    let pwm = Pwm::new(
        output_pin("MOTOR", "PA0"),
        1,
        String::new(),
        "htim2".to_string(),
    );
    assert!(pwm.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a definition whose `id` field is
/// the empty string.
#[test]
fn t6() {
    let pwm = Pwm::new(
        output_pin("MOTOR", "PA0"),
        1,
        "MOTOR_PWM".to_string(),
        String::new(),
    );
    assert!(pwm.is_valid().is_err());
}

/// t7
///
/// Verify that the `channel()` accessor returns exactly the value supplied
/// to `Pwm::new`.
#[test]
fn t7() {
    let pwm = Pwm::new(
        output_pin("MOTOR", "PA0"),
        3,
        "MOTOR_PWM".to_string(),
        "htim2".to_string(),
    );
    assert_eq!(pwm.channel(), 3);
}
