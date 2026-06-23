// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "pwm_handler.h"

#include "../../generated/command_ids.h"
#include "../common/helper_functions.h"

PwmHandler::PwmHandler(TIM_HandleTypeDef* htim, uint32_t channel, const IClock& clock)
    : m_htim(htim)
    , m_channel(channel)
    , m_clock(clock)
{
}


Status PwmHandler::handle(const MessageHeader& cmd_header,
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
    switch (static_cast<PwmCmd>(op)) {
        case PwmCmd::SetDuty: {
            if (cmd_header.get_size_bytes() < 3) {
                return Status::HandlerError;
            }
            const uint32_t duty_x100 = extract_u16_le(cmd_payload + 1);
            if (duty_x100 > 10000u) {
                return Status::HandlerError;
            }
            // Period is ARR; pulse counts span [0, period]. Add 1 because the
            // timer counts from 0 to ARR inclusive.
            const uint32_t period = __HAL_TIM_GET_AUTORELOAD(m_htim);
            const uint64_t span = static_cast<uint64_t>(period) + 1u;
            const uint32_t pulse =
                static_cast<uint32_t>((span * duty_x100) / 10000u);
            __HAL_TIM_SET_COMPARE(m_htim, m_channel, pulse);
            if (HAL_TIM_PWM_Start(m_htim, m_channel) != HAL_OK) {
                return Status::HandlerError;
            }
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
