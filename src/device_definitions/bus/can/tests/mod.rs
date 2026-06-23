// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`BusCan`](super::BusCan) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::BusCan;
use super::Gpio;

/// Helper to build a valid output Gpio for use as a CAN TX pin.
fn output_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// Helper to build a valid input Gpio for use as a CAN RX pin.
fn input_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), false)
}

/// t1
///
/// Verify that the accessors `tx()`, `rx()`, `name()`, and `id()` return
/// exactly the values supplied to `BusCan::new`.
#[test]
fn t1() {
    let tx = output_pin("CAN_TX", "PA12");
    let rx = input_pin("CAN_RX", "PA11");
    let bus = BusCan::new(tx.clone(), rx.clone(), "MOTOR_CAN".to_string(), "hcan1".to_string());

    assert!(bus.tx() == &tx);
    assert!(bus.rx() == &rx);
    assert_eq!(bus.name(), "MOTOR_CAN");
    assert_eq!(bus.id(), "hcan1");
}

/// t2
///
/// Verify that `is_valid()` accepts a fully populated bus definition with
/// valid TX and RX pins and non-empty `name` and `id`.
#[test]
fn t2() {
    let bus = BusCan::new(
        output_pin("CAN_TX", "PA12"),
        input_pin("CAN_RX", "PA11"),
        "MOTOR_CAN".to_string(),
        "hcan1".to_string(),
    );
    assert!(bus.is_valid().is_ok());
}

/// t3
///
/// Verify that `is_valid()` rejects a bus definition whose TX pin is not
/// itself a valid Gpio definition.
#[test]
fn t3() {
    let bad_tx = output_pin("CAN_TX", "QQ99");
    assert!(bad_tx.is_valid().is_err(), "test precondition: tx pin must be invalid");

    let bus = BusCan::new(
        bad_tx,
        input_pin("CAN_RX", "PA11"),
        "MOTOR_CAN".to_string(),
        "hcan1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t4
///
/// Verify that `is_valid()` rejects a bus definition whose RX pin is not
/// itself a valid Gpio definition.
#[test]
fn t4() {
    let bad_rx = input_pin("CAN_RX", "QQ99");
    assert!(bad_rx.is_valid().is_err(), "test precondition: rx pin must be invalid");

    let bus = BusCan::new(
        output_pin("CAN_TX", "PA12"),
        bad_rx,
        "MOTOR_CAN".to_string(),
        "hcan1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a bus definition whose `name` field is
/// the empty string.
#[test]
fn t5() {
    let bus = BusCan::new(
        output_pin("CAN_TX", "PA12"),
        input_pin("CAN_RX", "PA11"),
        String::new(),
        "hcan1".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a bus definition whose `id` field is
/// the empty string.
#[test]
fn t6() {
    let bus = BusCan::new(
        output_pin("CAN_TX", "PA12"),
        input_pin("CAN_RX", "PA11"),
        "MOTOR_CAN".to_string(),
        String::new(),
    );
    assert!(bus.is_valid().is_err());
}
