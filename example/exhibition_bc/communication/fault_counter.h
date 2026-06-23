// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef FAULT_COUNTER_H
#define FAULT_COUNTER_H

#include <stdint.h>


/// @file fault_counter.h
/// @brief Shared message-fault counter used by the scheduler.
/// @details
/// A single `uint32_t` wrapped in a tiny value type so it can be
/// dependency-injected by reference into the parts of the system that
/// produce and consume it. The `RemoteTerminal` (and, with its own private
/// instance, the `BusController`) increments the counter every time a message
/// fails due to a timeout or a CRC / malformed-frame error. Application-level
/// handler faults are deliberately *not* counted here.
///
/// On the RT side one `FaultCounter` is shared between the `RemoteTerminal`
/// (which writes it) and the system handler (which reads it back out over the
/// wire), so the host can poll the RT's running fault total. Keeping it a
/// standalone object instead of a member of either class avoids coupling the
/// system handler to the scheduler type.
struct FaultCounter {
    /// @brief Running count of timeout / CRC / malformed message failures.
    uint32_t count = 0;

    /// @brief Record one message fault.
    void increment() { ++count; }

    /// @brief Read the current fault total.
    uint32_t value() const { return count; }
};


#endif // FAULT_COUNTER_H
