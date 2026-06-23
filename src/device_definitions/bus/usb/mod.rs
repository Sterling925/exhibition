// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// USB bus definition

use super::Gpio;

#[cfg(test)]
mod tests;


pub struct BusUsb {
    dp: Gpio, // D+ data line (always required)
    dm: Gpio, // D- data line (always required)
    vbus: Option<Gpio>, // Optional VBUS sense pin (OTG)
    id_pin: Option<Gpio>, // Optional ID pin (OTG role detect)
    name: String, // Programming name like "HOST_USB"
    id: String, // Hardware identifier like "husb_otg_fs"
    cdc_device: bool, // True when configured as a USB *device* (CDC virtual COM)
}


#[allow(dead_code)]
impl BusUsb {
    pub fn new(
        dp: Gpio,
        dm: Gpio,
        vbus: Option<Gpio>,
        id_pin: Option<Gpio>,
        name: String,
        id: String, ) -> Self {
            // `cdc_device` defaults to `true`; the parser overrides it via
            // [`with_cdc_device`] once it has read the peripheral's
            // `VirtualMode`. Buses built directly (e.g. in tests) are treated
            // as device-mode unless told otherwise.
            Self { dp, dm, vbus, id_pin, name, id, cdc_device: true }
    }

    /// Set whether this USB block is configured as a USB **device** (a CDC
    /// virtual COM port the host enumerates) rather than a USB **host**.
    ///
    /// Only device-mode blocks can back the RT's USB-CDC transport, so the
    /// transport picker consults [`is_cdc_device`](Self::is_cdc_device) to
    /// decide eligibility. Returns `self` for builder-style chaining.
    pub fn with_cdc_device(mut self, cdc_device: bool) -> Self {
        self.cdc_device = cdc_device;
        self
    }

    /// Whether this USB block is configured as a USB device (CDC virtual COM
    /// eligible). `false` for host-only blocks, which cannot serve as the
    /// RT's transport.
    pub fn is_cdc_device(&self) -> bool {
        self.cdc_device
    }

    /// Get the D+ pin
    pub fn dp(&self) -> &Gpio {
        &self.dp
    }

    /// Get the D- pin
    pub fn dm(&self) -> &Gpio {
        &self.dm
    }

    /// Get the VBUS sense pin if defined
    pub fn vbus(&self) -> Option<Gpio> {
        self.vbus.clone()
    }

    /// Get the ID pin if defined
    pub fn id_pin(&self) -> Option<Gpio> {
        self.id_pin.clone()
    }

    /// Get programming name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get bus ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Sanity check this definition
    pub fn is_valid(&self) -> Result<(), String> {
        if self.dp.is_valid().is_err() {
            Err("Must have valid D+ pin for USB".to_string())
        }
        else if self.dm.is_valid().is_err() {
            Err("Must have valid D- pin for USB".to_string())
        }
        else if self.vbus.as_ref().is_some_and(|p| p.is_valid().is_err()) {
            Err("Must have valid VBUS pin for USB".to_string())
        }
        else if self.id_pin.as_ref().is_some_and(|p| p.is_valid().is_err()) {
            Err("Must have valid ID pin for USB".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for USB bus".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for USB bus".to_string())
        }
        else {
            Ok(())
        }
    }
}
