// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

use crate::device_definitions::rtc::Rtc;

/// Valid Rtc with the standard handle id is_valid returns Ok.
#[test]
fn t1() {
    let r = Rtc::new("hrtc".to_string());
    assert!(r.is_valid().is_ok());
    assert_eq!(r.id(), "hrtc");
}

/// Rtc with an empty id is_valid returns Err.
#[test]
fn t2() {
    let r = Rtc::new(String::new());
    assert!(r.is_valid().is_err());
}

/// Clone preserves the id.
#[test]
fn t3() {
    let r = Rtc::new("hrtc".to_string());
    let c = r.clone();
    assert_eq!(c.id(), r.id());
}
