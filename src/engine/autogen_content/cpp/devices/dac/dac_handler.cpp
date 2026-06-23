// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "dac_handler.h"

#include "../../generated/command_ids.h"
#include "../common/helper_functions.h"

DacHandler::DacHandler(DAC_HandleTypeDef* hdac, uint32_t channel, const IClock& clock)
    : m_hdac(hdac)
    , m_channel(channel)
    , m_clock(clock)
{
}


Status DacHandler::handle(const MessageHeader& cmd_header,
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
    switch (static_cast<DacCmd>(op)) {
        case DacCmd::SetCounts: {
            if (cmd_header.get_size_bytes() < 3) {
                return Status::HandlerError;
            }

            const uint32_t counts = extract_u16_le(cmd_payload + 1);

            if (HAL_DAC_SetValue(m_hdac, m_channel, DAC_ALIGN_12B_R, counts) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_DAC_Start(m_hdac, m_channel) != HAL_OK) {
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
