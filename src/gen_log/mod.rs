// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Generation log for the RT code generator.
//!
//! [`GenLog`] accumulates a human-readable record of what the generator did
//! and why, so the produced RT project carries its own traceability. It is
//! threaded (dependency-injection style) through the parse phase
//! ([`crate::ioc_parser`]) and the generate phase ([`crate::engine`]) and is
//! finally rendered to Markdown and written into the generated project at
//! `Core/Src/rt/api/generation_report.md`.
//!
//! The report captures four things:
//!
//! 1. **Source facts** — the `.ioc` path, the target MCU, and the chosen
//!    transport.
//! 2. **Devices enabled (and why)** — every peripheral the parser turned on,
//!    with the `.ioc` signal that triggered it.
//! 3. **Signals ignored / buses dropped** — pin signals that did not map to a
//!    device, so the absence of a peripheral is explained rather than silent.
//! 4. **Code-generation decisions** — the transport branch, the device
//!    handlers emitted, and the final API fingerprint.
//!
//! A [`disabled`](GenLog::disabled) logger drops every message and writes
//! nothing; the non-logging public entry points in the parser and engine pass
//! one so existing callers and tests are unaffected.
//!
//! The report deliberately omits a wall-clock timestamp so the generated
//! output stays reproducible; it stamps the deterministic crate version
//! instead.

use std::path::Path;


/// Accumulates the generation report. Construct with [`GenLog::new`] for a
/// live logger, or [`GenLog::disabled`] for a no-op logger whose methods all
/// return immediately and whose [`write_to`](GenLog::write_to) does nothing.
pub struct GenLog {
    enabled: bool,
    source: Vec<String>,
    enabled_devices: Vec<String>,
    ignored: Vec<String>,
    codegen: Vec<String>,
    fingerprint: Option<u32>,
}

impl GenLog {
    /// Create a live logger that records every message.
    pub fn new() -> Self {
        GenLog {
            enabled: true,
            source: Vec::new(),
            enabled_devices: Vec::new(),
            ignored: Vec::new(),
            codegen: Vec::new(),
            fingerprint: None,
        }
    }

    /// Create a no-op logger. Every recording method returns without storing
    /// anything and [`write_to`](GenLog::write_to) is a no-op, so non-logging
    /// callers pay nothing and produce no report file.
    // Used by the non-logging public wrappers (parse_str, build_artifacts, …),
    // which are themselves only reached from the test suite in this binary.
    #[allow(dead_code)]
    pub fn disabled() -> Self {
        GenLog {
            enabled: false,
            source: Vec::new(),
            enabled_devices: Vec::new(),
            ignored: Vec::new(),
            codegen: Vec::new(),
            fingerprint: None,
        }
    }

    /// Record a source fact (e.g. the `.ioc` path, the MCU, the transport)
    /// for the report's **Source** section.
    pub fn source(&mut self, line: impl Into<String>) {
        if self.enabled {
            self.source.push(line.into());
        }
    }

    /// Record an enabled peripheral with the reason it was turned on.
    ///
    /// `category` is the device family (e.g. `"GPIO"`, `"SPI"`, `"ADC"`),
    /// `name` identifies the instance (e.g. `"USART2"`, `"PA5"`), and `reason`
    /// names the `.ioc` signal or pin set that triggered it.
    pub fn enabled_device(
        &mut self,
        category: &str,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) {
        if self.enabled {
            self.enabled_devices.push(format!(
                "**{}** {} — {}",
                category,
                name.into(),
                reason.into(),
            ));
        }
    }

    /// Record something that did *not* become a device (an unrecognized pin
    /// signal, a peripheral that was absent) and why.
    pub fn ignored(&mut self, what: impl Into<String>, reason: impl Into<String>) {
        if self.enabled {
            self.ignored.push(format!("{} — {}", what.into(), reason.into()));
        }
    }

    /// Record a code-generation decision (transport branch, handler emitted,
    /// glue file written) for the report's **Code-generation decisions**
    /// section.
    pub fn decision(&mut self, line: impl Into<String>) {
        if self.enabled {
            self.codegen.push(line.into());
        }
    }

    /// Record the final API fingerprint value.
    pub fn set_fingerprint(&mut self, fingerprint: u32) {
        if self.enabled {
            self.fingerprint = Some(fingerprint);
        }
    }

    /// Render the accumulated record as a Markdown document.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(crate::engine::SPDX_MARKDOWN_HEADER);
        out.push_str("# Exhibition RT Generation Report\n\n");
        out.push_str(&format!(
            "Generated by exhibition v{}.\n",
            env!("CARGO_PKG_VERSION"),
        ));

        render_section(&mut out, "Source", &self.source);
        render_section(&mut out, "Devices enabled (and why)", &self.enabled_devices);
        render_section(&mut out, "Signals ignored / buses dropped", &self.ignored);
        render_section(&mut out, "Code-generation decisions", &self.codegen);

        out.push_str("\n## API fingerprint\n\n");
        match self.fingerprint {
            Some(fp) => out.push_str(&format!("`0x{:08x}`\n", fp)),
            None => out.push_str("_(not computed)_\n"),
        }

        out
    }

    /// Write the rendered report to `path`. A [`disabled`](GenLog::disabled)
    /// logger writes nothing and returns `Ok(())`.
    pub fn write_to(&self, path: &Path) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        std::fs::write(path, self.render())
            .map_err(|e| format!("Failed to write generation report '{}': {}", path.display(), e))
    }
}

impl Default for GenLog {
    fn default() -> Self {
        GenLog::new()
    }
}


/// Append a `## <title>` section listing `items` as a Markdown bullet list,
/// or `_(none)_` when the section is empty.
fn render_section(out: &mut String, title: &str, items: &[String]) {
    out.push_str(&format!("\n## {}\n\n", title));
    if items.is_empty() {
        out.push_str("_(none)_\n");
        return;
    }
    for item in items {
        out.push_str(&format!("- {}\n", item));
    }
}


#[cfg(test)]
mod tests;
