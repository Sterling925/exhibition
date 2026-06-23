// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef UART_FRAME_TRANSPORT_H
#define UART_FRAME_TRANSPORT_H

#include <stdint.h>

#include "main.h"
#include "../communication/i_frame_transport.h"
#include "../communication/scheduler.h"


/// @brief Polling-based IFrameTransport over a HAL UART.
/// @details
/// Frame chunks are reconstructed by peeking the size field out of each
/// received header. Each `receive()` pulls up to the bytes still missing for
/// the current chunk in a single non-blocking `HAL_UART_Receive(timeout=0)`
/// (the full 6-byte header or declared payload when starting fresh; a single
/// byte after a bad-header-CRC resync drop). The upper layer can call
/// `receive()` repeatedly until a full chunk is available. Transmits use
/// blocking `HAL_UART_Transmit`.
///
/// This MVP transport assumes:
///   - The HAL UART has been configured for polled / interrupt RX before
///     the transport runs (no DMA).
///   - The peer sends each chunk back-to-back without internal byte gaps.
///     A partial/dropped payload is recovered when the scheduler abandons the
///     frame (timeout / error) and calls reset_rx(), which resyncs the framer
///     to header hunting.
class UartFrameTransport : public IFrameTransport {
    public:

    /// @brief Construct bound to a HAL UART handle.
    /// @param huart HAL UART handle (e.g. `&huart3`). Must outlive *this*.
    explicit UartFrameTransport(UART_HandleTypeDef* huart);

    /// @brief Send one frame chunk via blocking HAL transmit.
    int32_t send(uint8_t const* bytes, uint32_t length) override;

    /// @brief Receive at most one frame chunk. Non-blocking.
    int32_t receive(uint8_t* out, uint32_t max_length) override;

    /// @brief Abort an in-progress receive and resync to header hunting.
    /// @details Called by the scheduler when it abandons a frame; see
    ///          IFrameTransport::reset_rx().
    void reset_rx() override;

    private:

    /// @brief What kind of chunk we are currently collecting from the wire.
    enum class RxState : uint8_t {
        Header,    ///< Next 6 bytes form a MessageHeader.
        Payload,   ///< Next m_expected_payload bytes form a payload.
    };

    UART_HandleTypeDef* m_huart;
    RxState             m_rx_state;
    uint32_t            m_expected_payload;
    uint8_t             m_rx_buf[kMaxPayloadBytes];
    uint32_t            m_rx_filled;
};


#endif // UART_FRAME_TRANSPORT_H
