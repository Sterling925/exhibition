// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef DAC_HANDLER_H
#define DAC_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for one DAC channel.
/// @details
/// Recognized opcodes:
///   - 0x01 SET_COUNTS : payload is uint16 (little-endian) DAC counts; the
///                       handler writes the value with `DAC_ALIGN_12B_R`.
///                       No response payload.
class DacHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL DAC handle and a channel constant.
    /// @param hdac HAL DAC handle (e.g. `&hdac1`). Must outlive *this*.
    /// @param channel HAL channel constant (e.g. `DAC_CHANNEL_1`).
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    DacHandler(DAC_HandleTypeDef* hdac, uint32_t channel, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    DAC_HandleTypeDef* m_hdac;
    uint32_t           m_channel;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // DAC_HANDLER_H
