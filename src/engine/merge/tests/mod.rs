// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`crate::engine::merge`] USER CODE preservation logic.

#[cfg(not(test))]
compile_error!("This file is only to be used in test build");

use crate::engine::merge::{preserve_command_descriptions, preserve_user_code};


/// t1
///
/// Round-trip identity: merging a freshly generated buffer against itself
/// (an empty region whose body is a single blank line) reproduces it exactly.
/// This is the unit-level guarantee behind the generator's idempotency.
#[test]
fn t1() {
    let g = "a\n/* USER CODE BEGIN Init */\n\n/* USER CODE END Init */\nb\n";
    assert_eq!(preserve_user_code(g, g), g);
}


/// t2
///
/// A user edit inside a region is spliced into the generated (empty) region,
/// with the generated marker framing preserved around it.
#[test]
fn t2() {
    let generated = "/* USER CODE BEGIN Init */\n\n/* USER CODE END Init */\n";
    let existing = "/* USER CODE BEGIN Init */\ninit_rt();\n/* USER CODE END Init */\n";
    let merged = preserve_user_code(generated, existing);
    assert_eq!(
        merged,
        "/* USER CODE BEGIN Init */\ninit_rt();\n/* USER CODE END Init */\n"
    );
}


/// t3
///
/// A region present only in the generated template (no counterpart in the
/// existing file) keeps its generated default body — e.g. a region added by a
/// newer generator shows up on regeneration.
#[test]
fn t3() {
    let generated = "/* USER CODE BEGIN New */\ndefault_body();\n/* USER CODE END New */\n";
    let existing = "nothing to preserve here\n";
    assert_eq!(preserve_user_code(generated, existing), generated);
}


/// t4
///
/// A region present in the existing file but absent from the template is moved
/// to the ORPHANED trailer rather than dropped, with the generated body still
/// emitted first.
#[test]
fn t4() {
    let generated = "x();\n";
    let existing = "/* USER CODE BEGIN Gone */\nkeep_me();\n/* USER CODE END Gone */\n";
    let merged = preserve_user_code(generated, existing);
    assert!(merged.starts_with("x();\n"));
    assert!(merged.contains("ORPHANED USER CODE"));
    assert!(merged.contains("/* USER CODE BEGIN Gone */"));
    assert!(merged.contains("keep_me();"));
}


/// t5
///
/// Merging again with the trailered output as the existing file reproduces the
/// same bytes: the orphan trailer is idempotent, so repeated regeneration does
/// not accumulate or duplicate it.
#[test]
fn t5() {
    let generated = "x();\n";
    let existing = "/* USER CODE BEGIN Gone */\nkeep_me();\n/* USER CODE END Gone */\n";
    let once = preserve_user_code(generated, existing);
    let twice = preserve_user_code(generated, &once);
    assert_eq!(once, twice);
}


/// t6
///
/// Indented regions (markers inside a function body) are recognized and their
/// indented bodies preserved.
#[test]
fn t6() {
    let generated = "    /* USER CODE BEGIN Init */\n\n    /* USER CODE END Init */\n";
    let existing =
        "    /* USER CODE BEGIN Init */\n        do_thing();\n    /* USER CODE END Init */\n";
    let merged = preserve_user_code(generated, existing);
    assert!(merged.contains("        do_thing();"));
}


/// t7
///
/// CRLF line terminators inside a preserved body are kept byte-for-byte; the
/// merge performs no newline normalization.
#[test]
fn t7() {
    let generated = "/* USER CODE BEGIN Init */\n\n/* USER CODE END Init */\n";
    let existing = "/* USER CODE BEGIN Init */\r\nwin();\r\n/* USER CODE END Init */\n";
    let merged = preserve_user_code(generated, existing);
    assert!(merged.contains("win();\r\n"));
}


/// t8
///
/// A marker that appears mid-line inside a string literal is not treated as a
/// region delimiter; the line is copied through verbatim.
#[test]
fn t8() {
    let generated = "const char* s = \"/* USER CODE BEGIN X */\";\n";
    let existing = "anything\n";
    assert_eq!(preserve_user_code(generated, existing), generated);
}


/// t9
///
/// A tag that appears twice in the existing file keeps the first body and warns
/// (no panic); the second body is discarded.
#[test]
fn t9() {
    let existing = "/* USER CODE BEGIN Dup */\nfirst();\n/* USER CODE END Dup */\n\
                    /* USER CODE BEGIN Dup */\nsecond();\n/* USER CODE END Dup */\n";
    let generated = "/* USER CODE BEGIN Dup */\n\n/* USER CODE END Dup */\n";
    let merged = preserve_user_code(generated, existing);
    assert!(merged.contains("first();"));
    assert!(!merged.contains("second();"));
}


/// t10
///
/// An unterminated BEGIN in the existing file is ignored (no panic); the
/// generated default for that region is kept.
#[test]
fn t10() {
    let existing = "/* USER CODE BEGIN Open */\nstuff();\n";
    let generated = "/* USER CODE BEGIN Open */\n\n/* USER CODE END Open */\n";
    assert_eq!(preserve_user_code(generated, existing), generated);
}


/// t11
///
/// Several distinct tags in one file are preserved independently.
#[test]
fn t11() {
    let generated = "/* USER CODE BEGIN A */\n\n/* USER CODE END A */\n\
                     mid\n\
                     /* USER CODE BEGIN B */\n\n/* USER CODE END B */\n";
    let existing = "/* USER CODE BEGIN A */\naaa();\n/* USER CODE END A */\n\
                    mid\n\
                    /* USER CODE BEGIN B */\nbbb();\n/* USER CODE END B */\n";
    let merged = preserve_user_code(generated, existing);
    assert_eq!(merged, existing);
}


// ---- preserve_command_descriptions (manifest description merge) ----

/// A freshly generated manifest body, every command description a `TODO`.
const GEN_MANIFEST: &str = "\
description: device label
commands:
  - opcode: 0x01
    name: A
    description: TODO
    cmd_payload:
      bytes: 0
  - opcode: 0x02
    name: B
    description: TODO
    cmd_payload:
      bytes: 0
";


/// t12
///
/// A filled-in description in the existing file is spliced into the generated
/// `TODO` for the same opcode; a command whose existing description is still
/// `TODO` keeps the generated `TODO`.
#[test]
fn t12() {
    let existing = "\
description: device label
commands:
  - opcode: 0x01
    name: A
    description: \"first cmd\"
    cmd_payload:
      bytes: 0
  - opcode: 0x02
    name: B
    description: TODO
    cmd_payload:
      bytes: 0
";
    let merged = preserve_command_descriptions(GEN_MANIFEST, existing);
    assert!(
        merged.contains("    description: \"first cmd\"\n"),
        "filled description spliced in by opcode"
    );
    assert_eq!(
        merged.matches("description: TODO").count(),
        1,
        "the still-TODO command keeps its placeholder"
    );
}


/// t13
///
/// Merging the generated body against itself (all `TODO`) is a byte-for-byte
/// no-op — the idempotency guarantee for an unedited regeneration.
#[test]
fn t13() {
    assert_eq!(preserve_command_descriptions(GEN_MANIFEST, GEN_MANIFEST), GEN_MANIFEST);
}


/// t14
///
/// Descriptions are matched by opcode regardless of command order in the
/// existing file, and the per-device (top-level) `description:` is regenerated
/// rather than pulled from a command.
#[test]
fn t14() {
    // Commands in the opposite order; a different device-level description.
    let existing = "\
description: SOMETHING ELSE
commands:
  - opcode: 0x02
    name: B
    description: \"second\"
    cmd_payload:
      bytes: 0
  - opcode: 0x01
    name: A
    description: \"first\"
    cmd_payload:
      bytes: 0
";
    let merged = preserve_command_descriptions(GEN_MANIFEST, existing);

    // The device-level description comes from the generated body, not existing.
    assert!(merged.contains("description: device label"));
    assert!(!merged.contains("SOMETHING ELSE"));

    // Each command description lands in its own (opcode-matched) block.
    let a = merged.find("name: A").unwrap();
    let b = merged.find("name: B").unwrap();
    assert!(merged[a..].starts_with("name: A\n    description: \"first\""));
    assert!(merged[b..].starts_with("name: B\n    description: \"second\""));
}


/// t15
///
/// An old-format existing file (no `description:` lines at all) leaves the
/// generated body unchanged.
#[test]
fn t15() {
    let existing = "\
commands:
  - opcode: 0x01
    name: A
    cmd_payload: 0
    resp_payload: 4 (uint32)
  - opcode: 0x02
    name: B
    cmd_payload: 0
    resp_payload: 0
";
    assert_eq!(preserve_command_descriptions(GEN_MANIFEST, existing), GEN_MANIFEST);
}


/// t16
///
/// Re-merging the already-merged output reproduces the same bytes: description
/// preservation is idempotent across repeated regenerations.
#[test]
fn t16() {
    let existing = "\
description: device label
commands:
  - opcode: 0x01
    name: A
    description: \"hand written\"
    cmd_payload:
      bytes: 0
  - opcode: 0x02
    name: B
    description: TODO
    cmd_payload:
      bytes: 0
";
    let once = preserve_command_descriptions(GEN_MANIFEST, existing);
    let twice = preserve_command_descriptions(GEN_MANIFEST, &once);
    assert_eq!(once, twice, "description preservation is idempotent");
    assert!(once.contains("description: \"hand written\""));
}
