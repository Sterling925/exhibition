// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef UART_HANDLER_H
#define UART_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for one UART/USART bus (as an addressable
///        device, not the RT's own transport).
/// @details
/// Recognized opcodes:
///   - 0x01 TX : remaining cmd payload bytes are transmitted with
///               `HAL_UART_Transmit`. No response payload.
///   - 0x02 RX : payload is a single uint16 (little-endian) byte count.
///               Response is that many bytes received with
///               `HAL_UART_Receive`.
class UartHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL UART handle.
    /// @param huart HAL UART handle (e.g. `&huart1`). Must outlive *this*.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    UartHandler(UART_HandleTypeDef* huart, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    UART_HandleTypeDef* m_huart;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // UART_HANDLER_H
