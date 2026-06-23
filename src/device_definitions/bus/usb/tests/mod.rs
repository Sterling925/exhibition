// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`BusUsb`](super::BusUsb) definition container.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::BusUsb;
use super::Gpio;

/// Helper to build a valid Gpio for a USB pin.
fn usb_pin(name: &str, id: &str, output: bool) -> Gpio {
    Gpio::new(name.to_string(), id.to_string(), output)
}

/// t1
///
/// Verify that the simple accessors `dp()`, `dm()`, `name()`, and `id()`
/// return exactly the values supplied to `BusUsb::new`.
#[test]
fn t1() {
    let dp = usb_pin("USB_DP", "PA12", true);
    let dm = usb_pin("USB_DM", "PA11", true);
    let bus = BusUsb::new(
        dp.clone(),
        dm.clone(),
        None,
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );

    assert!(bus.dp() == &dp);
    assert!(bus.dm() == &dm);
    assert_eq!(bus.name(), "HOST_USB");
    assert_eq!(bus.id(), "husb_otg_fs");
}

/// t2
///
/// Verify that `vbus()` and `id_pin()` return clones of the supplied pins
/// when present, and `None` when not supplied.
#[test]
fn t2() {
    let vbus = usb_pin("USB_VBUS", "PA9", false);
    let id_pin = usb_pin("USB_ID", "PA10", false);

    let bus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        Some(vbus.clone()),
        Some(id_pin.clone()),
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus.vbus() == Some(vbus));
    assert!(bus.id_pin() == Some(id_pin));

    let bus_no_opts = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus_no_opts.vbus().is_none());
    assert!(bus_no_opts.id_pin().is_none());
}

/// t3
///
/// Verify that `is_valid()` accepts a fully populated bus definition with
/// just D+ and D- (no optional pins) and non-empty `name` and `id`.
#[test]
fn t3() {
    let bus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus.is_valid().is_ok());

    // OTG configuration with both optional pins is also acceptable.
    let bus_otg = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        Some(usb_pin("USB_VBUS", "PA9", false)),
        Some(usb_pin("USB_ID", "PA10", false)),
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus_otg.is_valid().is_ok());
}

/// t4
///
/// Verify that `is_valid()` rejects a bus definition whose D+ pin is not
/// itself a valid Gpio definition.
#[test]
fn t4() {
    let bad_dp = usb_pin("USB_DP", "QQ99", true);
    assert!(bad_dp.is_valid().is_err(), "test precondition: dp pin must be invalid");

    let bus = BusUsb::new(
        bad_dp,
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t5
///
/// Verify that `is_valid()` rejects a bus definition whose D- pin is not
/// itself a valid Gpio definition.
#[test]
fn t5() {
    let bad_dm = usb_pin("USB_DM", "QQ99", true);
    assert!(bad_dm.is_valid().is_err(), "test precondition: dm pin must be invalid");

    let bus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        bad_dm,
        None,
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t6
///
/// Verify that `is_valid()` rejects a bus definition whose optional VBUS or
/// ID pin is present but not itself a valid Gpio definition.
#[test]
fn t6() {
    let bus_bad_vbus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        Some(usb_pin("USB_VBUS", "QQ99", false)),
        None,
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus_bad_vbus.is_valid().is_err());

    let bus_bad_id = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        Some(usb_pin("USB_ID", "QQ99", false)),
        "HOST_USB".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus_bad_id.is_valid().is_err());
}

/// t7
///
/// Verify that `is_valid()` rejects a bus definition whose `name` field is
/// the empty string.
#[test]
fn t7() {
    let bus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        String::new(),
        "husb_otg_fs".to_string(),
    );
    assert!(bus.is_valid().is_err());
}

/// t9
///
/// Verify the CDC-device flag: `new()` defaults to device-eligible (`true`),
/// and `with_cdc_device(false)` flips it (used by the parser for host-mode
/// USB blocks). The flag does not affect `is_valid()`.
#[test]
fn t9() {
    let base = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        "USB_OTG_FS".to_string(),
        "husb_otg_fs".to_string(),
    );
    assert!(base.is_cdc_device(), "new() must default to device-eligible");

    let host = base.with_cdc_device(false);
    assert!(!host.is_cdc_device(), "with_cdc_device(false) must clear the flag");
    assert!(host.is_valid().is_ok(), "mode flag must not affect validity");
}

/// t8
///
/// Verify that `is_valid()` rejects a bus definition whose `id` field is
/// the empty string.
#[test]
fn t8() {
    let bus = BusUsb::new(
        usb_pin("USB_DP", "PA12", true),
        usb_pin("USB_DM", "PA11", true),
        None,
        None,
        "HOST_USB".to_string(),
        String::new(),
    );
    assert!(bus.is_valid().is_err());
}
