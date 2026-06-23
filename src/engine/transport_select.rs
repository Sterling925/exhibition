// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Interactive picker for the bus the RT will listen on.
//!
//! Two transport families are eligible:
//!   - **UART** — any UART bus from the parsed [`DeviceSet`] with both TX
//!     and RX configured.
//!   - **USB CDC** — any USB bus with D+/D- configured. The host sees the
//!     RT as a virtual COM port (`/dev/ttyACM*` on Linux, `COMx` on
//!     Windows) so the BC side needs no changes.
//!
//! SPI/CAN as a transport require slave-mode wiring that is out of scope.
//! The picker prints a numbered list of eligible buses (UARTs first, then
//! USBs, each block sorted by name for stability) and reads a 1-based
//! selection from `stdin`. The selected bus is *not* exposed as a
//! peripheral sub-address.

use std::io::{BufRead, Write};

use crate::ioc_parser::DeviceSet;


/// USB device-controller speed. Determines whether the RT emits
/// `CDC_Transmit_HS` / `CDC_Receive_HS` or the `_FS` variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbSpeed {
    /// Full-speed (12 Mbit/s). Used for plain `USB_*` peripherals and
    /// `USB_OTG_FS`.
    Fs,
    /// High-speed (480 Mbit/s). Used for `USB_OTG_HS` peripherals.
    Hs,
}

impl UsbSpeed {
    /// Suffix baked into CubeMX-generated function names
    /// (`CDC_Transmit_HS` vs `CDC_Transmit_FS`).
    pub fn suffix(self) -> &'static str {
        match self {
            UsbSpeed::Fs => "FS",
            UsbSpeed::Hs => "HS",
        }
    }
}


/// Caller's choice of physical bus to back the RT's [`IFrameTransport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportChoice {
    /// UART selected. `bus_name` matches [`BusUart::name`]; `hal_handle`
    /// is the C++ HAL handle identifier (e.g. `huart3`). `baud_rate`
    /// carries the value parsed from `<PERIPH>.BaudRate` in the `.ioc`,
    /// or `None` when the IOC did not specify one — codegen treats
    /// `None` as "unknown, fall back to a default and warn".
    Uart {
        bus_name: String,
        hal_handle: String,
        baud_rate: Option<u32>,
    },
    /// USB CDC virtual COM port selected. `bus_name` matches
    /// [`BusUsb::name`] (e.g. `USB_OTG_HS`); `speed` picks the matching
    /// `CDC_Transmit_*` / `CDC_Receive_*` middleware function family.
    UsbCdc { bus_name: String, speed: UsbSpeed },
}

impl TransportChoice {
    /// Programming name of the chosen bus (used to filter it out of the
    /// peripheral list during sub-address assignment).
    pub fn bus_name(&self) -> &str {
        match self {
            TransportChoice::Uart { bus_name, .. } => bus_name,
            TransportChoice::UsbCdc { bus_name, .. } => bus_name,
        }
    }

    /// HAL handle identifier (without leading `&`), if the transport uses
    /// one. UART has a HAL handle; USB CDC is driven through standalone
    /// `CDC_Transmit_*` / `CDC_Receive_*` functions and so returns `None`.
    pub fn hal_handle(&self) -> Option<&str> {
        match self {
            TransportChoice::Uart { hal_handle, .. } => Some(hal_handle),
            TransportChoice::UsbCdc { .. } => None,
        }
    }

    /// CubeMX-generated transmit function for this transport, if it has
    /// one. Used by codegen to emit the `extern "C"` declaration in
    /// `device_registry.cpp`.
    pub fn transmit_fn_name(&self) -> Option<String> {
        match self {
            TransportChoice::Uart { .. } => None,
            TransportChoice::UsbCdc { speed, .. } => {
                Some(format!("CDC_Transmit_{}", speed.suffix()))
            }
        }
    }

    /// Short string for telemetry / messages (e.g. `"uart"`, `"usb_cdc"`).
    pub fn kind(&self) -> &'static str {
        match self {
            TransportChoice::Uart { .. } => "uart",
            TransportChoice::UsbCdc { .. } => "usb_cdc",
        }
    }

    /// Baud rate read from the `.ioc` for this transport. Returns `Some`
    /// only for a UART variant whose `<PERIPH>.BaudRate` line was present;
    /// returns `None` for an unconfigured UART or for any USB CDC choice
    /// (USB CDC ignores baud).
    pub fn baud_rate(&self) -> Option<u32> {
        match self {
            TransportChoice::Uart { baud_rate, .. } => *baud_rate,
            TransportChoice::UsbCdc { .. } => None,
        }
    }
}


/// Internal representation of one menu entry built from the [`DeviceSet`].
/// Keeps the picker generic over the underlying bus type.
struct MenuEntry {
    /// Human-readable line shown in the menu (after the index).
    display: String,
    /// The `TransportChoice` returned if the user selects this entry.
    choice: TransportChoice,
}


/// Prompt the user to choose a transport bus. Reads a 1-based numeric
/// selection from `input`, writes the menu to `output`.
///
/// Returns `Err` if no eligible buses are present or if the input
/// cannot be parsed as a valid choice.
pub fn select_transport_with_io<R: BufRead, W: Write>(
    devices: &DeviceSet,
    mut input: R,
    mut output: W,
) -> Result<TransportChoice, String> {
    let mut entries: Vec<MenuEntry> = Vec::new();

    // UART entries: TX + RX required, sorted by name.
    let mut uarts: Vec<&crate::device_definitions::bus::uart::BusUart> = devices
        .uart_buses
        .iter()
        .filter(|b| b.tx().is_some() && b.rx().is_some())
        .collect();
    uarts.sort_by(|a, b| a.name().cmp(b.name()));
    for bus in uarts {
        entries.push(MenuEntry {
            display: format!("{} ({}, UART)", bus.name(), bus.id()),
            choice: TransportChoice::Uart {
                bus_name: bus.name().to_string(),
                hal_handle: bus.id().to_string(),
                baud_rate: bus.baud_rate(),
            },
        });
    }

    // USB entries: DP + DM required (enforced by BusUsb::is_valid) AND the
    // block must be configured as a USB *device* (CDC virtual COM). A
    // host-mode USB (e.g. `USB_OTG_FS.VirtualMode=Host_Only`) cannot serve as
    // the RT's transport, so it is excluded here rather than offered and
    // mis-selected. Sorted by name; speed derived from the bus name emitted by
    // the parser (`USB_OTG_HS` → HS, otherwise FS).
    let mut usbs: Vec<&crate::device_definitions::bus::usb::BusUsb> = devices
        .usb_buses
        .iter()
        .filter(|b| b.is_valid().is_ok() && b.is_cdc_device())
        .collect();
    usbs.sort_by(|a, b| a.name().cmp(b.name()));
    for bus in usbs {
        let speed = if bus.name().contains("_HS") { UsbSpeed::Hs } else { UsbSpeed::Fs };
        entries.push(MenuEntry {
            display: format!("{} ({}, USB CDC {})", bus.name(), bus.id(), speed.suffix()),
            choice: TransportChoice::UsbCdc {
                bus_name: bus.name().to_string(),
                speed,
            },
        });
    }

    if entries.is_empty() {
        return Err(
            "No eligible transport buses in the IOC file. Need either a UART \
             with both TX and RX, or a USB peripheral with D+ and D-. SPI/CAN \
             transports are not supported."
                .to_string(),
        );
    }

    writeln!(output, "Select the bus the RT should listen on:")
        .map_err(|e| format!("Failed to write prompt: {}", e))?;
    for (idx, entry) in entries.iter().enumerate() {
        writeln!(output, "  {}. {}", idx + 1, entry.display)
            .map_err(|e| format!("Failed to write prompt: {}", e))?;
    }
    write!(output, "Enter choice [1-{}]: ", entries.len())
        .map_err(|e| format!("Failed to write prompt: {}", e))?;
    output.flush().ok();

    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read selection: {}", e))?;
    let trimmed = line.trim();
    let idx: usize = trimmed
        .parse::<usize>()
        .map_err(|_| format!("Invalid selection '{}'", trimmed))?;
    if idx < 1 || idx > entries.len() {
        return Err(format!(
            "Selection {} out of range [1, {}]",
            idx,
            entries.len()
        ));
    }
    Ok(entries.into_iter().nth(idx - 1).unwrap().choice)
}


/// Convenience wrapper around [`select_transport_with_io`] that reads from
/// `stdin` and writes to `stdout`. Intended for the binary entry point.
pub fn select_transport(devices: &DeviceSet) -> Result<TransportChoice, String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let locked = stdin.lock();
    select_transport_with_io(devices, locked, stdout.lock())
}
