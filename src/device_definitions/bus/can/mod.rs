// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// CAN bus definition

use super::Gpio;

#[cfg(test)]
mod tests;


pub struct BusCan {
    tx: Gpio, // The TX pin (CAN nodes must transmit for arbitration/ACK)
    rx: Gpio, // The RX pin (CAN nodes must receive for arbitration/ACK)
    name: String, // Programming name like "MOTOR_CAN_BUS"
    id: String, // Hardware identifier like "hcan1" or "hfdcan1"
}


#[allow(dead_code)]
impl BusCan {
    pub fn new(tx: Gpio, rx: Gpio, name: String, id: String) -> Self {
        Self { tx, rx, name, id }
    }

    /// Get the TX pin
    pub fn tx(&self) -> &Gpio {
        &self.tx
    }

    /// Get the RX pin
    pub fn rx(&self) -> &Gpio {
        &self.rx
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
        if self.tx.is_valid().is_err() {
            Err("Must have valid tx pin for CAN".to_string())
        }
        else if self.rx.is_valid().is_err() {
            Err("Must have valid rx pin for CAN".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for CAN bus".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for CAN bus".to_string())
        }
        else {
            Ok(())
        }
    }
}
