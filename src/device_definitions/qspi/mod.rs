// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Definition for an on-board QUADSPI (Quad SPI) peripheral.
//!
//! STM32 MCUs expose at most one QUADSPI controller. When a CubeMX project
//! enables it the generated HAL handle is always `hqspi`
//! (`QSPI_HandleTypeDef`), so this struct only carries the handle identifier
//! required by the code-generation back-end.

#[cfg(test)]
mod tests;


/// A hardware QUADSPI peripheral extracted from a CubeMX `.ioc` file.
///
/// The handle id is always `"hqspi"` for STM32 targets; it is stored
/// explicitly so the code-generator can reference it by name when
/// instantiating the C++ handler.
#[derive(Clone, Debug)]
pub struct Qspi {
    id: String,
}

impl Qspi {
    /// Create a new `Qspi` with the given HAL handle identifier.
    pub fn new(id: String) -> Self {
        Qspi { id }
    }

    /// HAL handle identifier, e.g. `"hqspi"`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Validate that the instance is well-formed.
    ///
    /// Returns `Err` if the handle id is empty.
    pub fn is_valid(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("Qspi: handle id must not be empty".to_string());
        }
        Ok(())
    }
}
