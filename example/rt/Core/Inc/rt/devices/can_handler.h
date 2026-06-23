// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef CAN_HANDLER_H
#define CAN_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "rt/communication/i_sub_address_handler.h"
#include "rt/communication/i_clock.h"


/// @brief Sub-address handler for one CAN bus.
/// @details
/// Only the classic CAN HAL (`HAL_CAN_*`) is supported in this MVP handler;
/// FDCAN peripherals are accepted by the parser but require a future variant.
/// Recognized opcodes:
///   - 0x01 TX_FRAME : payload `[op][id_be32][dlc][data...]`. Standard ID
///                     (11-bit) is assumed if id < 0x800, otherwise extended
///                     (29-bit). `dlc` is 0..8.
///   - 0x02 RX_FRAME : poll the RX FIFO0; if a frame is available, respond
///                     with `[id_be32][dlc][data8]` (13 bytes, padded by
///                     RT to size_14). If no frame is available, respond
///                     with a `dlc=0xFF` sentinel.
class CanHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL classic-CAN handle.
    /// @param hcan HAL CAN handle (e.g. `&hcan1`). Must outlive *this*.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    CanHandler(CAN_HandleTypeDef* hcan, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    CAN_HandleTypeDef* m_hcan;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // CAN_HANDLER_H
