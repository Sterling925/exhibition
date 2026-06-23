// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef ADC_HANDLER_H
#define ADC_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "rt/communication/i_sub_address_handler.h"
#include "rt/communication/i_clock.h"


/// @brief Sub-address handler for one ADC channel.
/// @details
/// Recognized opcodes:
///   - 0x01 SAMPLE : trigger a single conversion. Response is 6 bytes:
///                   uint16 raw counts little-endian (bytes 0-1), then
///                   uint32 BC-synchronized millisecond timestamp
///                   little-endian (bytes 2-5).
/// Multi-channel ADCs share one peripheral handle; if more than one channel
/// from the same `ADC_HandleTypeDef` is exposed, the host is responsible for
/// not interleaving samples (or for reconfiguring the channel before each
/// sample by extending this handler).
class AdcHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a specific HAL peripheral.
    /// @param hadc HAL ADC handle (e.g. `&hadc1`). Must outlive *this*.
    /// @param channel ADC channel number (e.g. 3 for ADC1_IN3). Carried for
    ///        introspection only; not currently consumed in the sample path.
    /// @param clock Read-only BC-synchronized clock; used to stamp each sample response. Must outlive *this*.
    AdcHandler(ADC_HandleTypeDef* hadc, uint8_t channel, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    ADC_HandleTypeDef* m_hadc;
    uint8_t            m_channel;

    /// @brief BC-synchronized clock; stamped into every SAMPLE response.
    const IClock& m_clock;
};


#endif // ADC_HANDLER_H
