// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SPI_HANDLER_H
#define SPI_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for one SPI bus.
/// @details
/// Recognized opcodes:
///   - 0x01 TX  : remaining cmd payload bytes are transmitted with
///                `HAL_SPI_Transmit`. No response payload.
///   - 0x02 RX  : payload is a single uint16 (little-endian) byte count to
///                read. Response is that many bytes received with
///                `HAL_SPI_Receive`.
/// Chip-select control is not exposed in this MVP handler. If a board needs
/// CS toggling, drive the CS GPIO as a separate sub-address before/after.
class SpiHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL SPI handle.
    /// @param hspi HAL SPI handle (e.g. `&hspi1`). Must outlive *this*.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    SpiHandler(SPI_HandleTypeDef* hspi, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    SPI_HandleTypeDef* m_hspi;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // SPI_HANDLER_H
