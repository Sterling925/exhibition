// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

use crate::device_definitions::qspi::Qspi;

/// Valid Qspi with the standard handle id is_valid returns Ok.
#[test]
fn t1() {
    let q = Qspi::new("hqspi".to_string());
    assert!(q.is_valid().is_ok());
    assert_eq!(q.id(), "hqspi");
}

/// Qspi with an empty id is_valid returns Err.
#[test]
fn t2() {
    let q = Qspi::new(String::new());
    assert!(q.is_valid().is_err());
}

/// Clone preserves the id.
#[test]
fn t3() {
    let q = Qspi::new("hqspi".to_string());
    let c = q.clone();
    assert_eq!(c.id(), q.id());
}
