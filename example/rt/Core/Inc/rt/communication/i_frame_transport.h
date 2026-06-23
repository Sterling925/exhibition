// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef I_FRAME_TRANSPORT_H
#define I_FRAME_TRANSPORT_H

#include <stdint.h>


/// @brief Abstract half-duplex frame transport injected into the Protocol T1
///        scheduler.
/// @details
/// A "frame" here is one logical wire chunk -- either a 6-byte serialized
/// MessageHeader or a payload of declared size. Both directions are
/// non-blocking. The scheduler polls receive() from its tick() loop and
/// honours the return codes documented below.
class IFrameTransport {
    public:

    virtual ~IFrameTransport() = default;

    /// @brief Send one frame chunk.
    /// @param bytes Buffer holding the chunk.
    /// @param length Number of bytes in the chunk.
    /// @return Bytes written on success (==length), 0 if the link is busy /
    ///         would block, -1 on hard error.
    virtual int32_t send(uint8_t const* bytes, uint32_t length) = 0;

    /// @brief Receive at most one frame chunk.
    /// @param out Destination buffer.
    /// @param max_length Capacity of out in bytes.
    /// @return Bytes copied into out (>0) on success, 0 if nothing is
    ///         waiting, -1 on hard error.
    virtual int32_t receive(uint8_t* out, uint32_t max_length) = 0;

    /// @brief Abort any in-progress receive and return the framer to hunting
    ///        for a fresh 6-byte header.
    /// @details
    /// The scheduler calls this when it abandons a frame (timeout / transport
    /// error / CRC failure / inbound fault) so transport framing cannot desync
    /// from protocol state. It does not touch the send path and does not flush
    /// the underlying byte source -- any residual bytes are realigned by the
    /// header sliding-window CRC resync inside receive().
    virtual void reset_rx() = 0;
};


#endif // I_FRAME_TRANSPORT_H
