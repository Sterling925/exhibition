// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Embedded C++ source content used by [`crate::engine`] when emitting an
//! RT firmware project.
//!
//! Every template file under `cpp/` is included into the compiled binary via
//! [`include_str!`] so the tool needs no runtime access to the source tree.
//! Templates are grouped by destination subdirectory; the
//! [`StaticFile`] struct pairs each `&'static str` with the relative path it
//! should be written to inside `exhibition_rt/`.

use crate::engine::addresses::DeviceKind;


/// A single embedded file destined for the generated output tree.
pub struct StaticFile {
    /// Path relative to `exhibition_rt/` where this file should be written.
    pub rel_path: &'static str,
    /// File contents.
    pub contents: &'static str,
}


/// Files copied verbatim into every RT project regardless of transport or
/// device mix: CRC helpers, the protocol layer, the system handler, and the
/// shared SysTick clock.
///
/// Per-peripheral device handlers are **not** here — they are emitted
/// selectively via [`device_handler_files`] so a project links only the
/// handlers it actually uses (an unused handler would reference HAL symbols
/// the `.ioc` never enabled, breaking the link).
pub const STATIC_FILES: &[StaticFile] = &[
    // common/
    StaticFile { rel_path: "common/crc.h",
        contents: include_str!("cpp/common/crc.h") },
    StaticFile { rel_path: "common/crc.cpp",
        contents: include_str!("cpp/common/crc.cpp") },
    StaticFile { rel_path: "common/helper_functions.h",
        contents: include_str!("cpp/common/helper_functions.h") },
    StaticFile { rel_path: "common/helper_functions.cpp",
        contents: include_str!("cpp/common/helper_functions.cpp") },

    // communication/
    StaticFile { rel_path: "communication/i_clock.h",
        contents: include_str!("cpp/communication/i_clock.h") },
    StaticFile { rel_path: "communication/i_frame_transport.h",
        contents: include_str!("cpp/communication/i_frame_transport.h") },
    StaticFile { rel_path: "communication/i_sub_address_handler.h",
        contents: include_str!("cpp/communication/i_sub_address_handler.h") },
    StaticFile { rel_path: "communication/i_bus_fault_observer.h",
        contents: include_str!("cpp/communication/i_bus_fault_observer.h") },
    StaticFile { rel_path: "communication/fault_counter.h",
        contents: include_str!("cpp/communication/fault_counter.h") },
    StaticFile { rel_path: "communication/message_header.h",
        contents: include_str!("cpp/communication/message_header.h") },
    StaticFile { rel_path: "communication/message_header.cpp",
        contents: include_str!("cpp/communication/message_header.cpp") },
    StaticFile { rel_path: "communication/scheduler.h",
        contents: include_str!("cpp/communication/scheduler.h") },
    StaticFile { rel_path: "communication/scheduler.cpp",
        contents: include_str!("cpp/communication/scheduler.cpp") },
    StaticFile { rel_path: "communication/sync.h",
        contents: include_str!("cpp/communication/sync.h") },
    StaticFile { rel_path: "communication/sync.cpp",
        contents: include_str!("cpp/communication/sync.cpp") },
    StaticFile { rel_path: "communication/offset_smoother.h",
        contents: include_str!("cpp/communication/offset_smoother.h") },
    StaticFile { rel_path: "communication/phase_anchor.h",
        contents: include_str!("cpp/communication/phase_anchor.h") },
    StaticFile { rel_path: "communication/clock_filter.h",
        contents: include_str!("cpp/communication/clock_filter.h") },
    StaticFile { rel_path: "communication/protocol_t1.md",
        contents: include_str!("../../../docs/protocol_t1.md") },

    // system/
    StaticFile { rel_path: "system/system_handler.h",
        contents: include_str!("cpp/system/system_handler.h") },
    StaticFile { rel_path: "system/system_handler.cpp",
        contents: include_str!("cpp/system/system_handler.cpp") },

    // transports/  (shared clock — copied unconditionally)
    StaticFile { rel_path: "transports/sys_tick_clock.h",
        contents: include_str!("cpp/transports/sys_tick_clock.h") },
];


/// The `.h`/`.cpp` pair implementing the handler for one [`DeviceKind`].
/// Returned by [`device_handler_files`] and written only when the project
/// actually uses that kind.
const GPIO_OUTPUT_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/gpio_output_handler.h",
        contents: include_str!("cpp/devices/gpio/gpio_output_handler.h") },
    StaticFile { rel_path: "devices/gpio_output_handler.cpp",
        contents: include_str!("cpp/devices/gpio/gpio_output_handler.cpp") },
];
const GPIO_INPUT_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/gpio_input_handler.h",
        contents: include_str!("cpp/devices/gpio/gpio_input_handler.h") },
    StaticFile { rel_path: "devices/gpio_input_handler.cpp",
        contents: include_str!("cpp/devices/gpio/gpio_input_handler.cpp") },
];
const ADC_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/adc_handler.h",
        contents: include_str!("cpp/devices/adc/adc_handler.h") },
    StaticFile { rel_path: "devices/adc_handler.cpp",
        contents: include_str!("cpp/devices/adc/adc_handler.cpp") },
];
const DAC_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/dac_handler.h",
        contents: include_str!("cpp/devices/dac/dac_handler.h") },
    StaticFile { rel_path: "devices/dac_handler.cpp",
        contents: include_str!("cpp/devices/dac/dac_handler.cpp") },
];
const PWM_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/pwm_handler.h",
        contents: include_str!("cpp/devices/pwm/pwm_handler.h") },
    StaticFile { rel_path: "devices/pwm_handler.cpp",
        contents: include_str!("cpp/devices/pwm/pwm_handler.cpp") },
];
const SPI_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/spi_handler.h",
        contents: include_str!("cpp/devices/spi/spi_handler.h") },
    StaticFile { rel_path: "devices/spi_handler.cpp",
        contents: include_str!("cpp/devices/spi/spi_handler.cpp") },
];
const UART_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/uart_handler.h",
        contents: include_str!("cpp/devices/uart/uart_handler.h") },
    StaticFile { rel_path: "devices/uart_handler.cpp",
        contents: include_str!("cpp/devices/uart/uart_handler.cpp") },
];
const I2C_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/i2c_handler.h",
        contents: include_str!("cpp/devices/i2c/i2c_handler.h") },
    StaticFile { rel_path: "devices/i2c_handler.cpp",
        contents: include_str!("cpp/devices/i2c/i2c_handler.cpp") },
];
const CAN_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/can_handler.h",
        contents: include_str!("cpp/devices/can/can_handler.h") },
    StaticFile { rel_path: "devices/can_handler.cpp",
        contents: include_str!("cpp/devices/can/can_handler.cpp") },
];
const QSPI_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/qspi_handler.h",
        contents: include_str!("cpp/devices/qspi/qspi_handler.h") },
    StaticFile { rel_path: "devices/qspi_handler.cpp",
        contents: include_str!("cpp/devices/qspi/qspi_handler.cpp") },
];
const RTC_HANDLER_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "devices/rtc_handler.h",
        contents: include_str!("cpp/devices/rtc/rtc_handler.h") },
    StaticFile { rel_path: "devices/rtc_handler.cpp",
        contents: include_str!("cpp/devices/rtc/rtc_handler.cpp") },
];


/// The handler source files for a single device kind. Codegen writes only
/// the files for kinds present in the resolved sub-address table, so an
/// unused peripheral never contributes a `.cpp` to the build.
pub fn device_handler_files(kind: DeviceKind) -> &'static [StaticFile] {
    match kind {
        DeviceKind::GpioOutput => GPIO_OUTPUT_HANDLER_FILES,
        DeviceKind::GpioInput  => GPIO_INPUT_HANDLER_FILES,
        DeviceKind::Adc        => ADC_HANDLER_FILES,
        DeviceKind::Dac        => DAC_HANDLER_FILES,
        DeviceKind::Pwm        => PWM_HANDLER_FILES,
        DeviceKind::Qspi       => QSPI_HANDLER_FILES,
        DeviceKind::Rtc        => RTC_HANDLER_FILES,
        DeviceKind::Spi        => SPI_HANDLER_FILES,
        DeviceKind::Uart       => UART_HANDLER_FILES,
        DeviceKind::I2c        => I2C_HANDLER_FILES,
        DeviceKind::Can        => CAN_HANDLER_FILES,
    }
}


/// UART-transport-specific files; included only when the chosen transport
/// is a UART.
pub const UART_TRANSPORT_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "transports/uart_frame_transport.h",
        contents: include_str!("cpp/transports/uart_frame_transport.h") },
    StaticFile { rel_path: "transports/uart_frame_transport.cpp",
        contents: include_str!("cpp/transports/uart_frame_transport.cpp") },
];


/// USB-CDC-transport-specific files; included only when the chosen
/// transport is a USB CDC virtual COM port.
pub const USB_CDC_TRANSPORT_FILES: &[StaticFile] = &[
    StaticFile { rel_path: "transports/usb_cdc_frame_transport.h",
        contents: include_str!("cpp/transports/usb_cdc_frame_transport.h") },
    StaticFile { rel_path: "transports/usb_cdc_frame_transport.cpp",
        contents: include_str!("cpp/transports/usb_cdc_frame_transport.cpp") },
];


/// Files that are copied verbatim into every generated BC (host) project.
///
/// The protocol layer (`common/crc`, `communication/*`) is the same bytes
/// the RT side uses — desktop-buildable, no STM32 HAL dependency — so we
/// reuse the on-target sources directly via `include_str!`. The `host/`
/// directory is BC-only: a desktop IClock and a serial IFrameTransport.
pub const BC_STATIC_FILES: &[StaticFile] = &[
    // common/  (shared bytes with RT)
    StaticFile { rel_path: "common/crc.h",
        contents: include_str!("cpp/common/crc.h") },
    StaticFile { rel_path: "common/crc.cpp",
        contents: include_str!("cpp/common/crc.cpp") },
    StaticFile { rel_path: "common/helper_functions.h",
        contents: include_str!("cpp/common/helper_functions.h") },
    StaticFile { rel_path: "common/helper_functions.cpp",
        contents: include_str!("cpp/common/helper_functions.cpp") },

    // communication/  (shared bytes with RT)
    StaticFile { rel_path: "communication/i_clock.h",
        contents: include_str!("cpp/communication/i_clock.h") },
    StaticFile { rel_path: "communication/i_frame_transport.h",
        contents: include_str!("cpp/communication/i_frame_transport.h") },
    StaticFile { rel_path: "communication/i_sub_address_handler.h",
        contents: include_str!("cpp/communication/i_sub_address_handler.h") },
    StaticFile { rel_path: "communication/i_bus_fault_observer.h",
        contents: include_str!("cpp/communication/i_bus_fault_observer.h") },
    StaticFile { rel_path: "communication/fault_counter.h",
        contents: include_str!("cpp/communication/fault_counter.h") },
    StaticFile { rel_path: "communication/message_header.h",
        contents: include_str!("cpp/communication/message_header.h") },
    StaticFile { rel_path: "communication/message_header.cpp",
        contents: include_str!("cpp/communication/message_header.cpp") },
    StaticFile { rel_path: "communication/scheduler.h",
        contents: include_str!("cpp/communication/scheduler.h") },
    StaticFile { rel_path: "communication/scheduler.cpp",
        contents: include_str!("cpp/communication/scheduler.cpp") },
    StaticFile { rel_path: "communication/sync.h",
        contents: include_str!("cpp/communication/sync.h") },
    StaticFile { rel_path: "communication/sync.cpp",
        contents: include_str!("cpp/communication/sync.cpp") },

    // host/  (BC-only desktop adapters)
    StaticFile { rel_path: "host/delay.h",
        contents: include_str!("cpp_bc/host/delay.h") },
    StaticFile { rel_path: "host/delay.cpp",
        contents: include_str!("cpp_bc/host/delay.cpp") },
    StaticFile { rel_path: "host/std_steady_clock.h",
        contents: include_str!("cpp_bc/host/std_steady_clock.h") },
    StaticFile { rel_path: "host/std_steady_clock.cpp",
        contents: include_str!("cpp_bc/host/std_steady_clock.cpp") },
    StaticFile { rel_path: "host/serial_frame_transport.h",
        contents: include_str!("cpp_bc/host/serial_frame_transport.h") },
    StaticFile { rel_path: "host/serial_frame_transport.cpp",
        contents: include_str!("cpp_bc/host/serial_frame_transport.cpp") },
    StaticFile { rel_path: "host/logger.h",
        contents: include_str!("cpp_bc/host/logger.h") },
    StaticFile { rel_path: "host/logger.cpp",
        contents: include_str!("cpp_bc/host/logger.cpp") },
    StaticFile { rel_path: "host/logging_frame_transport.h",
        contents: include_str!("cpp_bc/host/logging_frame_transport.h") },
    StaticFile { rel_path: "host/logging_frame_transport.cpp",
        contents: include_str!("cpp_bc/host/logging_frame_transport.cpp") },
];
