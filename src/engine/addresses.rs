// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Sub-address assignment for the generated RT firmware.
//!
//! Sub-address 1 is reserved by convention for the system handler that
//! serves the API fingerprint and the registered-device count. Peripheral
//! devices receive sub-addresses 2..31 (the protocol's usable range is
//! 1..31 since 0 is reserved by the wire format). The order is fully
//! deterministic: all peripherals are sorted lexicographically by
//! `"{type}::{sort_key}"`, then assigned in that order. Re-running the
//! generator on the same input produces the same mapping.
//!
//! GPIO pins are grouped per `(direction, bank)` — one slot represents
//! every input pin on port A, another slot every output pin on port B,
//! and so on. The wire payload carries a pin-index byte so a single
//! sub-address can serve up to 16 pins per bank, drastically reducing
//! sub-address pressure compared to a one-pin-per-slot design.

use std::collections::HashMap;

use crate::device_definitions::adc::Adc;
use crate::device_definitions::bus::can::BusCan;
use crate::device_definitions::bus::i2c::BusI2c;
use crate::device_definitions::bus::spi::BusSpi;
use crate::device_definitions::bus::uart::BusUart;
use crate::device_definitions::dac::Dac;
use crate::device_definitions::gpio::Gpio;
use crate::device_definitions::pwm::Pwm;
use crate::device_definitions::qspi::Qspi;
use crate::device_definitions::rtc::Rtc;
use crate::ioc_parser::DeviceSet;


/// Reserved sub-address for the system handler.
pub const SYSTEM_SUB_ADDRESS: u8 = 1;

/// First sub-address available for peripheral devices.
pub const FIRST_DEVICE_SUB_ADDRESS: u8 = 2;

/// One past the last usable sub-address (the protocol field is 8 bits but
/// the scheduler caps `kMaxSubAddresses` at 32, leaving 1..=31 usable).
pub const SUB_ADDRESS_LIMIT: u8 = 32;


/// Peripheral types recognized by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    GpioOutput,
    GpioInput,
    Adc,
    Dac,
    Pwm,
    Qspi,
    Rtc,
    Spi,
    Uart,
    I2c,
    Can,
}

impl DeviceKind {
    /// Sortable name. Sort order is the lexicographic order of these
    /// strings, so `adc` precedes `can` precedes `dac`, etc.
    pub fn sort_key(self) -> &'static str {
        match self {
            DeviceKind::Adc        => "adc",
            DeviceKind::Can        => "can",
            DeviceKind::Dac        => "dac",
            DeviceKind::GpioInput  => "gpio_in",
            DeviceKind::GpioOutput => "gpio_out",
            DeviceKind::I2c        => "i2c",
            DeviceKind::Pwm        => "pwm",
            DeviceKind::Qspi       => "qspi",
            DeviceKind::Rtc        => "rtc",
            DeviceKind::Spi        => "spi",
            DeviceKind::Uart       => "uart",
        }
    }

    /// Human-readable name used in the YAML manifest's `type:` field.
    pub fn manifest_name(self) -> &'static str {
        self.sort_key()
    }
}


/// One client-facing object backed by a [`DeviceSlot`].
///
/// For most peripheral kinds the slot maps 1:1 to a single member: a
/// `BusSpi` slot has one `SpiClient` member, an `Adc` slot has one
/// `AdcClient` member, etc. GPIO bank slots are the exception — one slot
/// (one sub-address, one handler) backs one member per configured pin in
/// the bank, and the pin's index is encoded in the command payload so the
/// RT handler can dispatch to the correct HAL pin.
#[derive(Clone, Debug)]
pub struct SlotMember {
    /// Identifier suitable as a C++ field name on `RtClient`
    /// (e.g. `gpio_PA5`, `adc_hadc1_ch3`).
    pub instance_name: String,
    /// Programming name from the IOC file (e.g. `GREEN_LED_PIN`). Falls
    /// back to `instance_name` if no separate label exists.
    pub label: String,
    /// Pin index 0..=15 within the bank for GPIO members; `None` for
    /// non-GPIO members. Encoded into the command payload by the BC
    /// client method bodies for GPIO.
    pub pin_index: Option<u8>,
}


/// A peripheral that has been assigned a sub-address.
///
/// The `details` map carries per-kind information consumed by the C++
/// registry generator and by the YAML manifest writer (HAL handle name,
/// pin id, channel number, etc.). Keeping it as a list of key/value pairs
/// (rather than a kind-specific struct per variant) keeps the manifest
/// writer simple and stable.
///
/// `members` enumerates the client-facing objects backed by this slot.
/// Non-GPIO slots always carry exactly one member; GPIO bank slots carry
/// one entry per configured pin in the bank.
#[derive(Clone, Debug)]
pub struct DeviceSlot {
    pub sub_address: u8,
    pub kind: DeviceKind,
    /// `kind.sort_key() + "::" + unique key for the device`.
    pub sort_key: String,
    /// Name suitable for use as a C++ identifier suffix for the on-RT
    /// handler instance (e.g. `gpio_out_bank_A`, `adc_hadc1_ch3`).
    pub instance_name: String,
    /// Human-readable description of the slot, used in the integration
    /// guide and the YAML manifest. For GPIO banks, the per-pin labels
    /// live in `members`.
    pub label: String,
    /// Ordered (key, value) pairs describing the peripheral.
    pub details: Vec<(String, String)>,
    /// Client-facing members backed by this slot.
    pub members: Vec<SlotMember>,
}


/// Assign sub-addresses to every peripheral in `devices`, excluding the
/// UART selected as the RT's own transport.
///
/// # Errors
/// Returns `Err` if more than 30 peripheral *slots* would need to be
/// addressed after the transport UART is removed (the protocol's usable
/// range is 1..31 and sub-address 1 is reserved for the system handler).
/// GPIO slot count is determined per `(direction, bank)` after grouping,
/// so a board with many GPIOs in a few banks still uses only a handful
/// of slots.
pub fn assign_sub_addresses(
    devices: &DeviceSet,
    rt_transport_uart_name: Option<&str>,
) -> Result<Vec<DeviceSlot>, String> {
    let mut bare: Vec<DeviceSlot> = Vec::new();

    bare.extend(slots_for_gpio_banks(&devices.gpios));
    for a in &devices.adcs {
        bare.push(slot_for_adc(a));
    }
    for d in &devices.dacs {
        bare.push(slot_for_dac(d));
    }
    for p in &devices.pwms {
        bare.push(slot_for_pwm(p));
    }
    for b in &devices.spi_buses {
        bare.push(slot_for_spi(b));
    }
    for b in &devices.uart_buses {
        // Skip the UART that the user selected as the RT's transport.
        if rt_transport_uart_name.map(|n| n == b.name()).unwrap_or(false) {
            continue;
        }
        bare.push(slot_for_uart(b));
    }
    for b in &devices.i2c_buses {
        bare.push(slot_for_i2c(b));
    }
    for b in &devices.can_buses {
        bare.push(slot_for_can(b));
    }
    for q in &devices.qspis {
        bare.push(slot_for_qspi(q));
    }
    for r in &devices.rtcs {
        bare.push(slot_for_rtc(r));
    }

    bare.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));

    let max_devices = (SUB_ADDRESS_LIMIT - FIRST_DEVICE_SUB_ADDRESS) as usize;
    if bare.len() > max_devices {
        return Err(format!(
            "Too many devices ({}): only {} sub-addresses available (2..{} inclusive)",
            bare.len(),
            max_devices,
            SUB_ADDRESS_LIMIT - 1
        ));
    }

    for (idx, slot) in bare.iter_mut().enumerate() {
        slot.sub_address = FIRST_DEVICE_SUB_ADDRESS + idx as u8;
    }

    Ok(bare)
}


/// Build one slot per `(direction, bank)` group present in `gpios`.
///
/// Pins with malformed identifiers (`bank()` or `pin()` returns `None`)
/// or with a pin number outside `0..=15` are silently dropped at this
/// layer; upstream `Gpio::is_valid` is expected to have caught them.
fn slots_for_gpio_banks(gpios: &[Gpio]) -> Vec<DeviceSlot> {
    let mut grouped: HashMap<(DeviceKind, char), Vec<&Gpio>> = HashMap::new();
    for g in gpios {
        let bank = match g.bank() {
            Some(b) => b,
            None => continue,
        };
        let kind = if g.output() {
            DeviceKind::GpioOutput
        } else {
            DeviceKind::GpioInput
        };
        grouped.entry((kind, bank)).or_default().push(g);
    }

    let mut out: Vec<DeviceSlot> = Vec::with_capacity(grouped.len());
    for ((kind, bank), mut pins) in grouped {
        pins.sort_by_key(|g| g.pin().unwrap_or(0));

        let mut pin_mask: u16 = 0;
        let mut members: Vec<SlotMember> = Vec::with_capacity(pins.len());
        for g in &pins {
            let Some(p) = g.pin() else { continue };
            if p > 15 {
                continue;
            }
            pin_mask |= 1u16 << p;
            members.push(SlotMember {
                instance_name: format!("gpio_{}", g.id()),
                label: g.name().to_string(),
                pin_index: Some(p as u8),
            });
        }

        let dir_short = match kind {
            DeviceKind::GpioOutput => "out",
            DeviceKind::GpioInput  => "in",
            _ => unreachable!(),
        };
        let dir_long = match kind {
            DeviceKind::GpioOutput => "output",
            DeviceKind::GpioInput  => "input",
            _ => unreachable!(),
        };
        let instance_name = format!("gpio_{}_bank_{}", dir_short, bank);
        let sort_key = format!("{}::bank_{}", kind.sort_key(), bank);
        let label = format!("GPIO {} bank {}", dir_long, bank);
        let details = vec![
            ("port".to_string(), format!("GPIO{}", bank)),
            ("pin_mask".to_string(), format!("0x{:04X}", pin_mask)),
        ];

        out.push(DeviceSlot {
            sub_address: 0,
            kind,
            sort_key,
            instance_name,
            label,
            details,
            members,
        });
    }
    out
}


fn slot_for_adc(a: &Adc) -> DeviceSlot {
    let key = format!("{}_ch{}", a.id(), a.channel());
    let instance_name = format!("adc_{}", key);
    let label = a.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Adc,
        sort_key: format!("{}::{}", DeviceKind::Adc.sort_key(), key),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![
            ("hal_handle".to_string(), format!("&{}", a.id())),
            ("channel".to_string(), a.channel().to_string()),
            ("pin_id".to_string(), a.pin().id().to_string()),
        ],
    }
}


fn slot_for_dac(d: &Dac) -> DeviceSlot {
    let key = format!("{}_ch{}", d.id(), d.channel());
    let instance_name = format!("dac_{}", key);
    let label = d.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Dac,
        sort_key: format!("{}::{}", DeviceKind::Dac.sort_key(), key),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![
            ("hal_handle".to_string(), format!("&{}", d.id())),
            ("channel".to_string(), format!("DAC_CHANNEL_{}", d.channel())),
            ("pin_id".to_string(), d.pin().id().to_string()),
        ],
    }
}


fn slot_for_pwm(p: &Pwm) -> DeviceSlot {
    let key = format!("{}_ch{}", p.id(), p.channel());
    let instance_name = format!("pwm_{}", key);
    let label = p.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Pwm,
        sort_key: format!("{}::{}", DeviceKind::Pwm.sort_key(), key),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![
            ("hal_handle".to_string(), format!("&{}", p.id())),
            ("channel".to_string(), format!("TIM_CHANNEL_{}", p.channel())),
            ("pin_id".to_string(), p.pin().id().to_string()),
        ],
    }
}


fn slot_for_spi(b: &BusSpi) -> DeviceSlot {
    let instance_name = format!("spi_{}", b.id());
    let label = b.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Spi,
        sort_key: format!("{}::{}", DeviceKind::Spi.sort_key(), b.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", b.id()))],
    }
}


fn slot_for_uart(b: &BusUart) -> DeviceSlot {
    let instance_name = format!("uart_{}", b.id());
    let label = b.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Uart,
        sort_key: format!("{}::{}", DeviceKind::Uart.sort_key(), b.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", b.id()))],
    }
}


fn slot_for_i2c(b: &BusI2c) -> DeviceSlot {
    let instance_name = format!("i2c_{}", b.id());
    let label = b.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::I2c,
        sort_key: format!("{}::{}", DeviceKind::I2c.sort_key(), b.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", b.id()))],
    }
}


fn slot_for_can(b: &BusCan) -> DeviceSlot {
    let instance_name = format!("can_{}", b.id());
    let label = b.name().to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Can,
        sort_key: format!("{}::{}", DeviceKind::Can.sort_key(), b.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", b.id()))],
    }
}


fn slot_for_qspi(q: &Qspi) -> DeviceSlot {
    let instance_name = "qspi".to_string();
    let label = "QSPI".to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Qspi,
        sort_key: format!("{}::{}", DeviceKind::Qspi.sort_key(), q.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", q.id()))],
    }
}


fn slot_for_rtc(r: &Rtc) -> DeviceSlot {
    let instance_name = "rtc".to_string();
    let label = "RTC".to_string();
    DeviceSlot {
        sub_address: 0,
        kind: DeviceKind::Rtc,
        sort_key: format!("{}::{}", DeviceKind::Rtc.sort_key(), r.id()),
        members: vec![single_self_member(&instance_name, &label)],
        instance_name,
        label,
        details: vec![("hal_handle".to_string(), format!("&{}", r.id()))],
    }
}


fn single_self_member(instance_name: &str, label: &str) -> SlotMember {
    SlotMember {
        instance_name: instance_name.to_string(),
        label: label.to_string(),
        pin_index: None,
    }
}
