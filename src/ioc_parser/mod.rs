// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Parser for STM32CubeMX `.ioc` project files.
//!
//! The parser consumes an `.ioc` file and returns a [`DeviceSet`] populated
//! with the [`Gpio`], [`BusSpi`], [`BusUart`], [`BusI2c`], [`BusCan`],
//! [`BusUsb`], [`Adc`], [`Dac`], [`Pwm`], [`Qspi`], and [`Rtc`] definitions
//! described by the project. The target MCU model is also extracted from the
//! `ProjectManager.DeviceId` entry into a [`Core`]. The existing
//! `is_valid()` methods on those types are the single source of truth for
//! validation; this module only classifies pins, groups them into buses,
//! and reports parse-time errors.
//!
//! # Entry points
//!
//! - [`parse_path`] reads from a file on disk.
//! - [`parse_str`] parses an in-memory `.ioc` document (used by tests and by
//!   callers that already have the file contents).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::device_definitions::adc::Adc;
use crate::device_definitions::bus::can::BusCan;
use crate::device_definitions::bus::i2c::BusI2c;
use crate::device_definitions::bus::spi::BusSpi;
use crate::device_definitions::bus::uart::BusUart;
use crate::device_definitions::bus::usb::BusUsb;
use crate::device_definitions::core::Core;
use crate::device_definitions::dac::Dac;
use crate::device_definitions::gpio::Gpio;
use crate::device_definitions::pwm::Pwm;
use crate::device_definitions::qspi::Qspi;
use crate::device_definitions::rtc::Rtc;
use crate::gen_log::GenLog;

#[cfg(test)]
mod tests;


/// How STM32CubeMX laid the project out on disk, derived from the
/// `ProjectManager.*` entries. Determines where generated sources must land
/// so the toolchain actually compiles them.
///
/// `Default` is the "under root" layout (sources build from `Core/Src`),
/// which is also the fallback when the `.ioc` omits the relevant keys (e.g.
/// small synthetic fixtures).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLayout {
    /// `ProjectManager.UnderRoot`. When `true`, user code builds from the
    /// project-root `Core/Src`; when `false`, CubeMX places it under the
    /// toolchain subfolder (`<toolchain>/Application/User/Core`).
    pub under_root: bool,
    /// `ProjectManager.TargetToolchain` (e.g. `"STM32CubeIDE"`). Empty when
    /// the `.ioc` does not specify one.
    pub toolchain: String,
}

impl Default for ProjectLayout {
    fn default() -> Self {
        ProjectLayout { under_root: true, toolchain: String::new() }
    }
}


/// Container for every device definition extracted from an `.ioc` file.
///
/// `core` is `None` when the source document has no `ProjectManager.DeviceId`
/// entry (e.g. small synthetic fixtures); real CubeMX exports always set it.
#[allow(dead_code)]
pub struct DeviceSet {
    pub core: Option<Core>,
    pub gpios: Vec<Gpio>,
    pub spi_buses: Vec<BusSpi>,
    pub uart_buses: Vec<BusUart>,
    pub i2c_buses: Vec<BusI2c>,
    pub can_buses: Vec<BusCan>,
    pub usb_buses: Vec<BusUsb>,
    pub adcs: Vec<Adc>,
    pub dacs: Vec<Dac>,
    pub pwms: Vec<Pwm>,
    pub qspis: Vec<Qspi>,
    pub rtcs: Vec<Rtc>,
    /// On-disk project layout, used by codegen to route generated sources.
    pub layout: ProjectLayout,
}


/// Parse an `.ioc` file from disk into a [`DeviceSet`].
///
/// Returns a human-readable error string on I/O failure or on any malformed
/// content discovered by [`parse_str`]. This non-logging form discards the
/// generation log; use [`parse_path_logged`] to capture it.
// Retained as the non-logging entry point exercised by the parser tests.
#[allow(dead_code)]
pub fn parse_path(path: &std::path::Path) -> Result<DeviceSet, String> {
    parse_path_logged(path, &mut GenLog::disabled())
}


/// Parse an `.ioc` file from disk into a [`DeviceSet`], recording which
/// devices were enabled (and why) and which signals were ignored into `log`.
pub fn parse_path_logged(
    path: &std::path::Path,
    log: &mut GenLog,
) -> Result<DeviceSet, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
    parse_str_logged(&content, log)
}


/// Parse an `.ioc` document held in memory into a [`DeviceSet`]. This
/// non-logging form discards the generation log; use [`parse_str_logged`] to
/// capture it.
// Retained as the non-logging entry point exercised by the parser tests.
#[allow(dead_code)]
pub fn parse_str(content: &str) -> Result<DeviceSet, String> {
    parse_str_logged(content, &mut GenLog::disabled())
}


/// Parse an `.ioc` document held in memory into a [`DeviceSet`], recording
/// parse-time decisions into `log`.
pub fn parse_str_logged(content: &str, log: &mut GenLog) -> Result<DeviceSet, String> {
    let entries = tokenize(content)?;
    let core = entries
        .get("ProjectManager.DeviceId")
        .map(|m| Core::new(m.clone()));
    let layout = parse_project_layout(&entries);
    let pin_ids = discover_pins(&entries);
    let adc_instance_map = collect_adc_instance_map(&entries);

    let mut acc = Accumulators::default();

    for (raw_key, pin_id) in &pin_ids {
        let signal_key = format!("{}.Signal", raw_key);
        let signal = entries
            .get(&signal_key)
            .ok_or_else(|| format!("Internal error: missing signal for pin '{}'", raw_key))?;
        let label = entries
            .get(&format!("{}.GPIO_Label", raw_key))
            .cloned()
            .unwrap_or_else(|| pin_id.clone());

        classify_pin(pin_id, signal, &label, &mut acc, &adc_instance_map, log)?;
    }

    let uart_baud = collect_uart_baud_rates(&entries)?;
    let usb_modes = collect_usb_device_modes(&entries, log);

    let mut spi_buses = build_spi_buses(acc.spi, log)?;
    let mut uart_buses = build_uart_buses(acc.uart, &uart_baud, log)?;
    let mut i2c_buses = build_i2c_buses(acc.i2c, log)?;
    let mut can_buses = build_can_buses(acc.can, log)?;
    let mut usb_buses = build_usb_buses(acc.usb, &usb_modes, log)?;

    let mut gpios = acc.gpios;
    let mut adcs = acc.adcs;
    let mut dacs = acc.dacs;
    let mut pwms = acc.pwms;
    let mut qspis: Vec<Qspi> = detect_qspi(&entries, log)?.into_iter().collect();
    let mut rtcs: Vec<Rtc> = detect_rtc(&entries, log)?.into_iter().collect();

    gpios.sort_by(|a, b| a.id().cmp(b.id()));
    spi_buses.sort_by(|a, b| a.name().cmp(b.name()));
    uart_buses.sort_by(|a, b| a.name().cmp(b.name()));
    i2c_buses.sort_by(|a, b| a.name().cmp(b.name()));
    can_buses.sort_by(|a, b| a.name().cmp(b.name()));
    usb_buses.sort_by(|a, b| a.name().cmp(b.name()));
    adcs.sort_by(|a, b| a.id().cmp(b.id()).then(a.channel().cmp(&b.channel())));
    dacs.sort_by(|a, b| a.id().cmp(b.id()).then(a.channel().cmp(&b.channel())));
    pwms.sort_by(|a, b| a.id().cmp(b.id()).then(a.channel().cmp(&b.channel())));
    qspis.sort_by(|a, b| a.id().cmp(b.id()));
    rtcs.sort_by(|a, b| a.id().cmp(b.id()));

    Ok(DeviceSet {
        core, gpios, spi_buses, uart_buses, i2c_buses, can_buses, usb_buses, adcs, dacs, pwms,
        qspis, rtcs, layout,
    })
}


/// Derive the [`ProjectLayout`] from the `ProjectManager.*` entries.
///
/// `ProjectManager.UnderRoot` is `true` unless the `.ioc` explicitly sets it
/// to `false`; a missing key (synthetic fixtures) therefore yields the
/// root layout. `ProjectManager.TargetToolchain` flows through verbatim, or
/// empty when absent.
fn parse_project_layout(entries: &HashMap<String, String>) -> ProjectLayout {
    let under_root = entries
        .get("ProjectManager.UnderRoot")
        .map(|v| !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true);
    let toolchain = entries
        .get("ProjectManager.TargetToolchain")
        .cloned()
        .unwrap_or_default();
    ProjectLayout { under_root, toolchain }
}


/// Decide, per USB peripheral key (`USB_OTG_FS`, `USB_OTG_HS`, `USB`),
/// whether it is configured as a USB **device** (CDC virtual-COM eligible)
/// rather than a USB **host**. Only device-mode blocks can back the RT's
/// USB-CDC transport.
///
/// The decision is, in priority order:
///   1. the peripheral's own `<periph>.VirtualMode` — `*Host*` ⇒ host,
///      `*Device*` ⇒ device;
///   2. otherwise, the middleware in use — a `USB_DEVICE.*` section ⇒ device,
///      a `USB_HOST.*` section ⇒ host;
///   3. otherwise (no mode and no middleware, e.g. a pins-only fixture) ⇒
///      device, preserving the historical permissive default.
fn collect_usb_device_modes(
    entries: &HashMap<String, String>,
    log: &mut GenLog,
) -> HashMap<String, bool> {
    let has_device_ip = entries.keys().any(|k| k.starts_with("USB_DEVICE."));
    let has_host_ip = entries.keys().any(|k| k.starts_with("USB_HOST."));

    let mut out = HashMap::new();
    for periph in ["USB_OTG_FS", "USB_OTG_HS", "USB"] {
        let (is_device, reason) = match entries.get(&format!("{}.VirtualMode", periph)) {
            Some(v) if v.contains("Host") => (false, format!("VirtualMode `{}`", v)),
            Some(v) if v.contains("Device") => (true, format!("VirtualMode `{}`", v)),
            _ if has_device_ip => (true, "USB_DEVICE middleware present".to_string()),
            _ if has_host_ip => (false, "USB_HOST middleware present".to_string()),
            _ => (true, "default (no mode or middleware specified)".to_string()),
        };
        // Only report the decision for peripherals the project actually
        // touches, to avoid logging the other two phantom USB instances.
        let configured = entries.keys().any(|k| k.starts_with(&format!("{}.", periph)));
        if configured {
            log.decision(format!(
                "USB `{}` resolved to {} mode — {}",
                periph,
                if is_device { "device" } else { "host" },
                reason,
            ));
        }
        out.insert(periph.to_string(), is_device);
    }
    out
}


/// Return an `Rtc` if the `.ioc` file has the RTC calendar feature enabled.
///
/// CubeMX represents virtual (non-pin) peripherals via `VP_*` entries. An
/// enabled RTC calendar is indicated by either of:
///   - `VP_RTC_VS_RTC_Calendar.Signal = RTC_VS_RTC_Calendar`
///   - `VP_RTC_VS_RTC_Activate.Signal = RTC_VS_RTC_Activate`
///
/// The HAL handle is always `hrtc` for STM32 targets. A detected instance is
/// validated through [`Rtc::is_valid`] before being returned, matching the
/// other peripheral detectors; a validation failure aborts the parse.
fn detect_rtc(entries: &HashMap<String, String>, log: &mut GenLog) -> Result<Option<Rtc>, String> {
    let calendar = entries
        .get("VP_RTC_VS_RTC_Calendar.Signal")
        .map(|v| v == "RTC_VS_RTC_Calendar")
        .unwrap_or(false);
    let activate = entries
        .get("VP_RTC_VS_RTC_Activate.Signal")
        .map(|v| v == "RTC_VS_RTC_Activate")
        .unwrap_or(false);
    if calendar || activate {
        let rtc = Rtc::new("hrtc".to_string());
        rtc.is_valid()
            .map_err(|e| format!("RTC: {}", e))?;
        let trigger = if calendar { "VP_RTC_VS_RTC_Calendar" } else { "VP_RTC_VS_RTC_Activate" };
        log.enabled_device("RTC", "hrtc", format!("virtual entry `{}`", trigger));
        Ok(Some(rtc))
    } else {
        Ok(None)
    }
}


/// Return a `Qspi` if the `.ioc` file enables the QUADSPI controller.
///
/// STM32 parts expose a single QUADSPI peripheral. It is detected either by a
/// pin whose `*.Signal` value names a QUADSPI function (`QUADSPI_CLK`,
/// `QUADSPI_BK1_NCS`, `QUADSPI_BK1_IO0`, …) or by a `Mcu.IP*` entry equal to
/// `QUADSPI`. The QUADSPI pin signals are not recognized by [`classify_pin`],
/// so they fall through un-classified and do not become spurious GPIOs.
///
/// The HAL handle is always `hqspi` for STM32 targets. A detected instance is
/// validated through [`Qspi::is_valid`] before being returned, matching the
/// other peripheral detectors; a validation failure aborts the parse.
fn detect_qspi(entries: &HashMap<String, String>, log: &mut GenLog) -> Result<Option<Qspi>, String> {
    let present = entries.iter().any(|(key, value)| {
        (key.ends_with(".Signal") && value.starts_with("QUADSPI"))
            || (key.starts_with("Mcu.IP") && value == "QUADSPI")
    });
    if present {
        let qspi = Qspi::new("hqspi".to_string());
        qspi.is_valid()
            .map_err(|e| format!("QSPI: {}", e))?;
        log.enabled_device("QSPI", "hqspi", "QUADSPI signal or `Mcu.IP*=QUADSPI` present");
        Ok(Some(qspi))
    } else {
        Ok(None)
    }
}


/// Per-peripheral accumulators populated while iterating the `.ioc` file.
#[derive(Default)]
struct Accumulators {
    gpios: Vec<Gpio>,
    spi: HashMap<String, SpiPins>,
    uart: HashMap<String, UartPins>,
    i2c: HashMap<String, I2cPins>,
    can: HashMap<String, CanPins>,
    usb: HashMap<String, UsbPins>,
    adcs: Vec<Adc>,
    dacs: Vec<Dac>,
    pwms: Vec<Pwm>,
}


#[derive(Default)]
struct SpiPins {
    sck: Option<Gpio>,
    miso: Option<Gpio>,
    mosi: Option<Gpio>,
    nss: Option<Gpio>,
}

#[derive(Default)]
struct UartPins {
    tx: Option<Gpio>,
    rx: Option<Gpio>,
}

#[derive(Default)]
struct I2cPins {
    sda: Option<Gpio>,
    scl: Option<Gpio>,
}

/// CAN family captured from the signal name; preserved so the build step can
/// emit the correct HAL handle (`hcan1` vs `hfdcan1`).
#[derive(Default)]
struct CanPins {
    family: String, // "CAN" or "FDCAN"
    tx: Option<Gpio>,
    rx: Option<Gpio>,
}

#[derive(Default)]
struct UsbPins {
    dp: Option<Gpio>,
    dm: Option<Gpio>,
    vbus: Option<Gpio>,
    id_pin: Option<Gpio>,
}


/// Split the `.ioc` text into a `key -> value` map.
///
/// Lines that are blank or that start (after trimming) with `#` are skipped.
/// A line that contains no `=` after trimming is reported as an error
/// referencing the 1-based line number.
fn tokenize(content: &str) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    for (idx, raw) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let eq = line
            .find('=')
            .ok_or_else(|| format!("Line {}: missing '='", line_no))?;
        let key = line[..eq].trim().to_string();
        let value = line[eq + 1..].trim().to_string();
        map.insert(key, value);
    }
    Ok(map)
}


/// Return the sorted set of pin entries configured in the file. Each entry
/// is `(raw_key_prefix, canonical_pin_id)`: the raw prefix is the exact
/// string CubeMX uses as the key (e.g. `PA0-WKUP`), needed to look up the
/// matching `.Signal`, `.GPIO_Label`, etc. lines; the canonical id is the
/// suffix-stripped `P<bank><pin>` form (e.g. `PA0`) used everywhere
/// downstream so [`Gpio::id`] is consistent regardless of which physical
/// pin variant was used.
fn discover_pins(entries: &HashMap<String, String>) -> Vec<(String, String)> {
    let re = pin_signal_regex();
    let mut pins: Vec<(String, String)> = entries
        .keys()
        .filter_map(|k| {
            re.captures(k).and_then(|c| {
                let canonical = c.get(1)?.as_str().to_string();
                let raw = k.strip_suffix(".Signal")?.to_string();
                Some((raw, canonical))
            })
        })
        .collect();
    pins.sort();
    pins.dedup();
    pins
}


/// Classify a single pin based on its `Signal` value, pushing into the
/// appropriate accumulator. Unrecognized signals are silently ignored (but
/// recorded in `log`).
///
/// GPIO, ADC, DAC and PWM pins each materialise one device here and are logged
/// as enabled immediately; SPI/UART/I2C/CAN/USB pins only accumulate, so their
/// "enabled" record is emitted later by the corresponding `build_*_buses`
/// function once the whole bus is known.
fn classify_pin(
    pin_id: &str,
    signal: &str,
    label: &str,
    acc: &mut Accumulators,
    adc_instance_map: &HashMap<u8, u32>,
    log: &mut GenLog,
) -> Result<(), String> {
    // Plain GPIO output.
    if signal == "GPIO_Output" {
        acc.gpios.push(Gpio::new(label.to_string(), pin_id.to_string(), true));
        log.enabled_device("GPIO", format!("{} (output)", pin_id),
                           format!("signal `{}`, label `{}`", signal, label));
        return Ok(());
    }

    // GPIO inputs: explicit Input, any EXTI line, and Analog channels all map
    // to non-output Gpios.
    if signal == "GPIO_Input"
        || signal == "GPIO_Analog"
        || signal.starts_with("GPIO_EXTI")
    {
        acc.gpios.push(Gpio::new(label.to_string(), pin_id.to_string(), false));
        log.enabled_device("GPIO", format!("{} (input)", pin_id),
                           format!("signal `{}`, label `{}`", signal, label));
        return Ok(());
    }

    // SPI peripheral pins.
    if let Some(caps) = spi_signal_regex().captures(signal) {
        let periph = format!("SPI{}", &caps[1]);
        let role = &caps[2];
        let entry = acc.spi.entry(periph.clone()).or_default();
        let is_output = role != "MISO";
        let gpio = Gpio::new(label.to_string(), pin_id.to_string(), is_output);
        match role {
            "SCK"  => entry.sck = Some(gpio),
            "MISO" => entry.miso = Some(gpio),
            "MOSI" => entry.mosi = Some(gpio),
            "NSS"  => entry.nss = Some(gpio),
            _ => unreachable!("regex restricts role to SCK|MISO|MOSI|NSS"),
        }
        return Ok(());
    }

    // USART / UART / LPUART peripheral pins.
    if let Some(caps) = uart_signal_regex().captures(signal) {
        let family = &caps[1];
        let num = &caps[2];
        let role = &caps[3];
        let periph = format!("{}{}", family, num);
        let entry = acc.uart.entry(periph.clone()).or_default();
        let is_output = role == "TX";
        let gpio = Gpio::new(label.to_string(), pin_id.to_string(), is_output);
        match role {
            "TX" => entry.tx = Some(gpio),
            "RX" => entry.rx = Some(gpio),
            _ => unreachable!("regex restricts role to TX|RX"),
        }
        return Ok(());
    }

    // I2C peripheral pins (SDA, SCL — open-drain outputs).
    if let Some(caps) = i2c_signal_regex().captures(signal) {
        let periph = format!("I2C{}", &caps[1]);
        let role = &caps[2];
        let entry = acc.i2c.entry(periph.clone()).or_default();
        let gpio = Gpio::new(label.to_string(), pin_id.to_string(), true);
        match role {
            "SDA" => entry.sda = Some(gpio),
            "SCL" => entry.scl = Some(gpio),
            _ => unreachable!("regex restricts role to SDA|SCL"),
        }
        return Ok(());
    }

    // CAN / FDCAN peripheral pins.
    if let Some(caps) = can_signal_regex().captures(signal) {
        let family = caps[1].to_string();
        let num = &caps[2];
        let role = &caps[3];
        let periph = format!("{}{}", family, num);
        let entry = acc.can.entry(periph.clone()).or_default();
        entry.family = family;
        let is_output = role == "TX";
        let gpio = Gpio::new(label.to_string(), pin_id.to_string(), is_output);
        match role {
            "TX" => entry.tx = Some(gpio),
            "RX" => entry.rx = Some(gpio),
            _ => unreachable!("regex restricts role to TX|RX"),
        }
        return Ok(());
    }

    // USB peripheral pins. Plain `USB_*` is one peripheral; `USB_OTG_FS_*`
    // and `USB_OTG_HS_*` are separate peripherals.
    if let Some(caps) = usb_signal_regex().captures(signal) {
        let family_key = match caps.get(1).map(|m| m.as_str()) {
            Some("FS") => "USB_OTG_FS".to_string(),
            Some("HS") => "USB_OTG_HS".to_string(),
            _ => "USB".to_string(),
        };
        let role = &caps[2];
        let entry = acc.usb.entry(family_key).or_default();
        let gpio = Gpio::new(label.to_string(), pin_id.to_string(), true);
        match role {
            "DP"   => entry.dp = Some(gpio),
            "DM"   => entry.dm = Some(gpio),
            "VBUS" => entry.vbus = Some(gpio),
            "ID"   => entry.id_pin = Some(gpio),
            _ => unreachable!("regex restricts role to DP|DM|VBUS|ID"),
        }
        return Ok(());
    }

    // ADC channel: one channel = one Adc instance.
    if let Some(caps) = adc_signal_regex().captures(signal) {
        let periph_num: u32 = caps[1].parse().map_err(|_| {
            format!("ADC peripheral number out of range in '{}'", signal)
        })?;
        let channel: u8 = caps[2].parse().map_err(|_| {
            format!("ADC channel number out of range in '{}'", signal)
        })?;
        let id = format!("hadc{}", periph_num);
        let pin = Gpio::new(label.to_string(), pin_id.to_string(), false);
        let adc = Adc::new(pin, channel, label.to_string(), id.clone());
        adc.is_valid()
            .map_err(|e| format!("{}: {}", signal, e))?;
        acc.adcs.push(adc);
        log.enabled_device("ADC", format!("{} channel {} on {}", pin_id, channel, id),
                           format!("signal `{}`", signal));
        return Ok(());
    }

    // ADCx_INn: CubeMX placeholder used on devices where multiple ADC
    // peripherals share channel inputs.  Resolve the instance by
    // cross-referencing the ADC peripheral configuration entries.
    if let Some(caps) = adcx_signal_regex().captures(signal) {
        let channel: u8 = caps[1].parse().map_err(|_| {
            format!("ADC channel number out of range in '{}'", signal)
        })?;
        if let Some(&periph_num) = adc_instance_map.get(&channel) {
            let id = format!("hadc{}", periph_num);
            let pin = Gpio::new(label.to_string(), pin_id.to_string(), false);
            let adc = Adc::new(pin, channel, label.to_string(), id.clone());
            adc.is_valid()
                .map_err(|e| format!("{}: {}", signal, e))?;
            acc.adcs.push(adc);
            log.enabled_device("ADC", format!("{} channel {} on {}", pin_id, channel, id),
                               format!("signal `{}` resolved to {} via ADC config", signal, id));
        } else {
            log.ignored(pin_id, format!(
                "signal `{}` matched ADCx_INn but no ADC instance owns channel {}",
                signal, channel,
            ));
        }
        return Ok(());
    }

    // DAC channel: one channel = one Dac instance. Peripheral number is
    // optional in the signal (`DAC_OUT1` vs `DAC1_OUT1`); default to 1.
    if let Some(caps) = dac_signal_regex().captures(signal) {
        let periph_num: u32 = caps
            .get(1)
            .map(|m| m.as_str().parse().unwrap_or(1))
            .unwrap_or(1);
        let channel: u8 = caps[2].parse().map_err(|_| {
            format!("DAC channel number out of range in '{}'", signal)
        })?;
        let id = format!("hdac{}", periph_num);
        let pin = Gpio::new(label.to_string(), pin_id.to_string(), true);
        let dac = Dac::new(pin, channel, label.to_string(), id.clone());
        dac.is_valid()
            .map_err(|e| format!("{}: {}", signal, e))?;
        acc.dacs.push(dac);
        log.enabled_device("DAC", format!("{} channel {} on {}", pin_id, channel, id),
                           format!("signal `{}`", signal));
        return Ok(());
    }

    // PWM channel from a timer. Complementary outputs (`*_CHnN`) are
    // intentionally skipped: the regex does not match them.
    if let Some(caps) = pwm_signal_regex().captures(signal) {
        let periph_num: u32 = caps[1].parse().map_err(|_| {
            format!("Timer peripheral number out of range in '{}'", signal)
        })?;
        let channel: u8 = caps[2].parse().map_err(|_| {
            format!("Timer channel number out of range in '{}'", signal)
        })?;
        let id = format!("htim{}", periph_num);
        let pin = Gpio::new(label.to_string(), pin_id.to_string(), true);
        let pwm = Pwm::new(pin, channel, label.to_string(), id.clone());
        pwm.is_valid()
            .map_err(|e| format!("{}: {}", signal, e))?;
        acc.pwms.push(pwm);
        log.enabled_device("PWM", format!("{} channel {} on {}", pin_id, channel, id),
                           format!("signal `{}`", signal));
        return Ok(());
    }

    // Anything else (timer non-PWM signals, sub-GHz, ETH, etc.) is ignored.
    // Record device-relevant signals so the absence of a peripheral is
    // explained; suppress obvious infrastructure signals (clocks, debug,
    // power) to keep the report readable.
    if !is_infrastructure_signal(signal) {
        log.ignored(pin_id, format!("unrecognized signal `{}`", signal));
    }
    Ok(())
}


/// Whether a pin `Signal` is clock/debug/power/system infrastructure rather
/// than a candidate peripheral. Such signals are never turned into devices and
/// are noisy in the generation report, so they are excluded from the
/// "ignored" list.
fn is_infrastructure_signal(signal: &str) -> bool {
    const PREFIXES: &[&str] = &["RCC", "SYS", "PWR", "DEBUG"];
    const CONTAINS: &[&str] = &["JTMS", "JTCK", "JTDI", "JTDO", "SWO", "SWCLK", "SWDIO", "OSC"];
    PREFIXES.iter().any(|p| signal.starts_with(p))
        || CONTAINS.iter().any(|c| signal.contains(c))
}


/// Materialise [`BusSpi`] instances from accumulated SPI pins.
fn build_spi_buses(
    acc: HashMap<String, SpiPins>,
    log: &mut GenLog,
) -> Result<Vec<BusSpi>, String> {
    let mut out = Vec::with_capacity(acc.len());
    for (periph, pins) in acc {
        let mut detail = vec![format!("SCK={}", opt_pin(&pins.sck))];
        detail.push(format!("MISO={}", opt_pin(&pins.miso)));
        detail.push(format!("MOSI={}", opt_pin(&pins.mosi)));
        detail.push(format!("NSS={}", opt_pin(&pins.nss)));
        let clk = pins
            .sck
            .ok_or_else(|| format!("{}: missing SCK", periph))?;
        if pins.nss.is_none() {
            log.ignored(format!("{} NSS", periph), "no NSS pin assigned (software CS)");
        }
        let cs = match pins.nss {
            Some(p) => vec![p],
            None => Vec::new(),
        };
        let id = format!("h{}", periph.to_lowercase());
        let bus = BusSpi::new(cs, pins.miso, pins.mosi, clk, periph.clone(), id);
        bus.is_valid()
            .map_err(|e| format!("{}: {}", periph, e))?;
        log.enabled_device("SPI", periph.clone(), detail.join(", "));
        out.push(bus);
    }
    Ok(out)
}


/// Render an optional pin as its id, or `-` when absent, for log detail lines.
fn opt_pin(p: &Option<Gpio>) -> String {
    p.as_ref().map(|g| g.id().to_string()).unwrap_or_else(|| "-".to_string())
}


/// Materialise [`BusUart`] instances from accumulated UART pins.
///
/// `baud` carries any `<PERIPH>.BaudRate` values extracted earlier by
/// [`collect_uart_baud_rates`]; peripherals without an entry are constructed
/// with `baud_rate = None`.
fn build_uart_buses(
    acc: HashMap<String, UartPins>,
    baud: &HashMap<String, u32>,
    log: &mut GenLog,
) -> Result<Vec<BusUart>, String> {
    let mut out = Vec::with_capacity(acc.len());
    for (periph, pins) in acc {
        let id = uart_handle_id(&periph);
        let baud_rate = baud.get(&periph).copied();
        let detail = format!(
            "TX={}, RX={}, baud={}, handle {}",
            opt_pin(&pins.tx), opt_pin(&pins.rx),
            baud_rate.map(|b| b.to_string()).unwrap_or_else(|| "(unset)".to_string()),
            id,
        );
        let bus = BusUart::new(pins.tx, pins.rx, periph.clone(), id, baud_rate);
        bus.is_valid()
            .map_err(|e| format!("{}: {}", periph, e))?;
        log.enabled_device("UART", periph.clone(), detail);
        out.push(bus);
    }
    Ok(out)
}


/// Collect every `<USART|UART|LPUART>n.BaudRate` entry from the tokenized
/// `.ioc` map into a peripheral-name → baud table.
///
/// A non-integer baud value is reported as a parse error referencing the
/// offending key (`"USART3.BaudRate: invalid integer 'foo'"`). Peripherals
/// without a `BaudRate` line simply do not appear in the returned map.
fn collect_uart_baud_rates(
    entries: &HashMap<String, String>,
) -> Result<HashMap<String, u32>, String> {
    let re = uart_baud_rate_regex();
    let mut out = HashMap::new();
    for (key, value) in entries {
        if let Some(caps) = re.captures(key) {
            let family = &caps[1];
            let num = &caps[2];
            let periph = format!("{}{}", family, num);
            let parsed: u32 = value.parse().map_err(|_| {
                format!("{}: invalid integer '{}'", key, value)
            })?;
            out.insert(periph, parsed);
        }
    }
    Ok(out)
}


/// Materialise [`BusI2c`] instances from accumulated I2C pins. Both SDA and
/// SCL are required; missing pins are reported with the peripheral name.
fn build_i2c_buses(
    acc: HashMap<String, I2cPins>,
    log: &mut GenLog,
) -> Result<Vec<BusI2c>, String> {
    let mut out = Vec::with_capacity(acc.len());
    for (periph, pins) in acc {
        let detail = format!("SDA={}, SCL={}", opt_pin(&pins.sda), opt_pin(&pins.scl));
        let sda = pins.sda.ok_or_else(|| format!("{}: missing SDA", periph))?;
        let scl = pins.scl.ok_or_else(|| format!("{}: missing SCL", periph))?;
        let id = format!("h{}", periph.to_lowercase());
        let bus = BusI2c::new(sda, scl, periph.clone(), id);
        bus.is_valid()
            .map_err(|e| format!("{}: {}", periph, e))?;
        log.enabled_device("I2C", periph.clone(), detail);
        out.push(bus);
    }
    Ok(out)
}


/// Materialise [`BusCan`] instances from accumulated CAN pins. Both TX and
/// RX are required; the HAL handle id uses `hcan<n>` for legacy CAN and
/// `hfdcan<n>` for FDCAN.
fn build_can_buses(
    acc: HashMap<String, CanPins>,
    log: &mut GenLog,
) -> Result<Vec<BusCan>, String> {
    let mut out = Vec::with_capacity(acc.len());
    for (periph, pins) in acc {
        let detail = format!(
            "{} family, TX={}, RX={}",
            if pins.family.is_empty() { "CAN" } else { &pins.family },
            opt_pin(&pins.tx), opt_pin(&pins.rx),
        );
        let tx = pins.tx.ok_or_else(|| format!("{}: missing TX", periph))?;
        let rx = pins.rx.ok_or_else(|| format!("{}: missing RX", periph))?;
        let id = format!("h{}", periph.to_lowercase());
        let bus = BusCan::new(tx, rx, periph.clone(), id);
        bus.is_valid()
            .map_err(|e| format!("{}: {}", periph, e))?;
        log.enabled_device("CAN", periph.clone(), detail);
        out.push(bus);
    }
    Ok(out)
}


/// Materialise [`BusUsb`] instances from accumulated USB pins. DP and DM
/// are required; VBUS and ID flow through as optional.
///
/// `modes` carries the host-vs-device determination from
/// [`collect_usb_device_modes`]; a peripheral missing from the map defaults
/// to device-mode.
fn build_usb_buses(
    acc: HashMap<String, UsbPins>,
    modes: &HashMap<String, bool>,
    log: &mut GenLog,
) -> Result<Vec<BusUsb>, String> {
    let mut out = Vec::with_capacity(acc.len());
    for (periph, pins) in acc {
        let cdc_device = modes.get(&periph).copied().unwrap_or(true);
        let detail = format!(
            "DP={}, DM={}, VBUS={}, ID={}, mode={}",
            opt_pin(&pins.dp), opt_pin(&pins.dm), opt_pin(&pins.vbus), opt_pin(&pins.id_pin),
            if cdc_device { "device (CDC-eligible)" } else { "host" },
        );
        let dp = pins.dp.ok_or_else(|| format!("{}: missing DP", periph))?;
        let dm = pins.dm.ok_or_else(|| format!("{}: missing DM", periph))?;
        let id = format!("h{}", periph.to_lowercase());
        let bus = BusUsb::new(dp, dm, pins.vbus, pins.id_pin, periph.clone(), id)
            .with_cdc_device(cdc_device);
        bus.is_valid()
            .map_err(|e| format!("{}: {}", periph, e))?;
        log.enabled_device("USB", periph.clone(), detail);
        out.push(bus);
    }
    Ok(out)
}


/// Map a UART peripheral key to its STM32 HAL handle name.
///
/// The HAL handle uses `huart<n>` for both `USART<n>` and `UART<n>`, and
/// `hlpuart<n>` for `LPUART<n>`. `USART` is normalised to `uart` (drop the
/// leading `s`) so that `USART1` → `huart1`.
fn uart_handle_id(periph: &str) -> String {
    let lower = periph.to_lowercase();
    let core = if let Some(rest) = lower.strip_prefix("usart") {
        format!("uart{}", rest)
    } else {
        lower
    };
    format!("h{}", core)
}


fn pin_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // CubeMX appends a function suffix to dual-function pins, using either a
    // dash (`PA0-WKUP`, `PC14-OSC32_IN`, `PH0-OSC_IN`) or a slash
    // (`PA0/WKUP`) as the separator depending on the CubeMX version. Capture
    // the canonical `P<bank><pin>` and tolerate any trailing `-…`/`/…`
    // decoration before `.Signal`.
    RE.get_or_init(|| Regex::new(r"^(P[A-K]\d{1,2})(?:[-/][A-Z0-9_]+)?\.Signal$").unwrap())
}

fn spi_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^SPI(\d+)_(NSS|SCK|MISO|MOSI)$").unwrap())
}

fn uart_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(USART|UART|LPUART)(\d+)_(TX|RX)$").unwrap())
}

fn uart_baud_rate_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Matches CubeMX entries such as `USART3.BaudRate`, `UART4.BaudRate`,
    // and `LPUART1.BaudRate`. Captures the family and peripheral index so
    // the caller can rebuild the canonical peripheral name.
    RE.get_or_init(|| Regex::new(r"^(USART|UART|LPUART)(\d+)\.BaudRate$").unwrap())
}

fn i2c_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^I2C(\d+)_(SDA|SCL)$").unwrap())
}

fn can_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(FDCAN|CAN)(\d+)_(TX|RX)$").unwrap())
}

fn usb_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^USB(?:_OTG_(FS|HS))?_(DP|DM|VBUS|ID)$").unwrap())
}

fn adc_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ADC(\d+)_(?:IN|INP|INN)(\d+)$").unwrap())
}

fn adcx_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ADCx_(?:IN|INP|INN)(\d+)$").unwrap())
}

fn adc_channel_config_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ADC(\d+)\.Channel-\d+\\#ChannelRegularConversion$").unwrap())
}

fn adc_channel_value_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ADC_CHANNEL_(\d+)$").unwrap())
}

/// Build a map from ADC input number to ADC peripheral instance number by
/// scanning entries of the form:
///   key:   `ADC1.Channel-0\#ChannelRegularConversion`
///   value: `ADC_CHANNEL_1`
///
/// `ADC_CHANNEL_N` corresponds directly to input `INn`, so the resulting map
/// lets `classify_pin` resolve `ADCx_INn` signals to the correct instance.
fn collect_adc_instance_map(entries: &HashMap<String, String>) -> HashMap<u8, u32> {
    let mut map = HashMap::new();
    for (key, value) in entries {
        if let Some(key_caps) = adc_channel_config_regex().captures(key)
            && let Some(val_caps) = adc_channel_value_regex().captures(value) {
                let periph_num: u32 = match key_caps[1].parse() {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let channel: u8 = match val_caps[1].parse() {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                map.insert(channel, periph_num);
            }
    }
    map
}

fn dac_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^DAC(\d+)?_OUT(\d+)$").unwrap())
}

fn pwm_signal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Note: `_CH<n>` only; `_CH<n>N` complementary outputs are NOT matched.
    RE.get_or_init(|| Regex::new(r"^TIM(\d+)_CH(\d+)$").unwrap())
}
