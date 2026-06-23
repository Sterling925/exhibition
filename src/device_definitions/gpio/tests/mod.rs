// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`Gpio`](super::Gpio) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))] 
compile_error!("This file is only to be used in test build");


use super::Gpio;

/// t1
///
/// Verify that `bank()` returns the expected bank character for well-formed
/// single-digit and double-digit pin identifiers.
/// Expects `Some('A')` for `"PA1"` and `Some('B')` for `"PB13"`.
#[test]
fn t1() {
    let g = Gpio::new("LED".to_string(), "PA1".to_string(), true);
    assert_eq!(g.bank(), Some('A'));

    let g = Gpio::new("LED".to_string(), "PB13".to_string(), true);
    assert_eq!(g.bank(), Some('B'));
}

/// t2
///
/// Verify that `pin()` returns the expected numeric pin for well-formed
/// single-digit and double-digit pin identifiers.
/// Expects `Some(1)` for `"PA1"` and `Some(13)` for `"PB13"`.
#[test]
fn t2() {
    let g = Gpio::new("LED".to_string(), "PA1".to_string(), true);
    assert_eq!(g.pin(), Some(1));

    let g = Gpio::new("LED".to_string(), "PB13".to_string(), true);
    assert_eq!(g.pin(), Some(13));
}

/// t3
///
/// Verify that `bank()` and `pin()` return `None` for malformed identifiers
/// that do not start with `'P'` or are missing required components.
#[test]
fn t3() {
    let g = Gpio::new("X".to_string(), "QA1".to_string(), true);
    assert_eq!(g.bank(), None);
    assert_eq!(g.pin(), None);

    let g = Gpio::new("X".to_string(), "P".to_string(), true);
    assert_eq!(g.bank(), None);
    assert_eq!(g.pin(), None);

    let g = Gpio::new("X".to_string(), "PA".to_string(), true);
    // Bank can still be extracted, but the pin number is missing.
    assert_eq!(g.bank(), Some('A'));
    assert_eq!(g.pin(), None);
}

/// t4
///
/// Verify that `is_valid()` accepts identifiers whose bank lies within
/// `'A'..='K'` and whose pin lies within `0..=15`.
/// Expects `Ok(())` for `"PA0"`, `"PB15"`, and `"PK7"`.
#[test]
fn t4() {
    for id in ["PA0", "PB15", "PK7"] {
        let g = Gpio::new("X".to_string(), id.to_string(), true);
        assert!(g.is_valid().is_ok(), "expected '{}' to be valid", id);
    }
}

/// t5
///
/// Verify that `is_valid()` rejects identifiers with a pin number outside
/// the supported `[0, 15]` range.
/// Expects an `Err` for `"PA16"` and `"PB99"`.
#[test]
fn t5() {
    let g = Gpio::new("X".to_string(), "PA16".to_string(), true);
    assert!(g.is_valid().is_err());

    let g = Gpio::new("X".to_string(), "PB99".to_string(), true);
    assert!(g.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects identifiers whose bank character lies
/// outside the supported `'A'..='K'` range.
/// Expects an `Err` for `"PL0"` and `"PZ3"`.
#[test]
fn t6() {
    let g = Gpio::new("X".to_string(), "PL0".to_string(), true);
    assert!(g.is_valid().is_err());

    let g = Gpio::new("X".to_string(), "PZ3".to_string(), true);
    assert!(g.is_valid().is_err());
}

/// t7
///
/// Verify that `is_valid()` rejects identifiers that are entirely malformed
/// (do not begin with `'P'`, missing pin number, empty string, etc.).
#[test]
fn t7() {
    let g = Gpio::new("X".to_string(), "QA1".to_string(), true);
    assert!(g.is_valid().is_err());

    let g = Gpio::new("X".to_string(), "PA".to_string(), true);
    assert!(g.is_valid().is_err());

    let g = Gpio::new("X".to_string(), String::new(), true);
    assert!(g.is_valid().is_err());
}

/// t8
///
/// Verify that the simple accessors `name()`, `id()`, and `output()` return
/// exactly the values supplied to `Gpio::new`.
#[test]
fn t8() {
    let g = Gpio::new("GREEN_LED_PIN".to_string(), "PC5".to_string(), true);
    assert_eq!(g.name(), "GREEN_LED_PIN");
    assert_eq!(g.id(), "PC5");
    assert_eq!(g.output(), true);

    let g = Gpio::new("BUTTON".to_string(), "PD2".to_string(), false);
    assert_eq!(g.output(), false);
}
