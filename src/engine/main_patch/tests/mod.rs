// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for [`crate::engine::main_patch`] — automatic wiring of the three
//! RT integration lines into CubeMX `main.c`.

#[cfg(not(test))]
compile_error!("This file is only to be used in test build");

use std::path::PathBuf;

use crate::engine::main_patch::{integrate_rt, patch_main_file, LineStatus};
use crate::ioc_parser::ProjectLayout;


/// A minimal but realistic CubeMX `main.c` skeleton with the three relevant
/// USER CODE regions empty, indented the way CubeMX emits them (column 0 for
/// `Includes`, two spaces for `2` and `3`).
fn bare_main_c() -> String {
    String::from(
        "#include \"main.h\"\n\
         /* USER CODE BEGIN Includes */\n\
         /* USER CODE END Includes */\n\
         int main(void)\n\
         {\n\
         \x20\x20MX_GPIO_Init();\n\
         \x20\x20/* USER CODE BEGIN 2 */\n\
         \x20\x20/* USER CODE END 2 */\n\
         \x20\x20while (1)\n\
         \x20\x20{\n\
         \x20\x20\x20\x20/* USER CODE BEGIN 3 */\n\
         \x20\x20\x20\x20/* USER CODE END 3 */\n\
         \x20\x20}\n\
         }\n",
    )
}


/// Produce a unique scratch directory, preferring the ramdisk. Mirrors the
/// helper used by the engine/bc_codegen test modules.
fn unique_tmp_dir(prefix: &str) -> PathBuf {
    let ramdisk = std::path::Path::new("/media/ramdisk");
    let root: PathBuf = if ramdisk.is_dir() {
        ramdisk.to_path_buf()
    } else {
        std::env::temp_dir()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    root.join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
}


/// t1
///
/// A fresh CubeMX `main.c` with empty regions gets all three lines inserted,
/// each immediately after its `BEGIN` marker, with the marker line's
/// indentation (none for `Includes`, two spaces for `2`, four for `3`).
#[test]
fn t1() {
    let (out, report) = integrate_rt(&bare_main_c());

    assert_eq!(report.include, LineStatus::Inserted);
    assert_eq!(report.init, LineStatus::Inserted);
    assert_eq!(report.tick, LineStatus::Inserted);
    assert!(report.changed);

    assert!(out.contains(
        "/* USER CODE BEGIN Includes */\n#include \"rt/device_registry.h\"\n/* USER CODE END Includes */"
    ));
    assert!(out.contains(
        "  /* USER CODE BEGIN 2 */\n  init_rt();\n  /* USER CODE END 2 */"
    ));
    assert!(out.contains(
        "    /* USER CODE BEGIN 3 */\n    tick_rt();\n    /* USER CODE END 3 */"
    ));
}


/// t2
///
/// Idempotency: feeding the patched output back through `integrate_rt` inserts
/// nothing, leaves the text byte-for-byte identical, and reports every line as
/// already present.
#[test]
fn t2() {
    let (first, _) = integrate_rt(&bare_main_c());
    let (second, report) = integrate_rt(&first);

    assert_eq!(first, second);
    assert!(!report.changed);
    assert_eq!(report.include, LineStatus::AlreadyPresent);
    assert_eq!(report.init, LineStatus::AlreadyPresent);
    assert_eq!(report.tick, LineStatus::AlreadyPresent);
}


/// t3
///
/// A region that already holds a hand-written call (here `init_rt()` without a
/// trailing semicolon-only difference) is detected via the marker substring and
/// left untouched — no duplicate is added.
#[test]
fn t3() {
    let src = bare_main_c().replace(
        "  /* USER CODE BEGIN 2 */\n  /* USER CODE END 2 */",
        "  /* USER CODE BEGIN 2 */\n  init_rt(); // wired by hand\n  /* USER CODE END 2 */",
    );

    let (out, report) = integrate_rt(&src);

    assert_eq!(report.init, LineStatus::AlreadyPresent);
    // Exactly one occurrence of init_rt( remains.
    assert_eq!(out.matches("init_rt(").count(), 1);
    // The other two were still missing, so they get inserted.
    assert_eq!(report.include, LineStatus::Inserted);
    assert_eq!(report.tick, LineStatus::Inserted);
}


/// t4
///
/// Pre-existing user code in a region is preserved; the new line is inserted at
/// the top of the region, above what the user already wrote.
#[test]
fn t4() {
    let src = bare_main_c().replace(
        "    /* USER CODE BEGIN 3 */\n    /* USER CODE END 3 */",
        "    /* USER CODE BEGIN 3 */\n    poll_my_sensor();\n    /* USER CODE END 3 */",
    );

    let (out, report) = integrate_rt(&src);

    assert_eq!(report.tick, LineStatus::Inserted);
    assert!(out.contains(
        "    /* USER CODE BEGIN 3 */\n    tick_rt();\n    poll_my_sensor();\n    /* USER CODE END 3 */"
    ));
}


/// t5
///
/// When a region's `BEGIN` marker is absent (here region `3` is removed
/// entirely), that line reports `RegionMissing` and the file is otherwise
/// untouched, while the regions that do exist are still patched.
#[test]
fn t5() {
    let src = bare_main_c().replace(
        "    /* USER CODE BEGIN 3 */\n    /* USER CODE END 3 */\n",
        "",
    );

    let (out, report) = integrate_rt(&src);

    assert_eq!(report.tick, LineStatus::RegionMissing);
    assert!(!out.contains("tick_rt("));
    // Includes + init were present and still get wired in.
    assert_eq!(report.include, LineStatus::Inserted);
    assert_eq!(report.init, LineStatus::Inserted);
    assert!(out.contains("init_rt();"));
}


/// t6
///
/// CRLF line endings are preserved: the inserted lines use `\r\n`, matching the
/// marker line, and no stray `\n`-only terminators are introduced.
#[test]
fn t6() {
    let crlf = bare_main_c().replace('\n', "\r\n");
    let (out, report) = integrate_rt(&crlf);

    assert!(report.changed);
    assert!(out.contains("/* USER CODE BEGIN 2 */\r\n  init_rt();\r\n"));
    // Every newline is part of a CRLF pair (no lone '\n').
    assert!(!out.replace("\r\n", "").contains('\n'));
}


/// t7
///
/// File wrapper: a project tree with no `main.c`/`main.cpp` yields `Ok(None)`;
/// one containing only `main.cpp` gets that file patched on disk.
#[test]
fn t7() {
    let layout = ProjectLayout::default(); // under_root = true -> Core/Src
    let root = unique_tmp_dir("exhibition_main_patch_t7");
    let core_src = root.join("Core/Src");
    std::fs::create_dir_all(&core_src).expect("mkdir");

    // No main.* present yet.
    assert!(patch_main_file(&root, &layout).unwrap().is_none());

    // Only main.cpp present -> it gets patched.
    let main_cpp = core_src.join("main.cpp");
    std::fs::write(&main_cpp, bare_main_c()).expect("write main.cpp");
    let (path, report) = patch_main_file(&root, &layout).unwrap().expect("some");
    assert_eq!(path, main_cpp);
    assert!(report.fully_wired());
    let on_disk = std::fs::read_to_string(&main_cpp).unwrap();
    assert!(on_disk.contains("init_rt();"));
    assert!(on_disk.contains("tick_rt();"));
    assert!(on_disk.contains("#include \"rt/device_registry.h\""));

    std::fs::remove_dir_all(&root).ok();
}
