// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "can_handler.h"

#include "../../generated/command_ids.h"

#include <cstring>


CanHandler::CanHandler(CAN_HandleTypeDef* hcan, const IClock& clock)
    : m_hcan(hcan)
    , m_clock(clock)
{
}


Status CanHandler::handle(const MessageHeader& cmd_header,
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
    switch (static_cast<CanCmd>(op)) {
        case CanCmd::TxFrame: {
            // [op][id_be32][dlc][data...]  -> minimum 6 bytes (dlc may be 0).
            if (cmd_len < 6) {
                return Status::HandlerError;
            }
            const uint32_t id =
                  (static_cast<uint32_t>(cmd_payload[1]) << 24)
                | (static_cast<uint32_t>(cmd_payload[2]) << 16)
                | (static_cast<uint32_t>(cmd_payload[3]) << 8)
                |  static_cast<uint32_t>(cmd_payload[4]);
            uint8_t dlc = cmd_payload[5];
            if (dlc > 8) {
                dlc = 8;
            }
            if (cmd_len < static_cast<uint32_t>(6u + dlc)) {
                return Status::HandlerError;
            }

            CAN_TxHeaderTypeDef tx_header{};
            if (id < 0x800u) {
                tx_header.StdId = id;
                tx_header.ExtId = 0;
                tx_header.IDE   = CAN_ID_STD;
            } else {
                tx_header.StdId = 0;
                tx_header.ExtId = id;
                tx_header.IDE   = CAN_ID_EXT;
            }
            tx_header.RTR              = CAN_RTR_DATA;
            tx_header.DLC              = dlc;
            tx_header.TransmitGlobalTime = DISABLE;

            uint8_t data[8] = { 0 };
            std::memcpy(data, cmd_payload + 6, dlc);

            uint32_t mailbox = 0;
            if (HAL_CAN_AddTxMessage(m_hcan, &tx_header, data, &mailbox) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        case CanCmd::RxFrame: {
            if (resp_buf_max < 13) {
                return Status::HandlerError;
            }
            if (HAL_CAN_GetRxFifoFillLevel(m_hcan, CAN_RX_FIFO0) == 0) {
                std::memset(resp_buf, 0, 13);
                resp_buf[4] = 0xFF; // sentinel dlc
                resp_len_out = 13;
                return Status::Ok;
            }
            CAN_RxHeaderTypeDef rx_header{};
            uint8_t data[8] = { 0 };
            if (HAL_CAN_GetRxMessage(m_hcan, CAN_RX_FIFO0, &rx_header, data) != HAL_OK) {
                return Status::HandlerError;
            }
            const uint32_t id = (rx_header.IDE == CAN_ID_EXT)
                ? rx_header.ExtId
                : rx_header.StdId;
            resp_buf[0] = static_cast<uint8_t>((id >> 24) & 0xFFu);
            resp_buf[1] = static_cast<uint8_t>((id >> 16) & 0xFFu);
            resp_buf[2] = static_cast<uint8_t>((id >> 8)  & 0xFFu);
            resp_buf[3] = static_cast<uint8_t>( id        & 0xFFu);
            resp_buf[4] = static_cast<uint8_t>(rx_header.DLC);
            std::memcpy(resp_buf + 5, data, 8);
            resp_len_out = 13;
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
