// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef I_SUB_ADDRESS_HANDLER_H
#define I_SUB_ADDRESS_HANDLER_H

#include <stdint.h>

#include "rt/communication/message_header.h"


/// @brief Status codes shared between BC and RT.
/// @details
/// Defined here so handler implementations can return them without pulling in
/// the full scheduler.h. The enum is repeated (as an alias) inside
/// scheduler.h so callers that include only scheduler.h still see it.
enum class Status : uint8_t {
    Ok = 0,                 ///< Transaction completed normally.
    QueueFull,              ///< BC submit rejected: priority queue saturated.
    InvalidSubAddress,      ///< Sub-address 0 or out of range.
    PayloadTooLarge,        ///< Request payload exceeds 2048 B.
    Timeout,                ///< Peer did not respond within the per-frame deadline.
    RemoteFault,            ///< Peer set the fault bit on ack/response.
    CrcFailHeader,          ///< Received header failed its own CRC.
    CrcFailPayload,         ///< Received payload's CRC did not match its header.
    TransportError,         ///< IFrameTransport returned -1.
    HandlerError,           ///< RT-side: ISubAddressHandler returned non-Ok.
};


/// @brief Namespace of a fault payload's reason code (byte 0 of a fault frame).
/// @details
/// A fault frame's payload is self-describing: byte 0 names the domain that
/// byte 1.. should be interpreted in, so a receiver dispatches on the domain
/// instead of assuming every fault byte is a Status value. Defined here
/// alongside Status for the same reason (shared BC/RT vocabulary, reachable
/// without pulling in the full scheduler.h).
///
/// Wire layout of a fault payload:
///   Byte 0    : FaultDomain tag.
///   Byte 1    : reason code within that domain (a Status for Protocol).
///   Byte 2..N : domain-specific detail (handler faults only).
enum class FaultDomain : uint8_t {
    Protocol = 0,   ///< Byte 1 is a Status value (RT protocol-level fault).
    Hal      = 1,   ///< Reserved: byte 1.. is a HAL/driver code (not yet emitted).
    App      = 2,   ///< Byte 1.. is application-defined handler fault detail.
};


/// @brief Per-sub-address handler interface invoked by the Protocol T1
///        RemoteTerminal.
/// @details
/// RT calls handle() once a command header and its payload (if any) have
/// passed CRC checks. The handler fills resp_buf with response bytes (for
/// read commands) and writes the byte count into resp_len_out.
///
/// A return value of Status::Ok causes RT to emit a normal ack/response.
/// Any other return value causes RT to emit a fault frame. The RT prepends a
/// FaultDomain::App tag byte ahead of the first resp_len_out bytes of resp_buf,
/// so on the wire the fault payload is { App, <handler bytes...> }; the
/// handler's bytes are the App-domain detail. (Note: the prepended tag can push
/// the frame up one payload-size slot.)
class ISubAddressHandler {
    public:

    virtual ~ISubAddressHandler() = default;

    /// @brief Process a validated command.
    /// @param cmd_header Header of the inbound command (CRC already checked).
    /// @param cmd_payload Pointer to the command payload bytes; valid only
    ///        for the duration of the call. Length is
    ///        cmd_header.get_size_bytes().
    /// @param resp_buf RT-owned buffer to write the response into.
    /// @param resp_buf_max Capacity of resp_buf.
    /// @param resp_len_out Bytes written into resp_buf (0 for write-only
    ///        commands).
    /// @return Status::Ok on success; any other value triggers a fault frame.
    virtual Status handle(const MessageHeader& cmd_header,
                          uint8_t const* cmd_payload,
                          uint8_t* resp_buf,
                          uint32_t resp_buf_max,
                          uint32_t& resp_len_out) = 0;
};


#endif // I_SUB_ADDRESS_HANDLER_H
