// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef I_BUS_FAULT_OBSERVER_H
#define I_BUS_FAULT_OBSERVER_H

#include <stdint.h>

#include "i_sub_address_handler.h"   // for Status


/// @brief Observer notified by BusController whenever a transaction attempt
///        fails (timeout, CRC / malformed frame, or a remote fault frame).
/// @details
/// Injected into BusController so a host build can surface every failed attempt
/// to a log — including attempts that are immediately recovered by the single
/// automatic retry, which would otherwise leave no trace except the running
/// fault counter. The interface is pure-virtual and depends only on the shared
/// Status enum, so the on-target RT build can compile it (and simply pass
/// nullptr, opting out) without pulling in any host-side logging code.
///
/// on_fault() is called from inside BusController::tick(); implementations must
/// not call tick() and should return quickly.
class IBusFaultObserver {
    public:

    virtual ~IBusFaultObserver() = default;

    /// @brief Report a single failed transaction attempt.
    /// @param sub_addr Sub-address of the affected transaction.
    /// @param status Reason the attempt failed (Timeout, CrcFail*, RemoteFault,
    ///        TransportError, ...).
    /// @param will_retry true if the BC will automatically retry this attempt;
    ///        false if the failure is terminal (non-retryable, or the single
    ///        retry has already been spent).
    virtual void on_fault(uint8_t sub_addr, Status status, bool will_retry) = 0;
};


#endif // I_BUS_FAULT_OBSERVER_H
