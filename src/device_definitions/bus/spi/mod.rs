// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// SPI bus definition

use super::Gpio;

#[cfg(test)]
mod tests;


pub struct BusSpi {
    cs: Vec<Gpio>, // List of chip select pins. one per device on bus
    miso: Option<Gpio>, // The MISO pin if RX supported
    mosi: Option<Gpio>, // The MOSI pin if TX supported
    clk: Gpio, // The Clock pin
    name: String, // Programming name like "THERMAL_CYCLER_BUS"
    id: String, // Hardware identifier like "hspi1"
}


#[allow(dead_code)]
impl BusSpi {
    pub fn new(
        cs: Vec<Gpio>,
        miso: Option<Gpio>,
        mosi: Option<Gpio>,
        clk: Gpio,
        name: String,
        id: String, ) -> Self {
            Self { cs, miso, mosi, clk, name, id }
    }

    /// Get list of valid Chip Select pins
    pub fn cs(&self) -> &Vec<Gpio> {
        &self.cs
    }

    /// Get MISO pin if defined
    pub fn miso(&self) -> Option<Gpio> {
        self.miso.clone()
    }

    /// Get MOSI pin if defined
    pub fn mosi(&self) -> Option<Gpio> {
        self.mosi.clone()
    }

    /// Get Clock pin
    pub fn clk(&self) -> &Gpio {
        &self.clk
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
        if self.miso.is_none() && self.mosi.is_none() {
            Err("Must have miso or mosi defined for SPI".to_string())
        }
        else if self.clk.is_valid().is_err() {
            Err("Must have valid clock pin for SPI".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for SPI bus".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for SPI bus".to_string())
        }
        else {
            Ok(())
        }
    }

}


