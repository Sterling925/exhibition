// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`BusI2c`](super::BusI2c) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::BusI2c;
use super::Gpio;

/// Helper to build a valid Gpio for use as an I2C pin (open-drain output).
fn i2c_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// t1
///
/// Verify that the accessors `sda()`, `scl()`, `name()`, and `id()` return
/// exactly the values supplied to `BusI2c::new`.
#[test]
fn t1() {
    let sda = i2c_pin("I2C_SDA", "PB7");
    let scl = i2c_pin("I2C_SCL", "PB6");
    let bus = BusI2c::new(sda.clone(), scl.clone(), "SENSOR_I2C".to_string(), "hi2c1".to_string());

    assert!(bus.sda() == &sda);
    assert!(bus.scl() == &scl);
    assert_eq!(bus.name(), "SENSOR_I2C");
    assert_eq!(bus.id(), "hi2c1");
}

/// t2
///
/// Verify that `is_valid()` accepts a fully populated bus definition with
/// valid SDA and SCL pins and non-empty `name` and `id`.
#[test]
fn t2() {
    let bus = BusI2c::new(
        i2c_pin("I2C_SDA", "PB7"),
        i2c_pin("I2C_SCL", "PB6"),
        "SENSOR_I2C".to_string(),
        "hi2c1".to_string(),
    );
    assert!(bus.is_valid().is_ok());
}

/// t3
///
/// Verify that `is_valid()` rejects a bus definition whose SDA pin is not
/// itself a valid Gpio definition.
#[test]
fn t3() {
    let bad_sda = i2c_pin("I2C_SDA", "QQ99");
    assert!(bad_sda.is_valid().is_err(), "test precondition: sda pin must be invalid");

    let bus = BusI2c::new(
        bad_sda,
        i2c_pin("I2C_SCL", "PB6"),
        "SENSOR_I2C".to_string(),
        "hi2c1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t4
///
/// Verify that `is_valid()` rejects a bus definition whose SCL pin is not
/// itself a valid Gpio definition.
#[test]
fn t4() {
    let bad_scl = i2c_pin("I2C_SCL", "QQ99");
    assert!(bad_scl.is_valid().is_err(), "test precondition: scl pin must be invalid");

    let bus = BusI2c::new(
        i2c_pin("I2C_SDA", "PB7"),
        bad_scl,
        "SENSOR_I2C".to_string(),
        "hi2c1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a bus definition whose `name` field is
/// the empty string.
#[test]
fn t5() {
    let bus = BusI2c::new(
        i2c_pin("I2C_SDA", "PB7"),
        i2c_pin("I2C_SCL", "PB6"),
        String::new(),
        "hi2c1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a bus definition whose `id` field is
/// the empty string.
#[test]
fn t6() {
    let bus = BusI2c::new(
        i2c_pin("I2C_SDA", "PB7"),
        i2c_pin("I2C_SCL", "PB6"),
        "SENSOR_I2C".to_string(),
        String::new(),
    );
    assert!(bus.is_valid().is_err());
}
