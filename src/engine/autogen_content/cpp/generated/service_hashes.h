// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SERVICE_HASHES_H
#define SERVICE_HASHES_H

#include <stdint.h>

/// @file service_hashes.h
/// @brief Per-service (per-kind) API hashes for every sub-address kind.
/// @details Auto-generated from the Exhibition command catalog. Each value
///          is an FNV-1a hash over that kind's command vocabulary and is
///          returned at runtime by the universal GET_SERVICE_HASH command
///          (opcode 0x00) on every sub-address of that kind.

constexpr uint32_t kServiceHashSystem = 0xD695D532u;
constexpr uint32_t kServiceHashAdc = 0x051B7A24u;
constexpr uint32_t kServiceHashCan = 0x1AD19012u;
constexpr uint32_t kServiceHashDac = 0x5BFEBA42u;
constexpr uint32_t kServiceHashGpioIn = 0x8997616Cu;
constexpr uint32_t kServiceHashGpioOut = 0x6D212AAEu;
constexpr uint32_t kServiceHashI2c = 0xC6B66801u;
constexpr uint32_t kServiceHashPwm = 0x38A7A790u;
constexpr uint32_t kServiceHashQspi = 0xD5D53DECu;
constexpr uint32_t kServiceHashRtc = 0xF4096995u;
constexpr uint32_t kServiceHashSpi = 0xEF71E046u;
constexpr uint32_t kServiceHashUart = 0xEF71E046u;

#endif // SERVICE_HASHES_H
