// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Unit tests for the [`ioc_parser`](super) module.

// Throw compile error if this file is included in a non-test build
#[cfg(not(test))]
compile_error!("This file is only to be used in test build");


use super::{parse_path, parse_str};

/// t1
///
/// Parse a GPIO-only fixture with one labelled output and one unlabelled
/// input. Verify both pins land in `gpios` with the correct name fallback
/// (label when present, pin id otherwise) and correct `output` flag, and
/// that no buses are produced.
#[test]
fn t1() {
    let ioc = "\
PA1.Signal=GPIO_Output
PA1.GPIO_Label=GREEN_LED_PIN
PB3.Signal=GPIO_Input
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.gpios.len(), 2);
    assert!(set.spi_buses.is_empty());
    assert!(set.uart_buses.is_empty());

    // Sorted by id: PA1 then PB3.
    assert_eq!(set.gpios[0].id(), "PA1");
    assert_eq!(set.gpios[0].name(), "GREEN_LED_PIN");
    assert_eq!(set.gpios[0].output(), true);

    assert_eq!(set.gpios[1].id(), "PB3");
    // No GPIO_Label was provided — should fall back to the pin id.
    assert_eq!(set.gpios[1].name(), "PB3");
    assert_eq!(set.gpios[1].output(), false);
}

/// t2
///
/// Parse a SPI fixture with NSS + SCK + MISO + MOSI assigned on SPI1.
/// Verify a single [`BusSpi`] is produced with `name == "SPI1"`,
/// `id == "hspi1"`, exactly one chip-select pin, both MISO and MOSI
/// populated, and that the bus is reported as valid.
#[test]
fn t2() {
    let ioc = "\
PA4.Signal=SPI1_NSS
PA5.Signal=SPI1_SCK
PA6.Signal=SPI1_MISO
PA7.Signal=SPI1_MOSI
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.spi_buses.len(), 1);
    assert!(set.uart_buses.is_empty());

    let bus = &set.spi_buses[0];
    assert_eq!(bus.name(), "SPI1");
    assert_eq!(bus.id(), "hspi1");
    assert_eq!(bus.cs().len(), 1);
    assert_eq!(bus.cs()[0].id(), "PA4");
    assert!(bus.miso().is_some());
    assert!(bus.mosi().is_some());
    assert_eq!(bus.clk().id(), "PA5");
    assert!(bus.is_valid().is_ok());
}

/// t3
///
/// Parse a USART fixture with TX + RX assigned on USART2. Verify a single
/// [`BusUart`] is produced with `name == "USART2"`, `id == "huart2"`, and
/// both `tx` and `rx` populated.
#[test]
fn t3() {
    let ioc = "\
PA2.Signal=USART2_TX
PA3.Signal=USART2_RX
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    assert!(set.spi_buses.is_empty());

    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "USART2");
    assert_eq!(bus.id(), "huart2");
    assert!(bus.tx().is_some());
    assert!(bus.rx().is_some());
    assert!(bus.is_valid().is_ok());
}

/// t4
///
/// A line containing no `=` is malformed and must produce an `Err` whose
/// message references the 1-based line number.
#[test]
fn t4() {
    let ioc = "\
PA1.Signal=GPIO_Output
this line has no equals sign
PB3.Signal=GPIO_Input
";
    let err = match parse_str(ioc) {
        Err(e) => e,
        Ok(_) => panic!("parse should fail on malformed line"),
    };
    assert!(err.contains("Line 2"), "error should mention line number: {}", err);
}

/// t5
///
/// A SPI peripheral that has MOSI assigned but no SCK is incomplete: the
/// parser must reject it with a message naming the peripheral.
#[test]
fn t5() {
    let ioc = "\
PA7.Signal=SPI1_MOSI
";
    let err = match parse_str(ioc) {
        Err(e) => e,
        Ok(_) => panic!("parse should fail without SCK"),
    };
    assert!(err.contains("SPI1"), "error should mention peripheral: {}", err);
}

/// t6
///
/// A pin whose `Signal` is an external-interrupt line (`GPIO_EXTI4`) is
/// classified as a non-output GPIO.
#[test]
fn t6() {
    let ioc = "\
PC4.Signal=GPIO_EXTI4
PC4.GPIO_Label=USER_BUTTON
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.gpios.len(), 1);
    assert_eq!(set.gpios[0].id(), "PC4");
    assert_eq!(set.gpios[0].name(), "USER_BUTTON");
    assert_eq!(set.gpios[0].output(), false);
}

/// t7
///
/// Parse a combined fixture covering GPIO + SPI1 + USART1 + UART4. Verify
/// all three result vectors are populated, that bus ordering is
/// deterministic (sorted by name), and that GPIO ordering is sorted by id.
#[test]
fn t7() {
    let ioc = "\
PA9.Signal=USART1_TX
PA10.Signal=USART1_RX
PC10.Signal=UART4_TX
PC11.Signal=UART4_RX
PA5.Signal=SPI1_SCK
PA6.Signal=SPI1_MISO
PA7.Signal=SPI1_MOSI
PB0.Signal=GPIO_Output
PB0.GPIO_Label=RED_LED
PB1.Signal=GPIO_Input
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.gpios.len(), 2);
    assert_eq!(set.gpios[0].id(), "PB0");
    assert_eq!(set.gpios[1].id(), "PB1");

    assert_eq!(set.spi_buses.len(), 1);
    assert_eq!(set.spi_buses[0].name(), "SPI1");
    // No NSS in this fixture — cs should be empty.
    assert!(set.spi_buses[0].cs().is_empty());

    assert_eq!(set.uart_buses.len(), 2);
    assert_eq!(set.uart_buses[0].name(), "UART4");
    assert_eq!(set.uart_buses[0].id(), "huart4");
    assert_eq!(set.uart_buses[1].name(), "USART1");
    assert_eq!(set.uart_buses[1].id(), "huart1");
}

/// t8
///
/// Round-trip through [`parse_path`]: write a small `.ioc` to a temp file,
/// parse it through the file-based entry point, and confirm the resulting
/// [`DeviceSet`] matches what [`parse_str`] produces for the same content.
#[test]
fn t8() {
    let ioc = "\
PA1.Signal=GPIO_Output
PA1.GPIO_Label=LED
PA5.Signal=SPI1_SCK
PA6.Signal=SPI1_MISO
";
    let mut path = std::env::temp_dir();
    path.push(format!("exhibition_ioc_parser_t8_{}.ioc", std::process::id()));
    std::fs::write(&path, ioc).expect("write fixture");

    let from_path = parse_path(&path).expect("parse_path should succeed");
    let from_str = parse_str(ioc).expect("parse_str should succeed");

    // Clean up the temp file before any assertion can short-circuit us.
    let _ = std::fs::remove_file(&path);

    assert_eq!(from_path.gpios.len(), from_str.gpios.len());
    assert_eq!(from_path.spi_buses.len(), from_str.spi_buses.len());
    assert_eq!(from_path.uart_buses.len(), from_str.uart_buses.len());

    assert_eq!(from_path.gpios[0].id(), "PA1");
    assert_eq!(from_path.gpios[0].name(), "LED");
    assert_eq!(from_path.spi_buses[0].name(), "SPI1");
    assert_eq!(from_path.spi_buses[0].clk().id(), "PA5");
}

/// t9
///
/// Parse an ADC fixture: `PA0.Signal=ADC1_IN3` with a `GPIO_Label`. Verify
/// that a single `Adc` is produced with `channel == 3`, `id == "hadc1"`,
/// `name` from the label, and that the pin id matches.
#[test]
fn t9() {
    let ioc = "\
PA0.Signal=ADC1_IN3
PA0.GPIO_Label=BATTERY_VOLTAGE
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.adcs.len(), 1);
    assert!(set.gpios.is_empty());
    let adc = &set.adcs[0];
    assert_eq!(adc.channel(), 3);
    assert_eq!(adc.id(), "hadc1");
    assert_eq!(adc.name(), "BATTERY_VOLTAGE");
    assert_eq!(adc.pin().id(), "PA0");
}

/// t10
///
/// Parse a DAC fixture covering both signal shapes: `DAC1_OUT1` (peripheral
/// number explicit) and `DAC_OUT2` (peripheral number implicit, defaults to
/// 1). Both should land under `id == "hdac1"`, with their channels carried
/// through.
#[test]
fn t10() {
    let ioc = "\
PA4.Signal=DAC1_OUT1
PA5.Signal=DAC_OUT2
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.dacs.len(), 2);
    // Sorted by id then channel: both have id "hdac1", channel 1 first then 2.
    assert_eq!(set.dacs[0].id(), "hdac1");
    assert_eq!(set.dacs[0].channel(), 1);
    assert_eq!(set.dacs[1].id(), "hdac1");
    assert_eq!(set.dacs[1].channel(), 2);
}

/// t11
///
/// Parse a PWM fixture: one positive channel (`TIM2_CH1`) and one
/// complementary output (`TIM1_CH1N`). Only the positive channel produces
/// a `Pwm`; the complementary output is silently ignored.
#[test]
fn t11() {
    let ioc = "\
PA0.Signal=TIM2_CH1
PA0.GPIO_Label=FAN_PWM
PB0.Signal=TIM1_CH1N
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.pwms.len(), 1);
    let pwm = &set.pwms[0];
    assert_eq!(pwm.id(), "htim2");
    assert_eq!(pwm.channel(), 1);
    assert_eq!(pwm.name(), "FAN_PWM");
    assert_eq!(pwm.pin().id(), "PA0");
}

/// t12
///
/// Parse an I2C fixture: SDA + SCL on `I2C1` produces one `BusI2c` with
/// `id == "hi2c1"`. A separate fixture with `I2C2_SDA` but no SCL produces
/// an error referencing `I2C2`.
#[test]
fn t12() {
    let ioc = "\
PB7.Signal=I2C1_SDA
PB6.Signal=I2C1_SCL
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert_eq!(set.i2c_buses.len(), 1);
    let bus = &set.i2c_buses[0];
    assert_eq!(bus.name(), "I2C1");
    assert_eq!(bus.id(), "hi2c1");
    assert_eq!(bus.sda().id(), "PB7");
    assert_eq!(bus.scl().id(), "PB6");

    let missing_scl = "\
PB7.Signal=I2C2_SDA
";
    let err = match parse_str(missing_scl) {
        Err(e) => e,
        Ok(_) => panic!("parse should fail without SCL"),
    };
    assert!(err.contains("I2C2"), "error should reference peripheral: {}", err);
}

/// t13
///
/// Parse a CAN fixture mixing legacy `CAN<n>_*` and `FDCAN<n>_*` signals.
/// Verify both families produce a `BusCan` with the correct HAL handle id
/// (`hcan1` vs `hfdcan1`), and that the result is sorted deterministically
/// by name.
#[test]
fn t13() {
    let ioc = "\
PB9.Signal=CAN1_TX
PB8.Signal=CAN1_RX
PD1.Signal=FDCAN1_TX
PD0.Signal=FDCAN1_RX
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert_eq!(set.can_buses.len(), 2);
    // Sorted by name: "CAN1" before "FDCAN1".
    assert_eq!(set.can_buses[0].name(), "CAN1");
    assert_eq!(set.can_buses[0].id(), "hcan1");
    assert_eq!(set.can_buses[1].name(), "FDCAN1");
    assert_eq!(set.can_buses[1].id(), "hfdcan1");
}

/// t14
///
/// Parse two USB fixtures: one OTG (`USB_OTG_FS_*` with all four pins)
/// producing a `BusUsb` with `id == "husb_otg_fs"` and both optional pins
/// populated; one plain USB (`USB_DP`/`USB_DM` only) producing a `BusUsb`
/// with `id == "husb"` and no optional pins.
#[test]
fn t14() {
    let ioc_otg = "\
PA12.Signal=USB_OTG_FS_DP
PA11.Signal=USB_OTG_FS_DM
PA9.Signal=USB_OTG_FS_VBUS
PA10.Signal=USB_OTG_FS_ID
";
    let set = parse_str(ioc_otg).expect("parse should succeed");
    assert_eq!(set.usb_buses.len(), 1);
    let bus = &set.usb_buses[0];
    assert_eq!(bus.name(), "USB_OTG_FS");
    assert_eq!(bus.id(), "husb_otg_fs");
    assert_eq!(bus.dp().id(), "PA12");
    assert_eq!(bus.dm().id(), "PA11");
    assert!(bus.vbus().is_some());
    assert!(bus.id_pin().is_some());

    let ioc_plain = "\
PA12.Signal=USB_DP
PA11.Signal=USB_DM
";
    let set = parse_str(ioc_plain).expect("parse should succeed");
    assert_eq!(set.usb_buses.len(), 1);
    let bus = &set.usb_buses[0];
    assert_eq!(bus.name(), "USB");
    assert_eq!(bus.id(), "husb");
    assert!(bus.vbus().is_none());
    assert!(bus.id_pin().is_none());
}


/// Build the absolute path to the on-disk `test.ioc` fixture. Anchored at
/// `CARGO_MANIFEST_DIR` so the path is independent of the directory
/// `cargo test` is invoked from.
fn test_ioc_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/ioc_parser/tests/test.ioc")
}


/// t15
///
/// Parse the real on-disk `test.ioc` fixture (STM32CubeMX export for an
/// STM32L433RCT6P) through [`parse_path`] and verify the top-level shape
/// of the resulting [`DeviceSet`]: the MCU model is extracted from
/// `ProjectManager.DeviceId` into `core` as `"STM32L433RCTxP"`, and the
/// pin-derived collections contain exactly one GPIO, one I2C bus, one
/// SPI bus, and no UART/CAN/USB/ADC/DAC/PWM entries. Subsequent tests
/// (t16–t19) drill into the details.
#[test]
fn t15() {
    let set = parse_path(&test_ioc_path()).expect("parse test.ioc");

    assert_eq!(set.core.as_ref().unwrap().model(), "STM32L433RCTxP");

    assert_eq!(set.gpios.len(), 1);
    assert_eq!(set.i2c_buses.len(), 1);
    assert_eq!(set.spi_buses.len(), 1);

    assert!(set.uart_buses.is_empty());
    assert!(set.can_buses.is_empty());
    assert!(set.usb_buses.is_empty());
    assert!(set.adcs.is_empty());
    assert!(set.dacs.is_empty());
    assert!(set.pwms.is_empty());
}

/// t16
///
/// Parse `test.ioc` and verify the SPI2 bus details: name `SPI2`, id
/// `hspi2`, clock pin `PB10`, MISO pin `PC2`, MOSI pin `PC3`, and an
/// empty chip-select list (the fixture assigns no NSS pin). This
/// exercises a real-world NSS-less SPI bus.
#[test]
fn t16() {
    let set = parse_path(&test_ioc_path()).expect("parse test.ioc");

    let bus = &set.spi_buses[0];
    assert_eq!(bus.name(), "SPI2");
    assert_eq!(bus.id(), "hspi2");
    assert_eq!(bus.clk().id(), "PB10");
    assert_eq!(bus.miso().unwrap().id(), "PC2");
    assert_eq!(bus.mosi().unwrap().id(), "PC3");
    assert!(bus.cs().is_empty());
    assert!(bus.is_valid().is_ok());
}

/// t17
///
/// Parse `test.ioc` and verify the I2C1 bus details: name `I2C1`, id
/// `hi2c1`, SDA pin `PA10`, SCL pin `PA9`, and a valid bus.
#[test]
fn t17() {
    let set = parse_path(&test_ioc_path()).expect("parse test.ioc");

    let bus = &set.i2c_buses[0];
    assert_eq!(bus.name(), "I2C1");
    assert_eq!(bus.id(), "hi2c1");
    assert_eq!(bus.sda().id(), "PA10");
    assert_eq!(bus.scl().id(), "PA9");
    assert!(bus.is_valid().is_ok());
}

/// t18
///
/// Parse `test.ioc` and verify the lone GPIO: `PC6` configured as a
/// non-output input, with no `GPIO_Label` in the fixture so the name
/// falls back to the pin id. The fixture also contains two
/// `GPIO_Output` entries whose keys are escaped with `\ (...)`
/// (`PA13\ (JTMS-SWDIO)` and `PC14-OSC32_IN\ (PC14)`); those must NOT
/// produce GPIOs because the pin-signal regex requires a bare
/// `P[A-K]\d{1,2}` key. Asserting `gpios.len() == 1` combined with the
/// PC6 identity check locks this in.
#[test]
fn t18() {
    let set = parse_path(&test_ioc_path()).expect("parse test.ioc");

    assert_eq!(set.gpios.len(), 1);
    let gpio = &set.gpios[0];
    assert_eq!(gpio.id(), "PC6");
    assert_eq!(gpio.name(), "PC6");
    assert_eq!(gpio.output(), false);
}

/// t19
///
/// Parse `test.ioc` and verify that the `Signal=SharedAnalog_PAx` pins
/// (PA5, PA6) are silently ignored by the parser. Although the fixture
/// also contains `SH.SharedAnalog_PA5.0=DAC1_OUT2` and
/// `SH.SharedAnalog_PA6.0=ADC1_IN11` metadata describing the underlying
/// analog routing, no signal regex matches `SharedAnalog_*`, so neither
/// pin should appear as a GPIO, ADC, or DAC. This locks in today's
/// behavior; if a future change teaches the parser to resolve `SH.*`
/// aliases, this test will fail loudly and should be updated then.
#[test]
fn t19() {
    let set = parse_path(&test_ioc_path()).expect("parse test.ioc");

    assert!(
        set.gpios.iter().all(|g| g.id() != "PA5" && g.id() != "PA6"),
        "SharedAnalog pins should not appear as GPIOs",
    );
    assert!(
        set.adcs.iter().all(|a| a.pin().id() != "PA6"),
        "SharedAnalog_PA6 should not produce an ADC",
    );
    assert!(
        set.dacs.iter().all(|d| d.pin().id() != "PA5"),
        "SharedAnalog_PA5 should not produce a DAC",
    );
}

/// t20
///
/// Verify that pins with CubeMX-appended decorations (e.g. `PA0-WKUP`,
/// `PC14-OSC32_IN`, `PH0-OSC_IN`) parse correctly: the resulting `Gpio`
/// id is the canonical `P<bank><pin>` form, and the signal is classified
/// the same as it would be for an undecorated pin. Covers the UART4
/// TX-on-PA0-WKUP case: the parser must keep the suffixed key, not drop it.
#[test]
fn t20() {
    let ioc = "\
PA0-WKUP.Signal=UART4_TX
PA0-WKUP.GPIO_Label=MyUart4Tx
PA1.Signal=UART4_RX
PA1.GPIO_Label=MyUart4Rx
PC14-OSC32_IN.Signal=GPIO_Input
PH0-OSC_IN.Signal=GPIO_Output
PH0-OSC_IN.GPIO_Label=BOARD_RESET
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "UART4");
    assert_eq!(bus.id(), "huart4");
    let tx = bus.tx().expect("TX should be present despite -WKUP suffix");
    assert_eq!(tx.id(), "PA0");
    assert_eq!(tx.name(), "MyUart4Tx");
    let rx = bus.rx().expect("RX should be present");
    assert_eq!(rx.id(), "PA1");

    // PC14-OSC32_IN and PH0-OSC_IN should land as plain GPIOs.
    assert!(set.gpios.iter().any(|g| g.id() == "PC14" && !g.output()));
    let ph0 = set.gpios.iter().find(|g| g.id() == "PH0").expect("PH0");
    assert!(ph0.output());
    assert_eq!(ph0.name(), "BOARD_RESET");
}

/// t21
///
/// Verify that a `<PERIPH>.BaudRate=<u32>` entry alongside the TX/RX
/// signal lines lands on the resulting [`BusUart::baud_rate`] for the
/// matching peripheral. This is the case that lets downstream codegen
/// avoid hardcoding 115200 when the `.ioc` already specifies a value.
#[test]
fn t21() {
    let ioc = "\
PD8.Signal=USART3_TX
PB11.Signal=USART3_RX
USART3.BaudRate=38400
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "USART3");
    assert_eq!(bus.baud_rate(), Some(38400));
}

/// t22
///
/// Verify that a UART whose CubeMX configuration omits `<PERIPH>.BaudRate`
/// is still constructed successfully, with `baud_rate() == None`. This is
/// the "unknown baud" signal that downstream codegen uses to decide
/// whether to emit the fallback-warning note.
#[test]
fn t22() {
    let ioc = "\
PA2.Signal=USART2_TX
PA3.Signal=USART2_RX
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "USART2");
    assert_eq!(bus.baud_rate(), None);
}

/// t23
///
/// Verify that the BaudRate extraction matches every UART family CubeMX
/// can emit: `USART`, `UART`, and `LPUART`. Uses an LPUART fixture
/// because it is the family most easily missed by an overly tight regex.
#[test]
fn t23() {
    let ioc = "\
PG7.Signal=LPUART1_TX
PG8.Signal=LPUART1_RX
LPUART1.BaudRate=230400
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "LPUART1");
    assert_eq!(bus.id(), "hlpuart1");
    assert_eq!(bus.baud_rate(), Some(230400));
}

/// t24
///
/// Verify that a non-numeric `<PERIPH>.BaudRate` value bubbles up as a
/// parse error whose message references the offending key, rather than
/// being silently dropped.
#[test]
fn t24() {
    let ioc = "\
PA2.Signal=USART2_TX
PA3.Signal=USART2_RX
USART2.BaudRate=not_a_number
";
    let err = match parse_str(ioc) {
        Ok(_) => panic!("invalid baud should produce an error"),
        Err(e) => e,
    };
    assert!(err.contains("USART2.BaudRate"),
        "error should reference the offending key; got: {}", err);
}

/// t25
///
/// Verify that `ProjectManager.UnderRoot=false` and
/// `ProjectManager.TargetToolchain` flow into [`ProjectLayout`]. This is the
/// CubeMX "advanced / toolchain subfolder" layout where user code builds from
/// `<toolchain>/Application/User/Core`, not the project-root `Core/Src`.
#[test]
fn t25() {
    let ioc = "\
ProjectManager.UnderRoot=false
ProjectManager.TargetToolchain=STM32CubeIDE
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert_eq!(set.layout.under_root, false);
    assert_eq!(set.layout.toolchain, "STM32CubeIDE");
}

/// t26
///
/// Verify that an `.ioc` with no `ProjectManager.UnderRoot` /
/// `TargetToolchain` keys (e.g. a minimal fixture) falls back to the
/// root layout: `under_root == true`, empty toolchain.
#[test]
fn t26() {
    let set = parse_str("PA2.Signal=USART2_TX\n").expect("parse should succeed");
    assert_eq!(set.layout.under_root, true);
    assert!(set.layout.toolchain.is_empty());
}

/// t27
///
/// Verify that a USB peripheral configured as a host (the real
/// `USB_OTG_FS.VirtualMode=Host_Only` + USB Host middleware case that made
/// the generator pick USB over UART3) is parsed with
/// `is_cdc_device() == false`, so the transport picker will exclude it.
#[test]
fn t27() {
    let ioc = "\
PA12.Signal=USB_OTG_FS_DP
PA11.Signal=USB_OTG_FS_DM
USB_OTG_FS.VirtualMode=Host_Only
USB_HOST.VirtualModeFS=Cdc
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert_eq!(set.usb_buses.len(), 1);
    assert!(!set.usb_buses[0].is_cdc_device(),
        "host-mode USB must not be CDC-device eligible");
}

/// t28
///
/// Verify the device-mode USB paths are CDC-device eligible: an explicit
/// `VirtualMode=Device_Only` peripheral, and (separately) a pins-only fixture
/// with no mode/middleware info, which defaults to device-eligible to
/// preserve the historical permissive behavior.
#[test]
fn t28() {
    let ioc_device = "\
PA12.Signal=USB_OTG_FS_DP
PA11.Signal=USB_OTG_FS_DM
USB_OTG_FS.VirtualMode=Device_Only
USB_DEVICE.CLASS_NAME_FS=CDC
";
    let set = parse_str(ioc_device).expect("parse should succeed");
    assert!(set.usb_buses[0].is_cdc_device(),
        "device-mode USB must be CDC-device eligible");

    let ioc_pins_only = "\
PA12.Signal=USB_DP
PA11.Signal=USB_DM
";
    let set = parse_str(ioc_pins_only).expect("parse should succeed");
    assert!(set.usb_buses[0].is_cdc_device(),
        "pins-only USB should default to device-eligible");
}

/// t29
///
/// Parse an ADCx fixture: `PA1.Signal=ADCx_IN1` paired with the CubeMX ADC
/// config entry `ADC1.Channel-0\#ChannelRegularConversion=ADC_CHANNEL_1`.
/// The instance must be resolved to ADC1, yielding channel==1, id=="hadc1".
#[test]
fn t29() {
    let ioc = "\
PA1.Signal=ADCx_IN1
PA1.GPIO_Label=ANALOG_IN
ADC1.Channel-0\\#ChannelRegularConversion=ADC_CHANNEL_1
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert_eq!(set.adcs.len(), 1);
    assert!(set.gpios.is_empty());
    let adc = &set.adcs[0];
    assert_eq!(adc.channel(), 1);
    assert_eq!(adc.id(), "hadc1");
    assert_eq!(adc.name(), "ANALOG_IN");
    assert_eq!(adc.pin().id(), "PA1");
}

/// t30
///
/// `ADCx_INn` with no matching config entry must be silently skipped — no
/// ADC device and no parse error.
#[test]
fn t30() {
    let ioc = "\
PA1.Signal=ADCx_IN1
";
    let set = parse_str(ioc).expect("parse should succeed");
    assert!(set.adcs.is_empty(), "unresolved ADCx signal must be silently dropped");
}

/// t31
///
/// A CAN/FDCAN peripheral that supplies only one of TX/RX is incomplete and
/// must produce an `Err` naming the peripheral and the missing pin. Covers both
/// `build_can_buses` error paths (missing-RX and missing-TX), which no prior
/// test exercised.
#[test]
fn t31() {
    // CAN1 with TX but no RX.
    let err = match parse_str("PB9.Signal=CAN1_TX\n") {
        Err(e) => e,
        Ok(_) => panic!("CAN1 without RX should fail"),
    };
    assert!(err.contains("CAN1") && err.contains("missing RX"),
        "error should name the bus and the missing pin: {}", err);

    // FDCAN1 with RX but no TX.
    let err = match parse_str("PD0.Signal=FDCAN1_RX\n") {
        Err(e) => e,
        Ok(_) => panic!("FDCAN1 without TX should fail"),
    };
    assert!(err.contains("FDCAN1") && err.contains("missing TX"),
        "error should name the bus and the missing pin: {}", err);
}

/// t32
///
/// A USB peripheral declaring only DP (no DM) is incomplete and must produce an
/// `Err` naming the peripheral and the missing pin, covering the
/// `build_usb_buses` error path that no prior test reached.
#[test]
fn t32() {
    let err = match parse_str("PA12.Signal=USB_OTG_FS_DP\n") {
        Err(e) => e,
        Ok(_) => panic!("USB with DP but no DM should fail"),
    };
    assert!(err.contains("USB_OTG_FS") && err.contains("missing DM"),
        "error should name the bus and the missing pin: {}", err);
}

/// t33
///
/// A QUADSPI peripheral declared via its pin signals is detected as a single
/// `Qspi` with the canonical `hqspi` handle. The QUADSPI pins must NOT be
/// misclassified as GPIOs (their signals match none of the pin regexes).
#[test]
fn t33() {
    let ioc = "\
PB10.Signal=QUADSPI_BK1_NCS
PE2.Signal=QUADSPI_BK1_IO2
PF10.Signal=QUADSPI_CLK
PF6.Signal=QUADSPI_BK1_IO3
PF8.Signal=QUADSPI_BK1_IO0
PF9.Signal=QUADSPI_BK1_IO1
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.qspis.len(), 1);
    assert_eq!(set.qspis[0].id(), "hqspi");
    assert!(set.qspis[0].is_valid().is_ok());
    // QUADSPI pins are not GPIOs.
    assert!(set.gpios.is_empty());
    assert!(set.spi_buses.is_empty());
}

/// t34
///
/// A QUADSPI peripheral declared only via a `Mcu.IP*` entry (no pin signals,
/// as in trimmed fixtures) is still detected.
#[test]
fn t34() {
    let set = parse_str("Mcu.IP4=QUADSPI\n").expect("parse should succeed");
    assert_eq!(set.qspis.len(), 1);
    assert_eq!(set.qspis[0].id(), "hqspi");
}

/// t35
///
/// A project with no QUADSPI configuration yields no `Qspi`.
#[test]
fn t35() {
    let set = parse_str("PA1.Signal=GPIO_Output\n").expect("parse should succeed");
    assert!(set.qspis.is_empty());
}

/// t36
///
/// CubeMX may decorate a dual-function pin with a slash (`PA0/WKUP`) instead of
/// a dash. The pin must still be discovered, its canonical id reduced to
/// `PA0`, and its signal classified — here a UART4 whose TX lands on the
/// slash-decorated pin yields a complete bus with both TX and RX.
#[test]
fn t36() {
    let ioc = "\
PA0/WKUP.Signal=UART4_TX
PA1.Signal=UART4_RX
";
    let set = parse_str(ioc).expect("parse should succeed");

    assert_eq!(set.uart_buses.len(), 1);
    let bus = &set.uart_buses[0];
    assert_eq!(bus.name(), "UART4");
    assert_eq!(bus.id(), "huart4");
    let tx = bus.tx().expect("TX should be discovered on the slash-decorated pin");
    assert_eq!(tx.id(), "PA0");
    assert!(bus.rx().is_some());
    assert!(bus.is_valid().is_ok());
}
