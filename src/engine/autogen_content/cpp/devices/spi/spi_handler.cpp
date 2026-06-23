// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "spi_handler.h"

#include "../../generated/command_ids.h"
#include "../common/helper_functions.h"

static constexpr uint32_t kSpiTimeoutMs = 50;


SpiHandler::SpiHandler(SPI_HandleTypeDef* hspi, const IClock& clock)
    : m_hspi(hspi)
    , m_clock(clock)
{
}


Status SpiHandler::handle(const MessageHeader& cmd_header,
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
    switch (static_cast<SpiCmd>(op)) {
        case SpiCmd::Tx: {
            // TX cmd: [op, len_lo, len_hi, data...]. The explicit length
            // field lets the BC pad the cmd payload up to a valid
            // PayloadSizes slot without the pad bytes being transmitted on
            // the wire.
            if (cmd_len < 3u) {
                return Status::HandlerError;
            }
            const uint32_t tx_len = extract_u16_le(cmd_payload + 1);
            if (tx_len == 0u) {
                return Status::Ok;
            }
            if (tx_len > (cmd_len - 3u)) {
                return Status::HandlerError;
            }
            if (HAL_SPI_Transmit(m_hspi,
                                 const_cast<uint8_t*>(cmd_payload + 3),
                                 static_cast<uint16_t>(tx_len),
                                 kSpiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        case SpiCmd::Rx: {
            if (cmd_len < 3) {
                return Status::HandlerError;
            }
            const uint32_t rx_len = extract_u16_le(cmd_payload + 1);

            if (rx_len > resp_buf_max) {
                return Status::PayloadTooLarge;
            }
            if (HAL_SPI_Receive(m_hspi,
                                resp_buf,
                                static_cast<uint16_t>(rx_len),
                                kSpiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            resp_len_out = rx_len;
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
