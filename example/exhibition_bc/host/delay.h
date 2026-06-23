// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef DELAY_H
#define DELAY_H

#include <stdint.h>

/// @file delay.h
/// @brief Host-side blocking delay helper for BC test sequences.

/// @brief Block the calling thread for at least `milliseconds` ms.
/// @details Pure host-side sleep; sends no bus traffic. Use it to space out
///          operations in a timing sequence (e.g. between turning an output
///          on and off). Backed by std::this_thread::sleep_for, so it yields
///          the CPU rather than busy-waiting.
/// @param milliseconds Minimum number of milliseconds to wait.
void delay(uint32_t milliseconds);

#endif // DELAY_H
