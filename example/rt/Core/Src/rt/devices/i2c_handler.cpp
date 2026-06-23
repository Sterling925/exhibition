// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/devices/i2c_handler.h"

#include "rt/command_ids.h"


static constexpr uint32_t kI2cTimeoutMs = 50;


I2cHandler::I2cHandler(I2C_HandleTypeDef* hi2c, const IClock& clock)
    : m_hi2c(hi2c)
    , m_clock(clock)
{
}


Status I2cHandler::handle(const MessageHeader& cmd_header,
                          uint8_t const* cmd_payload,
                          uint8_t* resp_buf,
                          uint32_t resp_buf_max,
                          uint32_t& resp_len_out)
{
    resp_len_out = 0;

    const uint32_t cmd_len = cmd_header.get_size_bytes();
    if (cmd_len < 1 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE0;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint8_t op = cmd_payload[0];
    switch (static_cast<I2cCmd>(op)) {
        case I2cCmd::WriteReg: {
            // WRITE_REG cmd: [op, addr7, reg, len, data...]. The explicit
            // length field lets the BC pad the cmd payload up to a valid
            // PayloadSizes slot without the pad bytes being written to the
            // device.
            if (cmd_len < 4u) {
                return Status::HandlerError;
            }
            const uint16_t dev_addr =
                static_cast<uint16_t>(cmd_payload[1]) << 1;
            const uint16_t reg = static_cast<uint16_t>(cmd_payload[2]);
            const uint32_t data_len = static_cast<uint32_t>(cmd_payload[3]);
            if (data_len > (cmd_len - 4u)) {
                return Status::HandlerError;
            }
            if (HAL_I2C_Mem_Write(m_hi2c,
                                  dev_addr,
                                  reg,
                                  I2C_MEMADD_SIZE_8BIT,
                                  const_cast<uint8_t*>(cmd_payload + 4),
                                  static_cast<uint16_t>(data_len),
                                  kI2cTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        case I2cCmd::ReadReg: {
            if (cmd_len < 4) {
                return Status::HandlerError;
            }
            const uint16_t dev_addr =
                static_cast<uint16_t>(cmd_payload[1]) << 1;
            const uint16_t reg = static_cast<uint16_t>(cmd_payload[2]);
            const uint32_t len = static_cast<uint32_t>(cmd_payload[3]);
            if (len > resp_buf_max) {
                return Status::PayloadTooLarge;
            }
            if (HAL_I2C_Mem_Read(m_hi2c,
                                 dev_addr,
                                 reg,
                                 I2C_MEMADD_SIZE_8BIT,
                                 resp_buf,
                                 static_cast<uint16_t>(len),
                                 kI2cTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            resp_len_out = len;
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
