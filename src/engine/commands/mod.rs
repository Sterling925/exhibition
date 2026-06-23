// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Authoritative command catalog.
//!
//! The opcode vocabulary for every sub-address kind lives in the embedded
//! per-type YAML files under `catalog/` (one file per [`DeviceKind`] plus a
//! synthetic `system` entry). These files are the single source of truth that
//! drives:
//!
//! 1. the generated C++ command enums (`command_ids.h`),
//! 2. the per-sub-address YAML manifest (via [`crate::engine::manifest`]),
//! 3. the opcode names the BC client uses for logging.
//!
//! The catalog is embedded into the binary with [`include_str!`], parsed once
//! on first access via a [`LazyLock`], and validated at parse time (see
//! [`Catalog::parse_embedded`]). A malformed catalog panics on first use; the
//! [`tests`] module forces parsing early so a broken catalog fails the build.
//!
//! Editing a catalog file changes opcode numbering/naming but **not** handler
//! behavior — the C++ handler logic that actually services each opcode is
//! implemented separately (see the device handler templates). Keep the two in
//! sync.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::engine::addresses::{DeviceKind, DeviceSlot, SYSTEM_SUB_ADDRESS};

#[cfg(test)]
mod tests;


/// One command (opcode) supported by a sub-address kind.
#[derive(Debug, Clone, Deserialize)]
pub struct CommandDef {
    /// Wire/manifest name, `SCREAMING_SNAKE_CASE` (e.g. `GET_FINGERPRINT`).
    /// Becomes a PascalCase C++ enum member via [`to_pascal_case`].
    pub name: String,
    /// Opcode byte. Stored in YAML as a quoted hex string (e.g. `"0x01"`)
    /// and parsed here; YAML has no native `0x` integer literal.
    #[serde(deserialize_with = "de_hex_u8")]
    pub opcode: u8,
    /// Optional build-time default description emitted into the manifest.
    /// `None` (field omitted in the catalog) falls back to the `TODO`
    /// placeholder. Must be single-line and free of YAML-breaking syntax
    /// (validated in [`Catalog::parse_embedded`]).
    #[serde(default)]
    pub description: Option<String>,
    /// Structured shape of the command (request) payload.
    pub cmd_payload: PayloadDef,
    /// Structured shape of the response payload.
    pub resp_payload: PayloadDef,
}


/// Structured description of one wire payload: a size expression plus an
/// optional ordered field breakdown.
///
/// `bytes` is a free-form string rather than an integer so it can carry the
/// non-numeric size notations the catalog uses (`"6+"` for "six or more",
/// `"len"` / `"N"` for a length carried elsewhere in the frame, or an
/// annotated count like `"13 (padded to 14)"`). `fields` is empty when the
/// payload is a single opaque blob or has no documented sub-fields.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PayloadDef {
    /// Byte-count expression: `"0"`, `"12"`, `"6+"`, `"len"`, `"N"`, … .
    #[serde(default)]
    pub bytes: String,
    /// Ordered field breakdown; each entry is one `name: type` pair.
    #[serde(default)]
    pub fields: Vec<PayloadField>,
}


/// One field in a [`PayloadDef`]: a name and a C-style type.
///
/// In the catalog YAML this is written as a single-key mapping so the source
/// stays compact, e.g. `- uid0: uint32_t`. The custom [`Deserialize`] below
/// rejects an entry that does not have exactly one `name: type` pair.
#[derive(Debug, Clone)]
pub struct PayloadField {
    /// Field name (free-form; used verbatim in the manifest, not as an ident).
    pub name: String,
    /// C-style type token, e.g. `uint32_t`, `int32_t`, `uint8_t[]`.
    pub ty: String,
}

impl<'de> Deserialize<'de> for PayloadField {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> serde::de::Visitor<'de> for FieldVisitor {
            type Value = PayloadField;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a single-key mapping `name: type`")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let Some((name, ty)) = map.next_entry::<String, String>()? else {
                    return Err(serde::de::Error::custom(
                        "payload field must have exactly one `name: type` entry",
                    ));
                };
                if map.next_entry::<String, String>()?.is_some() {
                    return Err(serde::de::Error::custom(
                        "payload field must have exactly one `name: type` entry",
                    ));
                }
                Ok(PayloadField { name, ty })
            }
        }

        de.deserialize_map(FieldVisitor)
    }
}


/// The full command set for one sub-address kind, parsed from one catalog
/// YAML file.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCommandSet {
    /// Catalog key: a [`DeviceKind::sort_key`] value, or `system`.
    pub kind: String,
    /// C++ `enum class` name emitted for this set (e.g. `GpioOutputCmd`).
    pub cpp_enum: String,
    /// Commands in declaration order.
    pub commands: Vec<CommandDef>,
}


/// Parse a quoted hex (or decimal) string into a `u8` for serde.
fn de_hex_u8<'de, D>(de: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(de)?;
    let trimmed = raw.trim();
    let parsed = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => trimmed.parse::<u8>(),
    };
    parsed.map_err(|e| serde::de::Error::custom(format!("invalid opcode '{raw}': {e}")))
}


/// The parsed, validated command catalog.
///
/// `sets` preserves embedding order for deterministic emission; `index` maps a
/// catalog key (`sort_key` or `system`) to its position in `sets`.
pub struct Catalog {
    sets: Vec<DeviceCommandSet>,
    index: HashMap<String, usize>,
}

/// Embedded catalog sources, in the order they are emitted. `system` first,
/// then device kinds in [`DeviceKind::sort_key`] order.
const CATALOG_SOURCES: &[(&str, &str)] = &[
    ("system", include_str!("catalog/system.yaml")),
    ("adc", include_str!("catalog/adc.yaml")),
    ("can", include_str!("catalog/can.yaml")),
    ("dac", include_str!("catalog/dac.yaml")),
    ("gpio_in", include_str!("catalog/gpio_in.yaml")),
    ("gpio_out", include_str!("catalog/gpio_out.yaml")),
    ("i2c", include_str!("catalog/i2c.yaml")),
    ("pwm", include_str!("catalog/pwm.yaml")),
    ("qspi", include_str!("catalog/qspi.yaml")),
    ("rtc", include_str!("catalog/rtc.yaml")),
    ("spi", include_str!("catalog/spi.yaml")),
    ("uart", include_str!("catalog/uart.yaml")),
];

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog::parse_embedded().expect("embedded command catalog must be valid")
});

impl Catalog {
    /// Parse and validate every embedded catalog file.
    ///
    /// # Errors
    /// Returns `Err` with a human-readable message if any file fails to parse
    /// or violates a validation rule (see the body for the exact rules).
    pub fn parse_embedded() -> Result<Catalog, String> {
        let mut sets: Vec<DeviceCommandSet> = Vec::with_capacity(CATALOG_SOURCES.len());
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut seen_enums: HashSet<String> = HashSet::new();

        for (key, src) in CATALOG_SOURCES {
            let set: DeviceCommandSet = serde_yaml::from_str(src)
                .map_err(|e| format!("command catalog '{key}.yaml': {e}"))?;

            if set.kind != *key {
                return Err(format!(
                    "command catalog '{key}.yaml': kind '{}' does not match file '{key}'",
                    set.kind
                ));
            }
            if !is_cpp_ident(&set.cpp_enum) {
                return Err(format!(
                    "command catalog '{key}.yaml': cpp_enum '{}' is not a valid C++ identifier",
                    set.cpp_enum
                ));
            }
            if !seen_enums.insert(set.cpp_enum.clone()) {
                return Err(format!(
                    "command catalog '{key}.yaml': duplicate cpp_enum '{}'",
                    set.cpp_enum
                ));
            }
            if set.commands.is_empty() {
                return Err(format!("command catalog '{key}.yaml': no commands"));
            }

            let mut seen_opcodes: HashSet<u8> = HashSet::new();
            let mut seen_members: HashSet<String> = HashSet::new();
            for c in &set.commands {
                if !is_cpp_ident(&c.name) {
                    return Err(format!(
                        "command catalog '{key}.yaml': command name '{}' is not a valid identifier",
                        c.name
                    ));
                }
                if !seen_opcodes.insert(c.opcode) {
                    return Err(format!(
                        "command catalog '{key}.yaml': duplicate opcode 0x{:02X}",
                        c.opcode
                    ));
                }
                // Opcode 0x00 is reserved framework-wide for the universal
                // GET_SERVICE_HASH command, which the RemoteTerminal answers
                // before any handler runs. Bind the name and the opcode to each
                // other so no catalog can repurpose 0x00 or move GET_SERVICE_HASH.
                if c.opcode == 0x00 && c.name != "GET_SERVICE_HASH" {
                    return Err(format!(
                        "command catalog '{key}.yaml': opcode 0x00 is reserved for \
                         GET_SERVICE_HASH (got '{}')",
                        c.name
                    ));
                }
                if c.name == "GET_SERVICE_HASH" && c.opcode != 0x00 {
                    return Err(format!(
                        "command catalog '{key}.yaml': GET_SERVICE_HASH must use opcode \
                         0x00 (got 0x{:02X})",
                        c.opcode
                    ));
                }
                let member = to_pascal_case(&c.name);
                if !seen_members.insert(member.clone()) {
                    return Err(format!(
                        "command catalog '{key}.yaml': command '{}' collides with another \
                         after PascalCase conversion ('{member}')",
                        c.name
                    ));
                }
                if let Some(desc) = &c.description {
                    validate_description(desc).map_err(|why| {
                        format!(
                            "command catalog '{key}.yaml': command '{}' description {why}",
                            c.name
                        )
                    })?;
                }
            }

            index.insert((*key).to_string(), sets.len());
            sets.push(set);
        }

        Ok(Catalog { sets, index })
    }

    /// Look up a command set by catalog key (`sort_key` or `system`).
    fn get(&self, key: &str) -> Option<&DeviceCommandSet> {
        self.index.get(key).map(|&i| &self.sets[i])
    }
}


/// The command set for a device kind.
pub fn for_kind(kind: DeviceKind) -> &'static DeviceCommandSet {
    CATALOG
        .get(kind.sort_key())
        .unwrap_or_else(|| panic!("device kind '{}' missing from command catalog", kind.sort_key()))
}

/// The command set for the system handler (sub-address 1).
pub fn system() -> &'static DeviceCommandSet {
    CATALOG.get("system").expect("system missing from command catalog")
}

/// Every command set in deterministic emission order (system first).
pub fn all() -> impl Iterator<Item = &'static DeviceCommandSet> {
    CATALOG.sets.iter()
}


/// FNV-1a 32-bit hash over one command set's vocabulary — opcode + name + wire
/// payload shapes, with descriptions excluded.
///
/// This is the *per-service* hash: it depends only on the embedded catalog for
/// the set's kind, so every sub-address of the same peripheral kind shares it
/// and the value is project-independent. It reuses the manifest command
/// renderer and canonicalizer ([`crate::engine::manifest::push_commands`] /
/// [`crate::engine::manifest::canonicalize`]) so it is stable against exactly
/// the same edits the global fingerprint ignores (descriptions, whitespace).
///
/// Only the rendered command list is hashed — not `kind`/`cpp_enum` — so the
/// value is the command vocabulary alone. The universal `GET_SERVICE_HASH`
/// (opcode `0x00`) entry is part of every set, so it is included here too,
/// which is correct: a service's vocabulary includes its universal command.
pub fn service_hash_of_set(set: &DeviceCommandSet) -> u32 {
    use crate::engine::manifest;
    let mut rendered = String::new();
    manifest::push_commands(&mut rendered, &set.commands);
    manifest::fnv1a_32(manifest::canonicalize(&rendered).as_bytes())
}


/// The C++ `constexpr` name for a set's per-service hash, e.g. `system` →
/// `kServiceHashSystem`, `gpio_out` → `kServiceHashGpioOut`. Shared by the
/// header generator ([`render_service_hashes_header`]) and the registry
/// generator so the two never drift.
pub fn service_hash_const_name(set: &DeviceCommandSet) -> String {
    format!("kServiceHash{}", to_pascal_case(&set.kind))
}


/// Render the generated `command_ids.h`: one `enum class … : uint8_t` per
/// command set, members named in PascalCase with the opcode byte as the value.
///
/// The output is project-independent (it depends only on the embedded
/// catalog), so every generated tree gets byte-identical content. A checked-in
/// golden copy under `autogen_content/cpp/generated/command_ids.h` is asserted
/// equal to this in the [`tests`] module.
pub fn render_command_ids_header() -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef COMMAND_IDS_H\n");
    s.push_str("#define COMMAND_IDS_H\n");
    s.push('\n');
    s.push_str("#include <stdint.h>\n");
    s.push('\n');
    s.push_str("/// @file command_ids.h\n");
    s.push_str("/// @brief Command opcode enums for every sub-address kind.\n");
    s.push_str("/// @details Auto-generated from the Exhibition command catalog. Each\n");
    s.push_str("///          enumerator is the on-wire opcode byte; switch on the matching\n");
    s.push_str("///          enum in a handler instead of using raw literals.\n");
    s.push('\n');
    for set in all() {
        s.push_str(&format!("/// @brief Command opcodes for {}.\n", set.kind));
        s.push_str(&format!("enum class {} : uint8_t {{\n", set.cpp_enum));
        for c in &set.commands {
            s.push_str(&format!("    {} = 0x{:02X},\n", to_pascal_case(&c.name), c.opcode));
        }
        s.push_str("};\n");
        s.push('\n');
    }
    s.push_str("#endif // COMMAND_IDS_H\n");
    s
}


/// Render the generated `service_hashes.h`: one `constexpr uint32_t` per
/// command set, the FNV-1a per-service hash of that kind's command vocabulary
/// ([`service_hash_of_set`]).
///
/// Like `command_ids.h`, the output is project-independent (it depends only on
/// the embedded catalog), so every generated tree — RT and BC alike — gets
/// byte-identical content. A checked-in golden copy under
/// `autogen_content/cpp/generated/service_hashes.h` is asserted equal to this
/// in the [`tests`] module. The constants let the generated `device_registry.cpp`
/// wire each sub-address's hash into the `RemoteTerminal`, and let the BC
/// compare a runtime `GET_SERVICE_HASH` reply against the expected value.
pub fn render_service_hashes_header() -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef SERVICE_HASHES_H\n");
    s.push_str("#define SERVICE_HASHES_H\n");
    s.push('\n');
    s.push_str("#include <stdint.h>\n");
    s.push('\n');
    s.push_str("/// @file service_hashes.h\n");
    s.push_str("/// @brief Per-service (per-kind) API hashes for every sub-address kind.\n");
    s.push_str("/// @details Auto-generated from the Exhibition command catalog. Each value\n");
    s.push_str("///          is an FNV-1a hash over that kind's command vocabulary and is\n");
    s.push_str("///          returned at runtime by the universal GET_SERVICE_HASH command\n");
    s.push_str("///          (opcode 0x00) on every sub-address of that kind.\n");
    s.push('\n');
    for set in all() {
        s.push_str(&format!(
            "constexpr uint32_t {} = 0x{:08X}u;\n",
            service_hash_const_name(set),
            service_hash_of_set(set),
        ));
    }
    s.push('\n');
    s.push_str("#endif // SERVICE_HASHES_H\n");
    s
}


/// Render the generated `sub_addresses.h`: a single `enum class SubAddress :
/// uint8_t` naming every assigned sub-address. `System` is fixed at
/// [`SYSTEM_SUB_ADDRESS`]; each slot contributes one member named by
/// [`to_pascal_case`] of its instance name.
///
/// Unlike `command_ids.h`, this is project-specific (it depends on the device
/// set / sub-address assignment).
///
/// # Panics
/// Panics if two slots map to the same PascalCase member name. Instance names
/// are unique by construction, so this signals an unexpected naming clash
/// rather than normal input.
pub fn render_sub_addresses_header(slots: &[DeviceSlot]) -> String {
    let mut s = String::new();
    s.push_str(crate::engine::SPDX_CPP_HEADER);
    s.push_str("#ifndef SUB_ADDRESSES_H\n");
    s.push_str("#define SUB_ADDRESSES_H\n");
    s.push('\n');
    s.push_str("#include <stdint.h>\n");
    s.push('\n');
    s.push_str("/// @file sub_addresses.h\n");
    s.push_str("/// @brief Named sub-addresses assigned for this project.\n");
    s.push_str("/// @details Auto-generated from the device set. `System` is always 1;\n");
    s.push_str("///          peripheral sub-addresses (2..31) are assigned deterministically.\n");
    s.push('\n');
    s.push_str("enum class SubAddress : uint8_t {\n");
    s.push_str(&format!("    System = {},\n", SYSTEM_SUB_ADDRESS));

    let mut seen: HashSet<String> = HashSet::new();
    seen.insert("System".to_string());
    for slot in slots {
        let member = to_pascal_case(&slot.instance_name);
        if !seen.insert(member.clone()) {
            panic!(
                "sub-address enum member collision: '{member}' (from instance '{}')",
                slot.instance_name
            );
        }
        s.push_str(&format!("    {} = {},\n", member, slot.sub_address));
    }
    s.push_str("};\n");
    s.push('\n');
    s.push_str("#endif // SUB_ADDRESSES_H\n");
    s
}


/// Validate a catalog command description against the single-line, plain-text
/// invariant the manifest emitter and [`crate::engine::manifest::canonicalize`]
/// rely on.
///
/// The description is written into the YAML unquoted (exactly like the
/// per-device `label`), so it must stay on one line and avoid the few tokens
/// that would change how a YAML parser reads the value: a colon-space (`": "`,
/// which would start a nested mapping), a space-hash (`" #"`, an inline
/// comment), or surrounding whitespace. Returns an `Err` describing the
/// violation, suitable for embedding in a catalog parse error.
fn validate_description(desc: &str) -> Result<(), String> {
    if desc.is_empty() {
        return Err("must not be empty (omit the field for a TODO default)".to_string());
    }
    if desc.contains('\n') || desc.contains('\r') {
        return Err("must be single-line".to_string());
    }
    if desc.trim() != desc {
        return Err("must not have leading or trailing whitespace".to_string());
    }
    if desc.contains(": ") {
        return Err("must not contain a colon-space ': ' (breaks unquoted YAML)".to_string());
    }
    if desc.contains(" #") {
        return Err("must not contain a space-hash ' #' (parsed as a YAML comment)".to_string());
    }
    Ok(())
}


/// True if `s` is a valid (non-empty) C++ identifier: leading letter or
/// underscore, then letters/digits/underscores.
fn is_cpp_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}


/// Convert a `snake_case` / `SCREAMING_SNAKE_CASE` name to `PascalCase`,
/// always yielding a valid C++ identifier.
///
/// Each underscore-separated segment is capitalized (first letter upper, the
/// rest lower) with digit runs preserved, so `adc_hadc1_ch0` becomes
/// `AdcHadc1Ch0`, `GET_FINGERPRINT` becomes `GetFingerprint`, and
/// `gpio_out_bank_A` becomes `GpioOutBankA`. An already-mixed token is folded
/// to first-upper-rest-lower, so `gpio_PA5` becomes `GpioPa5`.
pub fn to_pascal_case(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for segment in raw.split('_') {
        let mut first_alpha = true;
        for ch in segment.chars() {
            if ch.is_ascii_alphabetic() {
                if first_alpha {
                    out.push(ch.to_ascii_uppercase());
                    first_alpha = false;
                } else {
                    out.push(ch.to_ascii_lowercase());
                }
            } else if ch.is_ascii_digit() {
                out.push(ch);
                first_alpha = false;
            }
            // Any other character is dropped; validated inputs never contain
            // characters outside [A-Za-z0-9_].
        }
    }
    if out.is_empty() {
        out.push('_');
    } else if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}
