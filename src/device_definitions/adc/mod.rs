// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// Definition of on chip ADC channel

use super::gpio::Gpio;

#[cfg(test)]
mod tests;


pub struct Adc {
    pin: Gpio, // The input pin sampled by this ADC channel
    channel: u8, // Channel number on the ADC peripheral (e.g. 3 for ADC1_IN3)
    name: String, // Programming name like "BATTERY_VOLTAGE"
    id: String, // Hardware identifier like "hadc1"
}


impl Adc {
    pub fn new(pin: Gpio, channel: u8, name: String, id: String) -> Self {
        Self { pin, channel, name, id }
    }

    /// Get the input pin
    pub fn pin(&self) -> &Gpio {
        &self.pin
    }

    /// Get channel number on the ADC peripheral
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
            Err("Must have valid pin for ADC".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for ADC channel".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for ADC channel".to_string())
        }
        else {
            Ok(())
        }
    }
}
