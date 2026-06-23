// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef API_FINGERPRINT_H
#define API_FINGERPRINT_H

#include <stdint.h>

/// @brief 32-bit FNV-1a fingerprint of the generated API manifest.
/// @details Stamped here at generation time. The BC reads the
///          RT's runtime fingerprint via the system handler
///          (sub-address 1, opcode 0x01) and compares to confirm
///          both sides were built from the same .ioc.
constexpr uint32_t kApiFingerprint = 0xA084EED3u;

#endif // API_FINGERPRINT_H
