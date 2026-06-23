// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef RTC_HANDLER_H
#define RTC_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"


/// @brief Sub-address handler for the on-board RTC (Real-Time Clock).
/// @details
/// Recognized opcodes:
///   - 0x01 SET_DATETIME : write date and time to the RTC.
///           Command payload is 7 bytes:
///             [0] year-offset (0-99, representing 2000 + N)
///             [1] month       (1-12)
///             [2] day         (1-31)
///             [3] weekday     (1=Monday … 7=Sunday, matching RTC_WEEKDAY_*)
///             [4] hours       (0-23, 24-hour mode)
///             [5] minutes     (0-59)
///             [6] seconds     (0-59)
///           Every field is range-checked before the calendar is written; an
///           out-of-range field is rejected with Status::HandlerError and a
///           single 0xE1 diagnostic byte, leaving the RTC unchanged.
///           Response payload is 0 bytes on success.
///   - 0x02 GET_DATETIME : read date and time from the RTC.
///           Command payload is 0 bytes.
///           Response payload is 7 bytes with the same layout as SET_DATETIME.
///
/// @note The STM32 HAL requires that HAL_RTC_GetTime() is called before
///       HAL_RTC_GetDate() to unlock the shadow registers. The implementation
///       always follows this order.
class RtcHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to a specific HAL RTC peripheral.
    /// @param hrtc HAL RTC handle (e.g. `&hrtc`). Must outlive *this*.
    explicit RtcHandler(RTC_HandleTypeDef* hrtc);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    RTC_HandleTypeDef* m_hrtc;
};


#endif // RTC_HANDLER_H
