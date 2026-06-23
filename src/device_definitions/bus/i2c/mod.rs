// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// I2C bus definition

use super::Gpio;

#[cfg(test)]
mod tests;


pub struct BusI2c {
    sda: Gpio, // The serial data pin
    scl: Gpio, // The serial clock pin
    name: String, // Programming name like "SENSOR_I2C"
    id: String, // Hardware identifier like "hi2c1"
}


#[allow(dead_code)]
impl BusI2c {
    pub fn new(sda: Gpio, scl: Gpio, name: String, id: String) -> Self {
        Self { sda, scl, name, id }
    }

    /// Get the SDA pin
    pub fn sda(&self) -> &Gpio {
        &self.sda
    }

    /// Get the SCL pin
    pub fn scl(&self) -> &Gpio {
        &self.scl
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
        if self.sda.is_valid().is_err() {
            Err("Must have valid sda pin for I2C".to_string())
        }
        else if self.scl.is_valid().is_err() {
            Err("Must have valid scl pin for I2C".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for I2C bus".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for I2C bus".to_string())
        }
        else {
            Ok(())
        }
    }
}
