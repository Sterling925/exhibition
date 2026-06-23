// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef COMMAND_IDS_H
#define COMMAND_IDS_H

#include <stdint.h>

/// @file command_ids.h
/// @brief Command opcode enums for every sub-address kind.
/// @details Auto-generated from the Exhibition command catalog. Each
///          enumerator is the on-wire opcode byte; switch on the matching
///          enum in a handler instead of using raw literals.

/// @brief Command opcodes for system.
enum class SystemCmd : uint8_t {
    GetServiceHash = 0x00,
    GetFingerprint = 0x01,
    GetDeviceCount = 0x02,
    GetSysTick = 0x03,
    GetCoreDeviceId = 0x04,
    GetCpuId = 0x05,
    SyncProbe = 0x06,
    SyncFinal = 0x07,
    SyncAdjust = 0x08,
    GetMessageFaultCounter = 0x09,
    SoftReset = 0x0A,
    ReadMemory = 0x0B,
};

/// @brief Command opcodes for adc.
enum class AdcCmd : uint8_t {
    GetServiceHash = 0x00,
    Sample = 0x01,
};

/// @brief Command opcodes for can.
enum class CanCmd : uint8_t {
    GetServiceHash = 0x00,
    TxFrame = 0x01,
    RxFrame = 0x02,
};

/// @brief Command opcodes for dac.
enum class DacCmd : uint8_t {
    GetServiceHash = 0x00,
    SetCounts = 0x01,
};

/// @brief Command opcodes for gpio_in.
enum class GpioInputCmd : uint8_t {
    GetServiceHash = 0x00,
    Read = 0x01,
};

/// @brief Command opcodes for gpio_out.
enum class GpioOutputCmd : uint8_t {
    GetServiceHash = 0x00,
    Set = 0x01,
    Clear = 0x02,
};

/// @brief Command opcodes for i2c.
enum class I2cCmd : uint8_t {
    GetServiceHash = 0x00,
    WriteReg = 0x01,
    ReadReg = 0x02,
};

/// @brief Command opcodes for pwm.
enum class PwmCmd : uint8_t {
    GetServiceHash = 0x00,
    SetDuty = 0x01,
};

/// @brief Command opcodes for qspi.
enum class QspiCmd : uint8_t {
    GetServiceHash = 0x00,
    Read = 0x01,
    Program = 0x02,
    Erase = 0x03,
    ReadStatus = 0x04,
    WriteEnable = 0x05,
};

/// @brief Command opcodes for rtc.
enum class RtcCmd : uint8_t {
    GetServiceHash = 0x00,
    SetDatetime = 0x01,
    GetDatetime = 0x02,
};

/// @brief Command opcodes for spi.
enum class SpiCmd : uint8_t {
    GetServiceHash = 0x00,
    Tx = 0x01,
    Rx = 0x02,
};

/// @brief Command opcodes for uart.
enum class UartCmd : uint8_t {
    GetServiceHash = 0x00,
    Tx = 0x01,
    Rx = 0x02,
};

#endif // COMMAND_IDS_H
