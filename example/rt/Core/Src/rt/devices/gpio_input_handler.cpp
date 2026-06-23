// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/devices/gpio_input_handler.h"

#include "rt/command_ids.h"


GpioInputBankHandler::GpioInputBankHandler(GPIO_TypeDef* port,
                                           uint16_t configured_pin_mask,
                                           const IClock& clock)
    : m_port(port)
    , m_pin_mask(configured_pin_mask)
    , m_clock(clock)
{
}


Status GpioInputBankHandler::handle(const MessageHeader& cmd_header,
                                    uint8_t const* cmd_payload,
                                    uint8_t* resp_buf,
                                    uint32_t resp_buf_max,
                                    uint32_t& resp_len_out)
{
    resp_len_out = 0;

    if (cmd_header.get_size_bytes() < 2 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE0;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint8_t op      = cmd_payload[0];
    const uint8_t pin_idx = cmd_payload[1];

    if (pin_idx > 15) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE1;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint16_t pin_bit = static_cast<uint16_t>(1u << pin_idx);
    if ((pin_bit & m_pin_mask) == 0u) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE2;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    switch (static_cast<GpioInputCmd>(op)) {
        case GpioInputCmd::Read: {
            if (resp_buf_max < 1) {
                return Status::HandlerError;
            }
            const GPIO_PinState state = HAL_GPIO_ReadPin(m_port, pin_bit);
            resp_buf[0] = (state == GPIO_PIN_SET) ? 1 : 0;
            resp_len_out = 1;
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
