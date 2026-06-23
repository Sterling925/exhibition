// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef DEVICE_REGISTRY_H
#define DEVICE_REGISTRY_H

/// @file device_registry.h
/// @brief Entry points into the auto-generated RemoteTerminal.
/// @details
/// Call `init_rt()` once at startup after the HAL is initialized.
/// Call `tick_rt()` from the main loop (or a timer task) as often
/// as you can afford; each call advances the RT state machine by
/// at most one step. Both are non-blocking.

#ifdef __cplusplus
extern "C" {
#endif

void init_rt();
void tick_rt();

#ifdef __cplusplus
}
#endif

#endif // DEVICE_REGISTRY_H
