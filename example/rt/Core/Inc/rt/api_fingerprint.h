// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef API_FINGERPRINT_H
#define API_FINGERPRINT_H

#include <stdint.h>

/// @brief 32-bit FNV-1a fingerprint of the generated API manifest.
/// @details Regenerated each time `exhibition` runs. The BC can
///          read this value via the system handler (sub-address 1,
///          opcode 0x01) and compare against its own copy.
constexpr uint32_t kApiFingerprint = 0xA084EED3u;

#endif // API_FINGERPRINT_H
