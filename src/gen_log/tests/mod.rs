// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for [`crate::gen_log::GenLog`].

use super::GenLog;

/*
 * t1: A disabled logger must drop every recorded message and write no file.
 * Expects: after recording into a `disabled()` logger, every report section
 * renders as "_(none)_", the fingerprint shows "not computed", and `write_to`
 * leaves no file on disk.
 */
#[test]
fn t1() {
    let mut log = GenLog::disabled();
    log.source("ioc: foo.ioc");
    log.enabled_device("GPIO", "PA5", "signal GPIO_Output");
    log.ignored("PB0", "unrecognized signal 'RCC_OSC'");
    log.decision("emitted gpio handler");
    log.set_fingerprint(0xdead_beef);

    let rendered = log.render();
    assert!(rendered.contains("_(none)_"), "disabled sections should be empty");
    assert!(!rendered.contains("PA5"), "disabled logger must drop messages");
    assert!(
        rendered.contains("_(not computed)_"),
        "disabled logger must drop the fingerprint",
    );

    let dir = std::env::temp_dir().join("exhibition_genlog_t1");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("report.md");
    let _ = std::fs::remove_file(&path);
    log.write_to(&path).expect("write_to on disabled logger");
    assert!(!path.exists(), "disabled logger must not write a file");
}

/*
 * t2: A live logger must render every section it was given and format the
 * fingerprint as zero-padded lowercase hex. Expects: each recorded item and
 * the "0x" fingerprint appear in the rendered Markdown, and `write_to`
 * produces a file whose contents match `render()`.
 */
#[test]
fn t2() {
    let mut log = GenLog::new();
    log.source("ioc: foo.ioc");
    log.source("mcu: STM32F407VGTx");
    log.enabled_device("UART", "USART2", "TX=PA2, RX=PA3");
    log.ignored("PB0", "unrecognized signal 'RCC_OSC'");
    log.decision("transport: UART on USART2");
    log.set_fingerprint(0x0001_02ab);

    let rendered = log.render();
    assert!(rendered.contains("# Exhibition RT Generation Report"));
    assert!(rendered.contains("STM32F407VGTx"));
    assert!(rendered.contains("**UART** USART2 — TX=PA2, RX=PA3"));
    assert!(rendered.contains("unrecognized signal 'RCC_OSC'"));
    assert!(rendered.contains("transport: UART on USART2"));
    assert!(rendered.contains("`0x000102ab`"), "fingerprint must be 0x-padded hex");
    assert!(!rendered.contains("_(none)_"), "no section should be empty here");

    let dir = std::env::temp_dir().join("exhibition_genlog_t2");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("report.md");
    log.write_to(&path).expect("write_to on live logger");
    let on_disk = std::fs::read_to_string(&path).expect("read report");
    assert_eq!(on_disk, rendered, "written file must match render()");
}
