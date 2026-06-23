// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "uart_frame_transport.h"

#include <cstring>

#include "../communication/message_header.h"


static constexpr uint32_t kTxTimeoutMs = 100;
static constexpr uint32_t kHeaderBytes = 6;


UartFrameTransport::UartFrameTransport(UART_HandleTypeDef* huart)
    : m_huart(huart)
    , m_rx_state(RxState::Header)
    , m_expected_payload(0)
    , m_rx_filled(0)
{
    std::memset(m_rx_buf, 0, sizeof(m_rx_buf));
}


int32_t UartFrameTransport::send(uint8_t const* bytes, uint32_t length)
{
    if (HAL_UART_Transmit(m_huart,
                          const_cast<uint8_t*>(bytes),
                          static_cast<uint16_t>(length),
                          kTxTimeoutMs) != HAL_OK) {
        return -1;
    }
    return static_cast<int32_t>(length);
}


void UartFrameTransport::reset_rx()
{
    m_rx_state         = RxState::Header;
    m_expected_payload = 0;
    m_rx_filled        = 0;
}


int32_t UartFrameTransport::receive(uint8_t* out, uint32_t max_length)
{
    const uint32_t needed =
        (m_rx_state == RxState::Header) ? kHeaderBytes : m_expected_payload;

    // Payload size parsed when a header completes in-loop below; reused after
    // the loop so the header is never deserialized twice.
    uint32_t hdr_payload = 0;

    // Drain any bytes already available from the UART without blocking.
    while (m_rx_filled < needed) {
        // Ask the UART for exactly the bytes still missing for this chunk in a
        // single transfer: the full header/payload when starting fresh, or a
        // single byte after a resync drop (see below). needed <=
        // sizeof(m_rx_buf), so the destination range is always in bounds.
        const uint16_t want =
            static_cast<uint16_t>(needed - m_rx_filled);
        const HAL_StatusTypeDef rs =
            HAL_UART_Receive(m_huart, m_rx_buf + m_rx_filled, want, 0);
        // With Timeout=0 the polling receive pulls every byte already in the
        // RX register back-to-back and returns HAL_TIMEOUT once it momentarily
        // runs dry; RxXferCount then holds the bytes it could not fill, so
        // (want - RxXferCount) is the count actually received (0..want).
        if (rs != HAL_OK && rs != HAL_TIMEOUT) {
            return -1;
        }
        const uint16_t got =
            static_cast<uint16_t>(want - m_huart->RxXferCount);
        if (got == 0) {
            // No byte available right now; report partial / nothing to caller.
            return 0;
        }
        m_rx_filled += got;

        // If we just completed a header, validate it. On a bad CRC we drop
        // only the oldest byte and slide the window, so a header that starts
        // mid-buffer can still resync (matches SerialFrameTransport on the
        // BC side). The next iteration re-requests just the missing byte.
        if (m_rx_state == RxState::Header && m_rx_filled == kHeaderBytes) {
            MessageHeader hdr;
            if (!hdr.deserialize(m_rx_buf)) {
                std::memmove(m_rx_buf, m_rx_buf + 1, kHeaderBytes - 1u);
                m_rx_filled = kHeaderBytes - 1u;
                continue;
            }
            hdr_payload = hdr.get_size_bytes();
            break;
        }
    }

    if (m_rx_filled < needed) {
        return 0;
    }
    if (needed > max_length) {
        return -1;
    }

    std::memcpy(out, m_rx_buf, needed);
    const int32_t ret = static_cast<int32_t>(needed);

    if (m_rx_state == RxState::Header) {
        if (hdr_payload > 0) {
            m_rx_state = RxState::Payload;
            m_expected_payload = hdr_payload;
        } else {
            m_rx_state = RxState::Header;
            m_expected_payload = 0;
        }
    } else {
        m_rx_state = RxState::Header;
        m_expected_payload = 0;
    }
    m_rx_filled = 0;
    return ret;
}
