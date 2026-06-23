// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/devices/adc_handler.h"

#include "rt/command_ids.h"
#include "rt/common/helper_functions.h"

AdcHandler::AdcHandler(ADC_HandleTypeDef* hadc, uint8_t channel, const IClock& clock)
    : m_hadc(hadc)
    , m_channel(channel)
    , m_clock(clock)
{
}


Status AdcHandler::handle(const MessageHeader& cmd_header,
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
    switch (static_cast<AdcCmd>(op)) {
        case AdcCmd::Sample: {
            if (resp_buf_max < 6) {
                return Status::HandlerError;
            }
            if (HAL_ADC_Start(m_hadc) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_ADC_PollForConversion(m_hadc, 10) != HAL_OK) {
                HAL_ADC_Stop(m_hadc);
                return Status::HandlerError;
            }
            const uint32_t counts = HAL_ADC_GetValue(m_hadc);
            HAL_ADC_Stop(m_hadc);
            emplace_u16_le(resp_buf, counts);
            // Return synchronized timestamp, or systick if
            // not synchronized.
            emplace_u32_le(resp_buf + 2 , m_clock.now_bc_ms());
            resp_len_out = sizeof(uint16_t) + sizeof(uint32_t);
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
