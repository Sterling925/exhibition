// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// Definition of ARM core.

#[allow(dead_code)]
pub struct Core {
    model: String, // ARM model such as "STM32F767ZI"
}


#[allow(dead_code)]
impl Core {
    pub fn new(model: String) -> Self {
        Self { model }
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}