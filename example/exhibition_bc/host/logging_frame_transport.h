// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef LOGGING_FRAME_TRANSPORT_H
#define LOGGING_FRAME_TRANSPORT_H

#include <stdint.h>

#include "../communication/i_frame_transport.h"


class Logger;


/// @file logging_frame_transport.h
/// @brief IFrameTransport decorator that logs every frame it relays.
/// @details
/// Wraps a real transport (e.g. SerialFrameTransport) and forwards send() /
/// receive() unchanged, logging each frame to a Logger with decoded header
/// fields, the decoded command opcode, and a raw hex dump. Transport-level
/// errors (a negative return from the inner transport) are logged too.
///
/// Frame typing is exact, not heuristic: the BC's wire protocol is strictly
/// ordered (command header, optional command payload; response header,
/// optional response payload), so this class mirrors that ordering with two
/// tiny state machines rather than guessing whether 6 bytes are a header or a
/// 6-byte payload.
///
/// Opcode and sub-address names are supplied by injected functions so this
/// file stays independent of the generated api: the generated `op_name()` and
/// `sub_address_name()` are passed at construction.
class LoggingFrameTransport : public IFrameTransport {
    public:

    /// @brief Signature of the opcode-decoder function injected at construction.
    /// @param sub_address Routed sub-address of the command.
    /// @param opcode First payload byte of the command.
    /// @return Static human-readable name, or "UNKNOWN" for unrecognized pairs.
    using OpNameFn = char const* (*)(uint8_t sub_address, uint8_t opcode);

    /// @brief Signature of the sub-address-decoder function injected at
    ///        construction.
    /// @param sub_address Routed sub-address.
    /// @return Static human-readable name, or "UNKNOWN" for unrecognized values.
    using SubAddrNameFn = char const* (*)(uint8_t sub_address);

    /// @brief Construct a decorator over `inner`, logging to `log`.
    /// @param inner Real transport; must outlive this object.
    /// @param log Destination logger; must outlive this object.
    /// @param op_name Opcode decoder, or nullptr to skip opcode naming.
    /// @param sub_address_name Sub-address decoder, or nullptr to skip naming.
    LoggingFrameTransport(IFrameTransport& inner, Logger& log, OpNameFn op_name,
                          SubAddrNameFn sub_address_name);
    ~LoggingFrameTransport() override = default;

    LoggingFrameTransport(const LoggingFrameTransport&) = delete;
    LoggingFrameTransport& operator=(const LoggingFrameTransport&) = delete;

    int32_t send(uint8_t const* bytes, uint32_t length) override;
    int32_t receive(uint8_t* out, uint32_t max_length) override;

    /// @brief Resync the receive path: reset this decorator's RX logging state
    ///        and forward to the inner transport's reset_rx().
    void reset_rx() override;

    private:

    /// @brief Resolve a sub-address to its name via the injected decoder, or
    ///        "?" when none was supplied.
    char const* sa_name(uint8_t sub_address) const;

    enum class TxState : uint8_t { ExpectCmdHeader, ExpectCmdPayload };
    enum class RxState : uint8_t { ExpectHeader, ExpectPayload };

    IFrameTransport& m_inner;
    Logger&          m_log;
    OpNameFn         m_op_name;
    SubAddrNameFn    m_sub_address_name;

    TxState m_tx_state;
    uint8_t m_tx_cmd_sa;

    RxState m_rx_state;
    uint8_t m_rx_sa;
};


#endif // LOGGING_FRAME_TRANSPORT_H
