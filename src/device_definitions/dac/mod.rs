// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// Definition of on chip DAC channel

use super::gpio::Gpio;

#[cfg(test)]
mod tests;


pub struct Dac {
    pin: Gpio, // The analog output pin driven by this DAC channel
    channel: u8, // Channel number on the DAC peripheral (e.g. 1 for DAC1_OUT1)
    name: String, // Programming name like "REF_VOLTAGE_OUT"
    id: String, // Hardware identifier like "hdac1"
}


impl Dac {
    pub fn new(pin: Gpio, channel: u8, name: String, id: String) -> Self {
        Self { pin, channel, name, id }
    }

    /// Get the output pin
    pub fn pin(&self) -> &Gpio {
        &self.pin
    }

    /// Get channel number on the DAC peripheral
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Get programming name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get hardware identifier
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Sanity check this definition
    pub fn is_valid(&self) -> Result<(), String> {
        if self.pin.is_valid().is_err() {
            Err("Must have valid pin for DAC".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for DAC channel".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for DAC channel".to_string())
        }
        else {
            Ok(())
        }
    }
}
