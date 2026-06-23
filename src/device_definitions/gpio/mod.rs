// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

// GPIO definition container

#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq)]
pub struct Gpio {
    name: String, // Code name from IOC file like "GREEN_LED_PIN"
	id: String, // Identifier of pin like `PA1` or `PB13`
    output: bool, // Is pin an input or an output pin
}


impl std::fmt::Display for Gpio {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		if self.output{
        	write!(f, "GPIO output, {}, {}, ", self.id, self.name)
		}
		else {
        	write!(f, "GPIO input, {}, {}, ", self.id, self.name)
		}
    }
}


impl Gpio {
	pub fn new(name: String, id: String, output: bool) -> Self {
		Self{
			name, id, output
		}
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn output(&self) -> bool {
		self.output
	}

    /// Get bank name if valid definition
	///
	/// Extracts the bank character from the pin identifier.
	/// For example `"PA1"` returns `Some('A')` and `"PB13"` returns `Some('B')`.
	/// Returns `None` if the identifier is malformed (does not start with `'P'`
	/// followed by an ASCII alphabetic character).
	pub fn bank(&self) -> Option<char> {
		let mut chars = self.id.chars();
		if chars.next()? != 'P' {
			return None;
		}
		let bank = chars.next()?;
		if !bank.is_ascii_alphabetic() {
			return None;
		}
		Some(bank.to_ascii_uppercase())
	}

	/// Get pin number if valid definition
	///
	/// Extracts the numeric pin component from the pin identifier.
	/// For example `"PA1"` returns `Some(1)` and `"PB13"` returns `Some(13)`.
	/// Returns `None` if the identifier is malformed or the trailing portion
	/// cannot be parsed as a `u16`.
	pub fn pin(&self) -> Option<u16> {
		let rest = self.id.strip_prefix('P')?;
		let mut iter = rest.chars();
		let _bank = iter.next()?;
		let digits = iter.as_str();
		if digits.is_empty() {
			return None;
		}
		digits.parse::<u16>().ok()
	}

	/// Check if definition is valid
	///
	/// Returns `Ok(())` when the identifier parses as `P<bank><pin>` with the
	/// bank in `'A'..='K'` and the pin in `0..=15`. Otherwise returns `Err`
	/// describing why the definition is rejected.
	pub fn is_valid(&self) -> Result<(), String> {
		let bank = self
			.bank()
			.ok_or_else(|| format!("Invalid pin identifier '{}': cannot extract bank", self.id))?;
		let pin = self
			.pin()
			.ok_or_else(|| format!("Invalid pin identifier '{}': cannot extract pin number", self.id))?;

		if !('A'..='K').contains(&bank) {
			return Err(format!(
				"Bank '{}' is out of range ['A', 'K'] in identifier '{}'",
				bank, self.id
			));
		}

		if pin > 15 {
			return Err(format!(
				"Pin {} is out of range [0, 15] in identifier '{}'",
				pin, self.id
			));
		}

		Ok(())
	}


}










