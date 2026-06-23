// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// UART bus definition

use super::Gpio;

#[cfg(test)]
mod tests;


pub struct BusUart {
    tx: Option<Gpio>, // The TX pin if transmit supported
    rx: Option<Gpio>, // The RX pin if receive supported
    name: String, // Programming name like "DEBUG_UART"
    id: String, // Hardware identifier like "huart1"
    baud_rate: Option<u32>, // Baud rate read from `<PERIPH>.BaudRate`, if present
}


impl BusUart {
    pub fn new(
        tx: Option<Gpio>,
        rx: Option<Gpio>,
        name: String,
        id: String,
        baud_rate: Option<u32>,
    ) -> Self {
        Self { tx, rx, name, id, baud_rate }
    }

    /// Get TX pin if defined
    pub fn tx(&self) -> Option<Gpio> {
        self.tx.clone()
    }

    /// Get RX pin if defined
    pub fn rx(&self) -> Option<Gpio> {
        self.rx.clone()
    }

    /// Get programming name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get bus ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Configured baud rate from the `.ioc`, or `None` if the
    /// `<PERIPH>.BaudRate` key was absent.
    pub fn baud_rate(&self) -> Option<u32> {
        self.baud_rate
    }

    /// Sanity check this definition
    pub fn is_valid(&self) -> Result<(), String> {
        if self.tx.is_none() && self.rx.is_none() {
            Err("Must have tx or rx defined for UART".to_string())
        }
        else if self.tx.as_ref().is_some_and(|p| p.is_valid().is_err()) {
            Err("Must have valid tx pin for UART".to_string())
        }
        else if self.rx.as_ref().is_some_and(|p| p.is_valid().is_err()) {
            Err("Must have valid rx pin for UART".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for UART bus".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for UART bus".to_string())
        }
        else {
            Ok(())
        }
    }

}
