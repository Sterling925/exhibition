// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef API_FINGERPRINT_H
#define API_FINGERPRINT_H

#include <stdint.h>

/// @file api_fingerprint.h
/// @brief Host-test placeholder for the per-project API fingerprint.
/// @details The real `api_fingerprint.h` is generated per project by the
///          Exhibition codegen (see `engine/codegen.rs`), which stamps the FNV
///          hash of the command/sub-address manifest. This in-tree stub exists
///          only so the static handler templates (e.g. `system_handler.cpp`,
///          which references `kApiFingerprint`) can be compiled and linked by
///          the host unit tests under `tests/`. It is NOT a `STATIC_FILES`
///          entry and is never copied into a generated project.
constexpr uint32_t kApiFingerprint = 0u;

#endif // API_FINGERPRINT_H
