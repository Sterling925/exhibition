// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef I2C_HANDLER_H
#define I2C_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "rt/communication/i_sub_address_handler.h"
#include "rt/communication/i_clock.h"


/// @brief Sub-address handler for one I2C bus.
/// @details
/// Recognized opcodes:
///   - 0x01 WRITE_REG : payload `[op][dev_addr_7b][reg][data...]`. The
///                      handler shifts the device address left by one and
///                      calls `HAL_I2C_Mem_Write` with an 8-bit register
///                      address.
///   - 0x02 READ_REG  : payload `[op][dev_addr_7b][reg][len]`. Response is
///                      `len` bytes read via `HAL_I2C_Mem_Read`.
/// 10-bit addressing and 16-bit register addressing are out of MVP scope.
class I2cHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a HAL I2C handle.
    /// @param hi2c HAL I2C handle (e.g. `&hi2c1`). Must outlive *this*.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    I2cHandler(I2C_HandleTypeDef* hi2c, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    I2C_HandleTypeDef* m_hi2c;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // I2C_HANDLER_H
