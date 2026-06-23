// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "logging_frame_transport.h"

#include <cstdio>

#include "logger.h"
#include "../communication/message_header.h"


namespace {

char const* prio_name(MessagePriorities p)
{
    switch (p) {
        case MessagePriorities::Critical: return "Critical";
        case MessagePriorities::High:     return "High";
        case MessagePriorities::Medium:   return "Medium";
        case MessagePriorities::Low:      return "Low";
    }
    return "?";
}

} // namespace


LoggingFrameTransport::LoggingFrameTransport(IFrameTransport& inner,
                                             Logger& log,
                                             OpNameFn op_name,
                                             SubAddrNameFn sub_address_name)
    : m_inner(inner)
    , m_log(log)
    , m_op_name(op_name)
    , m_sub_address_name(sub_address_name)
    , m_tx_state(TxState::ExpectCmdHeader)
    , m_tx_cmd_sa(0)
    , m_rx_state(RxState::ExpectHeader)
    , m_rx_sa(0)
{
}


/// Resolve a sub-address to its name via the injected decoder, or "?" when
/// none was supplied.
char const* LoggingFrameTransport::sa_name(uint8_t sub_address) const
{
    return (m_sub_address_name != nullptr) ? m_sub_address_name(sub_address)
                                           : "?";
}


int32_t LoggingFrameTransport::send(uint8_t const* bytes, uint32_t length)
{
    const int32_t r = m_inner.send(bytes, length);
    if (r < 0) {
        m_log.line("ERROR", "TX transport error");
        return r;
    }

    char buf[96];
    if (m_tx_state == TxState::ExpectCmdHeader) {
        MessageHeader hdr;
        if (length == 6u && bytes != nullptr && hdr.deserialize(bytes)) {
            const uint8_t sa = hdr.get_sub_address();
            const unsigned sz = hdr.get_size_bytes();
            std::snprintf(buf, sizeof(buf),
                          "TX HDR cmd sa=%s size=%u prio=%s fault=%s",
                          sa_name(sa), sz, prio_name(hdr.get_priority()),
                          hdr.get_fault_flag() ? "FAULT" : "ok");
            m_log.line("INFO", buf);
            m_log.hex("DEBUG", "  bytes: ", bytes, length);
            m_tx_cmd_sa = hdr.get_sub_address();
            m_tx_state  = (sz > 0u) ? TxState::ExpectCmdPayload
                                    : TxState::ExpectCmdHeader;
        } else {
            m_log.line("INFO", "TX (unrecognized frame)");
            m_log.hex("DEBUG", "  bytes: ", bytes, length);
        }
    } else { // ExpectCmdPayload
        const uint8_t op = (length > 0u && bytes != nullptr) ? bytes[0] : 0u;
        char const* name = (m_op_name != nullptr)
                               ? m_op_name(m_tx_cmd_sa, op)
                               : "?";
        std::snprintf(buf, sizeof(buf),
                      "TX PAYLOAD sa=%s op=%s(0x%02X) len=%u",
                      sa_name(m_tx_cmd_sa), name,
                      static_cast<unsigned>(op), length);
        m_log.line("INFO", buf);
        m_log.hex("DEBUG", "  bytes: ", bytes, length);
        m_tx_state = TxState::ExpectCmdHeader;
    }
    return r;
}


void LoggingFrameTransport::reset_rx()
{
    // Abort mirrors a fresh start: realign our RX logging state machine and
    // forward the resync to the real transport underneath.
    m_rx_state = RxState::ExpectHeader;
    m_rx_sa    = 0;
    m_inner.reset_rx();
}


int32_t LoggingFrameTransport::receive(uint8_t* out, uint32_t max_length)
{
    const int32_t r = m_inner.receive(out, max_length);
    if (r < 0) {
        m_log.line("ERROR", "RX transport error");
        return r;
    }
    if (r == 0) {
        return 0; // Nothing waiting; not logged to avoid spamming the poll loop.
    }

    char buf[96];
    if (m_rx_state == RxState::ExpectHeader) {
        MessageHeader hdr;
        if (r == 6 && out != nullptr && hdr.deserialize(out)) {
            const uint8_t sa = hdr.get_sub_address();
            const unsigned sz = hdr.get_size_bytes();
            const bool fault  = hdr.get_fault_flag();
            std::snprintf(buf, sizeof(buf),
                          "RX HDR resp sa=%s size=%u fault=%s",
                          sa_name(sa), sz, fault ? "FAULT" : "ok");
            m_log.line("INFO", buf);
            m_log.hex("DEBUG", "  bytes: ", out, static_cast<uint32_t>(r));
            if (fault) {
                m_log.line("WARN", "remote fault flagged in response header");
            }
            m_rx_sa    = hdr.get_sub_address();
            m_rx_state = (sz > 0u) ? RxState::ExpectPayload
                                   : RxState::ExpectHeader;
        } else {
            m_log.line("INFO", "RX (unrecognized frame)");
            m_log.hex("DEBUG", "  bytes: ", out, static_cast<uint32_t>(r));
        }
    } else { // ExpectPayload
        std::snprintf(buf, sizeof(buf), "RX PAYLOAD sa=%s len=%u",
                      sa_name(m_rx_sa),
                      static_cast<unsigned>(r));
        m_log.line("INFO", buf);
        m_log.hex("DEBUG", "  bytes: ", out, static_cast<uint32_t>(r));
        m_rx_state = RxState::ExpectHeader;
    }
    return r;
}
