// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef PWM_HANDLER_H
#define PWM_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for one PWM channel.
/// @details
/// Recognized opcodes:
///   - 0x01 SET_DUTY : payload is uint16 (little-endian) duty in hundredths
///                     of a percent (0..10000). The handler converts the
///                     value into compare-register counts using the timer's
///                     current period, then starts the PWM channel.
class PwmHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL timer handle and a channel constant.
    /// @param htim HAL timer handle (e.g. `&htim2`). Must outlive *this*.
    /// @param channel HAL channel constant (e.g. `TIM_CHANNEL_1`).
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    PwmHandler(TIM_HandleTypeDef* htim, uint32_t channel, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    TIM_HandleTypeDef* m_htim;
    uint32_t           m_channel;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // PWM_HANDLER_H
