// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/devices/rtc_handler.h"

#include "rt/command_ids.h"


RtcHandler::RtcHandler(RTC_HandleTypeDef* hrtc)
    : m_hrtc(hrtc)
{
}


Status RtcHandler::handle(const MessageHeader& cmd_header,
                          uint8_t const* cmd_payload,
                          uint8_t* resp_buf,
                          uint32_t resp_buf_max,
                          uint32_t& resp_len_out)
{
    resp_len_out = 0;

    if (cmd_header.get_size_bytes() < 1 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE0;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint8_t op = cmd_payload[0];
    switch (static_cast<RtcCmd>(op)) {
        case RtcCmd::SetDatetime: {
            if (cmd_header.get_size_bytes() < 8) {
                return Status::HandlerError;
            }

            const uint8_t year_offset = cmd_payload[1];
            const uint8_t month       = cmd_payload[2];
            const uint8_t day         = cmd_payload[3];
            const uint8_t weekday     = cmd_payload[4];
            const uint8_t hours       = cmd_payload[5];
            const uint8_t minutes     = cmd_payload[6];
            const uint8_t seconds     = cmd_payload[7];

            // Reject out-of-range fields before touching the calendar. Per
            // RM0090 (RTC_DR/RTC_TR) weekday 0 is forbidden, and the BCD
            // encoders silently corrupt the calendar for any field outside
            // these ranges when USE_FULL_ASSERT is disabled. Respond with the
            // 0xE1 diagnostic byte so the bus controller can log the reason.
            const bool in_range =
                year_offset <= 99 &&
                month   >= 1 && month   <= 12 &&
                day     >= 1 && day     <= 31 &&
                weekday >= 1 && weekday <= 7  &&
                hours   <= 23 &&
                minutes <= 59 &&
                seconds <= 59;
            if (!in_range) {
                if (resp_buf_max >= 1) {
                    resp_buf[0] = 0xE1;
                    resp_len_out = 1;
                }
                return Status::HandlerError;
            }

            RTC_DateTypeDef date = {};
            date.Year    = year_offset;
            date.Month   = month;
            date.Date    = day;
            date.WeekDay = weekday;

            RTC_TimeTypeDef time = {};
            time.Hours   = hours;
            time.Minutes = minutes;
            time.Seconds = seconds;

            if (HAL_RTC_SetDate(m_hrtc, &date, RTC_FORMAT_BIN) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_RTC_SetTime(m_hrtc, &time, RTC_FORMAT_BIN) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        case RtcCmd::GetDatetime: {
            if (resp_buf_max < 7) {
                return Status::HandlerError;
            }
            RTC_TimeTypeDef time = {};
            RTC_DateTypeDef date = {};
            // HAL requires GetTime before GetDate to unlock shadow registers.
            if (HAL_RTC_GetTime(m_hrtc, &time, RTC_FORMAT_BIN) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_RTC_GetDate(m_hrtc, &date, RTC_FORMAT_BIN) != HAL_OK) {
                return Status::HandlerError;
            }
            resp_buf[0] = date.Year;
            resp_buf[1] = date.Month;
            resp_buf[2] = date.Date;
            resp_buf[3] = date.WeekDay;
            resp_buf[4] = time.Hours;
            resp_buf[5] = time.Minutes;
            resp_buf[6] = time.Seconds;
            resp_len_out = 7;
            return Status::Ok;
        }
        default:
            if (resp_buf_max >= 1) {
                resp_buf[0] = op;
                resp_len_out = 1;
            }
            return Status::HandlerError;
    }
}
