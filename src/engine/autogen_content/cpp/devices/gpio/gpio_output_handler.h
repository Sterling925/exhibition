// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef GPIO_OUTPUT_HANDLER_H
#define GPIO_OUTPUT_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for every GPIO output pin in one bank.
/// @details
/// Bound at construction time to a HAL port (`GPIOA`, `GPIOB`, ...) and a
/// 16-bit mask whose set bits identify the pin indices in the bank that
/// the IOC configured as outputs. The command payload is two bytes:
///   - `cmd_payload[0]` opcode:
///       - 0x01 SET   : drive the selected pin high. No response payload.
///       - 0x02 CLEAR : drive the selected pin low.  No response payload.
///   - `cmd_payload[1]` pin index 0..15 within this bank.
///
/// Fault one-byte responses:
///   - 0xE0 : payload shorter than 2 bytes (malformed request).
///   - 0xE1 : pin index out of range (> 15).
///   - 0xE2 : pin index valid but not present in the configured mask
///            (pin not wired up on this bank).
///   - any other byte : echoed unknown opcode.
class GpioOutputBankHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to the HAL port and the set of configured
    ///        output pins.
    /// @param port HAL port (`GPIOA`, `GPIOB`, ...). Must outlive *this*.
    /// @param configured_pin_mask Bit i set means pin i in this bank
    ///        is configured as an output and may be addressed.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    GpioOutputBankHandler(GPIO_TypeDef* port, uint16_t configured_pin_mask, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    GPIO_TypeDef* m_port;
    uint16_t      m_pin_mask;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // GPIO_OUTPUT_HANDLER_H
