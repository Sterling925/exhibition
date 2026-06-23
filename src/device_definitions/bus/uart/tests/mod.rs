// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`BusUart`](super::BusUart) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::BusUart;
use super::Gpio;

/// Helper to build a valid output Gpio for use as a TX pin.
fn output_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), true)
}

/// Helper to build a valid input Gpio for use as an RX pin.
fn input_pin(name: &str, id: &str) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), false)
}

/// t1
///
/// Verify that the simple accessors `name()` and `id()` return exactly the
/// values supplied to `BusUart::new`.
#[test]
fn t1() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        "DEBUG_UART".to_string(),
        "huart1".to_string(),
        None,
    );

    assert_eq!(bus.name(), "DEBUG_UART");
    assert_eq!(bus.id(), "huart1");
}

/// t2
///
/// Verify that `tx()` and `rx()` return clones of the supplied pins when
/// present, and `None` when not supplied.
#[test]
fn t2() {
    let tx = output_pin("TX", "PA9");
    let rx = input_pin("RX", "PA10");

    let bus = BusUart::new(
        Some(tx.clone()),
        Some(rx.clone()),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.tx() == Some(tx));
    assert!(bus.rx() == Some(rx));

    let bus_no_tx = BusUart::new(
        None,
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus_no_tx.tx().is_none());
    assert!(bus_no_tx.rx().is_some());

    let bus_no_rx = BusUart::new(
        Some(output_pin("TX", "PA9")),
        None,
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus_no_rx.tx().is_some());
    assert!(bus_no_rx.rx().is_none());
}

/// t3
///
/// Verify that `is_valid()` accepts a fully populated bus definition with
/// both TX and RX present, as well as transmit-only and receive-only
/// configurations.
#[test]
fn t3() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.is_valid().is_ok());

    // Transmit-only bus (TX only) is acceptable.
    let tx_only = BusUart::new(
        Some(output_pin("TX", "PA9")),
        None,
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(tx_only.is_valid().is_ok());

    // Receive-only bus (RX only) is acceptable.
    let rx_only = BusUart::new(
        None,
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(rx_only.is_valid().is_ok());
}

/// t4
///
/// Verify that `is_valid()` rejects a bus definition that has neither TX
/// nor RX defined, since such a bus could neither transmit nor receive.
#[test]
fn t4() {
    let bus = BusUart::new(
        None,
        None,
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a bus definition whose TX pin is not
/// itself a valid Gpio definition.
#[test]
fn t5() {
    let bad_tx = output_pin("TX", "QQ99");
    assert!(bad_tx.is_valid().is_err(), "test precondition: tx pin must be invalid");

    let bus = BusUart::new(
        Some(bad_tx),
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a bus definition whose RX pin is not
/// itself a valid Gpio definition.
#[test]
fn t6() {
    let bad_rx = input_pin("RX", "QQ99");
    assert!(bad_rx.is_valid().is_err(), "test precondition: rx pin must be invalid");

    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(bad_rx),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.is_valid().is_err());
}

/// t7
///
/// Verify that `is_valid()` rejects a bus definition whose `name` field is
/// the empty string.
#[test]
fn t7() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        String::new(),
        "huart1".to_string(),
        None,
    );
    assert!(bus.is_valid().is_err());
}

/// t8
///
/// Verify that `is_valid()` rejects a bus definition whose `id` field is
/// the empty string.
#[test]
fn t8() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        String::new(),
        None,
    );
    assert!(bus.is_valid().is_err());
}

/// t9
///
/// Verify that a baud rate supplied to `BusUart::new` is preserved by
/// `baud_rate()`, and that providing one does not break validation.
#[test]
fn t9() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        Some(38400),
    );
    assert_eq!(bus.baud_rate(), Some(38400));
    assert!(bus.is_valid().is_ok());
}

/// t10
///
/// Verify that an absent baud rate (`None`) is reported back as `None` by
/// `baud_rate()` and does not affect validation — a missing
/// `<PERIPH>.BaudRate` in the `.ioc` must not invalidate the bus.
#[test]
fn t10() {
    let bus = BusUart::new(
        Some(output_pin("TX", "PA9")),
        Some(input_pin("RX", "PA10")),
        "BUS".to_string(),
        "huart1".to_string(),
        None,
    );
    assert_eq!(bus.baud_rate(), None);
    assert!(bus.is_valid().is_ok());
}
