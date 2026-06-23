// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// Definition of an on chip PWM channel

use super::gpio::Gpio;

#[cfg(test)]
mod tests;


pub struct Pwm {
    pin: Gpio, // The output pin driven by this PWM channel
    channel: u8, // Channel number on the timer peripheral (e.g. 1 for TIM2_CH1)
    name: String, // Programming name like "MOTOR_PWM"
    id: String, // Hardware identifier like "htim2"
}


impl Pwm {
    pub fn new(pin: Gpio, channel: u8, name: String, id: String) -> Self {
        Self { pin, channel, name, id }
    }

    /// Get the output pin
    pub fn pin(&self) -> &Gpio {
        &self.pin
    }

    /// Get channel number on the timer peripheral
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
            Err("Must have valid pin for PWM".to_string())
        }
        else if !self.pin.output() {
            Err("PWM pin must be configured as output".to_string())
        }
        else if self.name.is_empty() {
            Err("Must define name for PWM channel".to_string())
        }
        else if self.id.is_empty() {
            Err("Must define id for PWM channel".to_string())
        }
        else {
            Ok(())
        }
    }
}
