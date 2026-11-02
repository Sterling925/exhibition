// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/communication/scheduler.h"
#include "rt/common/crc.h"

#include <cstring>


///////////////////////////////////////////////////////////////////////////////
// Local helpers
///////////////////////////////////////////////////////////////////////////////

namespace {

/// @brief Reserved opcode of the universal GET_SERVICE_HASH command.
/// @details Answered framework-side by RemoteTerminal for every sub-address
///          (before the handler runs). Defined as a literal here rather than
///          pulled from the generated command_ids.h so the protocol-level
///          scheduler stays independent of the command catalog.
constexpr uint8_t kServiceHashOpcode = 0x00u;

/// @brief Compute payload CRC; returns 0 for empty payloads (matches the
///        zeroed default in MessageHeader).
uint16_t payload_crc(uint8_t const* data, uint32_t length)
{
    if (length == 0 || data == nullptr) {
        return 0;
    }
    return calc_crc16_lt(data, length);
}

/// @brief Local alias for the public payload_size_to_bytes helper from
///        scheduler.h. Kept so call sites below read like before.
inline uint32_t size_to_bytes(PayloadSizes ps)
{
    return payload_size_to_bytes(ps);
}

/// @brief Prepend the App fault-domain tag to handler-supplied fault detail.
/// @details
/// Shifts the first `len` bytes of `buf` up by one and writes FaultDomain::App
/// into byte 0, so the BC can tell a handler fault (app/HAL detail) apart from a
/// protocol fault (a Status code) without guessing. If the buffer is already
/// full, falls back to a 2-byte { App, 0 } frame (the handler detail is dropped
/// rather than overflowing). `cap` is the buffer capacity; `len` is updated.
void prepend_app_fault_tag(uint8_t* buf, uint32_t cap, uint32_t& len)
{
    if (len + 1u <= cap) {
        std::memmove(buf + 1, buf, len);
        buf[0] = static_cast<uint8_t>(FaultDomain::App);
        ++len;
    } else {
        // No room to keep the detail; emit a tag-only fault.
        buf[0] = static_cast<uint8_t>(FaultDomain::App);
        buf[1] = 0u;
        len    = 2;
    }
}

/// @brief Unsigned-wrap-safe deadline check.
bool deadline_expired(uint32_t now_ms, uint32_t deadline_ms)
{
    // True iff (now - deadline) does not wrap into a far-future delta.
    // i.e. deadline has been reached or passed.
    return static_cast<int32_t>(now_ms - deadline_ms) >= 0;
}

} // namespace


///////////////////////////////////////////////////////////////////////////////
// BusController
///////////////////////////////////////////////////////////////////////////////

BusController::BusController(IFrameTransport& transport, IClock& clock,
                             IBusFaultObserver* fault_observer)
    : m_transport(transport)
    , m_clock(clock)
    , m_fault_observer(fault_observer)
    , m_queue_len(0)
    , m_active_sa(0)
{
    for (uint8_t i = 0; i < kMaxSubAddresses; ++i) {
        m_slots[i].state         = BcSlotState::Idle;
        m_slots[i].ack_was_fault = false;
        m_slots[i].retried       = false;
        m_slots[i].resp_len      = 0;
        m_slots[i].deadline_ms   = 0;
        m_slots[i].cb            = nullptr;
        m_slots[i].user          = nullptr;
    }
}

uint8_t BusController::pending_count() const
{
    return m_queue_len;
}

uint32_t BusController::fault_count() const
{
    return m_fault_counter.value();
}

void BusController::build_cmd_header(const Request& req, MessageHeader& out) const
{
    uint8_t const* payload = (req.cmd_len > kInlinePayloadBytes) ? req.cmd_ext
                                                                 : req.cmd_inline;
    out = MessageHeader();
    out.set_priority(req.priority);
    out.set_command_flag(true);
    out.set_size(req.cmd_size);
    out.set_fault_flag(false);   // No fault on outbound commands.
    out.set_sub_address(req.sub_address);
    if (req.cmd_len == kHeaderInlinePayloadBytes) {
        // Protocol T2: a 2-byte command rides inline in header bytes 2-3 and
        // is covered by the header CRC; no payload chunk follows.
        out.set_inline_payload(payload);
    } else {
        out.set_payload_crc(payload_crc(payload, req.cmd_len));
    }
    out.update_header_crc();
}

bool BusController::enqueue_sorted(const QueuedReq& q)
{
    if (m_queue_len >= kBcQueueDepth) {
        return false;
    }
    // Find insertion point: first index whose priority_byte is strictly
    // greater than q's (stable: equal priorities preserve submission order).
    uint8_t idx = 0;
    while (idx < m_queue_len && m_queue[idx].priority_byte <= q.priority_byte) {
        ++idx;
    }
    // Shift right to make room.
    for (uint8_t i = m_queue_len; i > idx; --i) {
        m_queue[i] = m_queue[i - 1];
    }
    m_queue[idx] = q;
    ++m_queue_len;
    return true;
}

bool BusController::submit(const Request& req,
                           CompletionFn cb,
                           void* user,
                           Status& status_out)
{
    if (req.sub_address == 0 || req.sub_address >= kMaxSubAddresses) {
        status_out = Status::InvalidSubAddress;
        return false;
    }
    if (req.cmd_len > kMaxPayloadBytes) {
        status_out = Status::PayloadTooLarge;
        return false;
    }

    // Build header now so we can cache priority for the queue sort.
    MessageHeader hdr;
    build_cmd_header(req, hdr);

    QueuedReq q;
    q.req           = req;
    q.cb            = cb;
    q.user          = user;
    q.priority_byte = hdr.get_priority_byte();
    if (!enqueue_sorted(q)) {
        status_out = Status::QueueFull;
        return false;
    }

    status_out = Status::Ok;
    return true;
}

uint8_t const* BusController::slot_cmd_ptr(const BcSlot& slot)
{
    return (slot.req.cmd_len > kInlinePayloadBytes) ? slot.req.cmd_ext
                                                    : slot.req.cmd_inline;
}

uint8_t* BusController::slot_resp_buf(BcSlot& slot, uint32_t& cap_out)
{
    if (slot.req.resp_buf_max > kInlinePayloadBytes) {
        cap_out = slot.req.resp_buf_max;
        return slot.req.resp_ext;
    }
    cap_out = kInlinePayloadBytes;
    return slot.resp_inline;
}

void BusController::start_next_if_possible()
{
    // Called only when no slot is active (see tick()). Picks the head of the
    // priority-sorted queue and launches it on its target sub-address slot.
    if (m_queue_len == 0) {
        return;
    }
    const QueuedReq q = m_queue[0];
    const uint8_t sa  = q.req.sub_address;

    BcSlot& s         = m_slots[sa];
    s.req             = q.req;
    s.cb              = q.cb;
    s.user            = q.user;
    s.state           = BcSlotState::SendCmdHeader;
    s.deadline_ms     = m_clock.now_ms() + kFrameTimeoutMs;
    s.ack_was_fault   = false;
    s.retried         = false;
    s.resp_len        = 0;
    s.resp_header     = MessageHeader();
    build_cmd_header(s.req, s.pending_header);
    m_active_sa       = sa;

    // Pop head.
    for (uint8_t j = 0; j + 1 < m_queue_len; ++j) {
        m_queue[j] = m_queue[j + 1];
    }
    --m_queue_len;
}

void BusController::complete_slot(uint8_t sub_addr, Status status)
{
    BcSlot& s = m_slots[sub_addr];
    CompletionFn cb = s.cb;
    void* user      = s.user;
    MessageHeader resp_header = s.resp_header;

    uint32_t cap = 0;
    uint8_t* resp_buf = slot_resp_buf(s, cap);
    uint32_t resp_len = s.resp_len;
    uint8_t const* resp_payload = (resp_len > 0) ? resp_buf : nullptr;

    // Reset the slot to Idle BEFORE firing the callback so a re-entrant
    // submit() from inside the callback can succeed.
    s.state         = BcSlotState::Idle;
    s.cb            = nullptr;
    s.user          = nullptr;
    s.resp_len      = 0;
    s.ack_was_fault = false;
    m_active_sa     = 0;   // Slot is now Idle; no transaction in flight.

    // Resync the framer to header hunting. On a normal completion the framer is
    // already idle so this is a no-op; on an abort (timeout / error / CRC fail /
    // PayloadTooLarge) it drops any stuck partial payload so framing cannot
    // desync from protocol state. Not reached on a non-expired partial read.
    m_transport.reset_rx();

    if (cb != nullptr) {
        cb(status, resp_header, resp_payload, resp_len, user);
    }
}

void BusController::complete_inline_resp(uint8_t sub_addr)
{
    // Protocol T2: a size-2 response (normal or fault) carries its payload in
    // header bytes 2-3. Land it in the slot's response buffer exactly as a
    // payload chunk would, so callbacks and fail_slot()'s retry check (which
    // reads the fault domain/reason from resp_buf) see the same bytes.
    BcSlot& s = m_slots[sub_addr];
    uint32_t cap      = 0;
    uint8_t* resp_buf = slot_resp_buf(s, cap);
    s.resp_header.get_inline_payload(resp_buf);
    s.resp_len = kHeaderInlinePayloadBytes;
    if (s.ack_was_fault) {
        fail_slot(sub_addr, Status::RemoteFault);
    } else {
        complete_slot(sub_addr, Status::Ok);
    }
}

void BusController::fail_slot(uint8_t sub_addr, Status status)
{
    BcSlot& s = m_slots[sub_addr];

    // Only timeouts and CRC / malformed-frame failures are transient enough to
    // be worth a retry (and to count toward the fault total). A RemoteFault is
    // retryable only when the RT's fault payload says the failure was a payload
    // CRC / malformed frame; a handler-level fault (or InvalidSubAddress /
    // PayloadTooLarge) is deterministic, so a retry would just fail again.
    bool countable = false;
    switch (status) {
    case Status::Timeout:
    case Status::CrcFailHeader:
    case Status::CrcFailPayload:
        countable = true;
        break;
    case Status::RemoteFault: {
        uint32_t cap      = 0;
        uint8_t* resp_buf = slot_resp_buf(s, cap);
        // Only a *protocol*-domain CrcFailPayload is retryable. Requiring the
        // Protocol tag in byte 0 means an App-domain handler fault can never be
        // mistaken for a retryable CRC failure, even if its detail byte happens
        // to equal the CrcFailPayload value.
        countable = (s.resp_len >= 2 &&
                     resp_buf[0] == static_cast<uint8_t>(FaultDomain::Protocol) &&
                     resp_buf[1] == static_cast<uint8_t>(Status::CrcFailPayload));
        break;
    }
    default:
        break;   // TransportError / PayloadTooLarge / handler fault: terminal.
    }

    // Notify the observer of this failed attempt before acting on the retry /
    // completion decision. Fires for every fault (not just countable ones) so a
    // host log can surface terminal fault frames as well as recoverable ones;
    // will_retry tells the observer which attempts the BC will re-send.
    const bool will_retry = countable && !s.retried;
    if (m_fault_observer != nullptr) {
        m_fault_observer->on_fault(sub_addr, status, will_retry);
    }

    if (countable) {
        // Count every failed attempt, including one later recovered by the retry.
        m_fault_counter.increment();
        if (!s.retried) {
            // Single automatic retry: rewind to the command header and run the
            // normal sequence again. pending_header is unchanged, so the same
            // header is re-serialized and re-sent.
            s.retried       = true;
            s.state         = BcSlotState::SendCmdHeader;
            s.ack_was_fault = false;
            s.resp_len      = 0;
            s.resp_header   = MessageHeader();
            s.deadline_ms   = m_clock.now_ms() + kFrameTimeoutMs;
            // Drop any stuck partial frame so framing resyncs before resending.
            m_transport.reset_rx();
            return;
        }
    }

    complete_slot(sub_addr, status);
}

void BusController::advance_slot(uint8_t sub_addr, bool& did_send, bool& did_recv)
{
    BcSlot& s = m_slots[sub_addr];
    const uint32_t now = m_clock.now_ms();

    switch (s.state) {

    case BcSlotState::SendCmdHeader: {
        uint8_t buf[6];
        if (!s.pending_header.serialize(buf, sizeof(buf))) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        const int32_t r = m_transport.send(buf, 6);
        did_send = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == 6) {
            // Size-0 and inline (size-2) reads have no separate header-receipt
            // ack on the wire; the next frame is the response header carrying
            // the data.
            if (!s.pending_header.has_payload_chunk() && s.req.is_read) {
                s.state = BcSlotState::WaitRespHeader;
            } else {
                s.state = BcSlotState::WaitCmdAck;
            }
            s.deadline_ms = now + kFrameTimeoutMs;
            return;
        }
        // Busy / 0 bytes sent: just check timeout.
        if (deadline_expired(now, s.deadline_ms)) {
            fail_slot(sub_addr, Status::Timeout);
        }
        return;
    }

    case BcSlotState::WaitCmdAck: {
        uint8_t buf[6];
        const int32_t r = m_transport.receive(buf, sizeof(buf));
        did_recv = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == 0) {
            if (deadline_expired(now, s.deadline_ms)) {
                fail_slot(sub_addr, Status::Timeout);
            }
            return;
        }
        if (r != 6) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        MessageHeader hdr;
        if (!hdr.deserialize(buf)) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        s.resp_header = hdr;
        if (hdr.get_fault_flag()) {
            // Fault ack. Receive the fault payload if any, then complete.
            s.ack_was_fault = true;
            if (hdr.has_payload_chunk()) {
                s.state       = BcSlotState::RecvRespPayload;
                s.deadline_ms = now + kFrameTimeoutMs;
            } else if (hdr.get_size_bytes() == kHeaderInlinePayloadBytes) {
                complete_inline_resp(sub_addr);
            } else {
                fail_slot(sub_addr, Status::RemoteFault);
            }
            return;
        }
        // Normal ack. Proceed to send payload, or wrap up. An inline (size-2)
        // write has no payload chunk, so this ack is its final one.
        if (s.pending_header.has_payload_chunk()) {
            s.state       = BcSlotState::SendCmdPayload;
            s.deadline_ms = now + kFrameTimeoutMs;
        } else if (s.req.is_read) {
            s.state       = BcSlotState::WaitRespHeader;
            s.deadline_ms = now + kFrameTimeoutMs;
        } else {
            complete_slot(sub_addr, Status::Ok);
        }
        return;
    }

    case BcSlotState::SendCmdPayload: {
        uint8_t const* payload = slot_cmd_ptr(s);
        const int32_t r = m_transport.send(payload, s.req.cmd_len);
        did_send = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == static_cast<int32_t>(s.req.cmd_len)) {
            if (s.req.is_read) {
                s.state       = BcSlotState::WaitRespHeader;
                s.deadline_ms = now + kFrameTimeoutMs;
            } else {
                s.state       = BcSlotState::WaitWriteAck;
                s.deadline_ms = now + kFrameTimeoutMs;
            }
            return;
        }
        if (deadline_expired(now, s.deadline_ms)) {
            fail_slot(sub_addr, Status::Timeout);
        }
        return;
    }

    case BcSlotState::WaitWriteAck: {
        uint8_t buf[6];
        const int32_t r = m_transport.receive(buf, sizeof(buf));
        did_recv = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == 0) {
            if (deadline_expired(now, s.deadline_ms)) {
                fail_slot(sub_addr, Status::Timeout);
            }
            return;
        }
        if (r != 6) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        MessageHeader hdr;
        if (!hdr.deserialize(buf)) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        s.resp_header = hdr;
        if (hdr.get_fault_flag()) {
            s.ack_was_fault = true;
            if (hdr.has_payload_chunk()) {
                s.state       = BcSlotState::RecvRespPayload;
                s.deadline_ms = now + kFrameTimeoutMs;
            } else if (hdr.get_size_bytes() == kHeaderInlinePayloadBytes) {
                complete_inline_resp(sub_addr);
            } else {
                fail_slot(sub_addr, Status::RemoteFault);
            }
            return;
        }
        complete_slot(sub_addr, Status::Ok);
        return;
    }

    case BcSlotState::WaitRespHeader: {
        uint8_t buf[6];
        const int32_t r = m_transport.receive(buf, sizeof(buf));
        did_recv = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == 0) {
            if (deadline_expired(now, s.deadline_ms)) {
                fail_slot(sub_addr, Status::Timeout);
            }
            return;
        }
        if (r != 6) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        MessageHeader hdr;
        if (!hdr.deserialize(buf)) {
            fail_slot(sub_addr, Status::CrcFailHeader);
            return;
        }
        s.resp_header = hdr;
        if (hdr.get_fault_flag()) {
            s.ack_was_fault = true;
        }
        if (hdr.has_payload_chunk()) {
            s.state       = BcSlotState::RecvRespPayload;
            s.deadline_ms = now + kFrameTimeoutMs;
        } else if (hdr.get_size_bytes() == kHeaderInlinePayloadBytes) {
            complete_inline_resp(sub_addr);
        } else if (s.ack_was_fault) {
            fail_slot(sub_addr, Status::RemoteFault);
        } else {
            complete_slot(sub_addr, Status::Ok);
        }
        return;
    }

    case BcSlotState::RecvRespPayload: {
        uint32_t cap = 0;
        uint8_t* resp_buf = slot_resp_buf(s, cap);
        const uint32_t expected = s.resp_header.get_size_bytes();
        if (expected > cap) {
            // No room for the response.
            fail_slot(sub_addr, Status::PayloadTooLarge);
            return;
        }
        const int32_t r = m_transport.receive(resp_buf, cap);
        did_recv = true;
        if (r < 0) {
            fail_slot(sub_addr, Status::TransportError);
            return;
        }
        if (r == 0) {
            if (deadline_expired(now, s.deadline_ms)) {
                fail_slot(sub_addr, Status::Timeout);
            }
            return;
        }
        if (static_cast<uint32_t>(r) != expected) {
            fail_slot(sub_addr, Status::CrcFailPayload);
            return;
        }
        const uint16_t got_crc = payload_crc(resp_buf, expected);
        if (got_crc != s.resp_header.get_payload_crc()) {
            fail_slot(sub_addr, Status::CrcFailPayload);
            return;
        }
        s.resp_len = expected;
        if (s.ack_was_fault) {
            fail_slot(sub_addr, Status::RemoteFault);
        } else {
            complete_slot(sub_addr, Status::Ok);
        }
        return;
    }

    case BcSlotState::Idle:
    default:
        return;
    }
}

void BusController::tick()
{
    // m_active_sa tracks the one in-flight slot (0 == none); it is maintained
    // by start_next_if_possible() and complete_slot(), so no scan is needed.
    if (m_active_sa == 0) {
        start_next_if_possible();
        if (m_active_sa == 0) {
            return;
        }
    }

    bool did_send = false;
    bool did_recv = false;
    advance_slot(m_active_sa, did_send, did_recv);
    (void)did_send;
    (void)did_recv;
}


///////////////////////////////////////////////////////////////////////////////
// RemoteTerminal
///////////////////////////////////////////////////////////////////////////////

RemoteTerminal::RemoteTerminal(IFrameTransport& transport, IClock& clock, FaultCounter& fault_counter)
    : m_transport(transport)
    , m_clock(clock)
    , m_fault_counter(fault_counter)
    , m_state(RtState::Idle)
    , m_deadline_ms(0)
    , m_cmd_rx_ms(0)
    , m_resp_len(0)
{
    for (uint8_t i = 0; i < kMaxSubAddresses; ++i) {
        m_handlers[i]     = nullptr;
        m_service_hash[i] = 0u;
    }
}

bool RemoteTerminal::register_handler(uint8_t sub_addr, ISubAddressHandler* handler,
                                      uint32_t service_hash)
{
    if (sub_addr == 0 || sub_addr >= kMaxSubAddresses) {
        return false;
    }
    m_handlers[sub_addr]     = handler;
    m_service_hash[sub_addr] = service_hash;
    return true;
}

bool RemoteTerminal::send_ack(uint8_t sub_addr,
                              MessagePriorities prio,
                              PayloadSizes size,
                              bool fault,
                              uint8_t const* payload,
                              uint32_t payload_len)
{
    MessageHeader hdr;
    hdr.set_priority(prio);
    hdr.set_command_flag(false);          // Response.
    hdr.set_size(size);
    hdr.set_fault_flag(fault);
    hdr.set_sub_address(sub_addr);
    // Protocol T2: a 2-byte payload rides inline in header bytes 2-3 and no
    // payload chunk follows it.
    const bool inline_payload = (size == PayloadSizes::size_2 && payload != nullptr);
    if (inline_payload) {
        hdr.set_inline_payload(payload);
    } else {
        hdr.set_payload_crc(payload_crc(payload, payload_len));
    }
    hdr.update_header_crc();

    uint8_t buf[6];
    if (!hdr.serialize(buf, sizeof(buf))) {
        return false;
    }
    int32_t r = m_transport.send(buf, 6);
    if (r != 6) {
        return false;
    }
    if (!inline_payload && payload_len > 0 && payload != nullptr) {
        r = m_transport.send(payload, payload_len);
        if (r != static_cast<int32_t>(payload_len)) {
            return false;
        }
    }
    return true;
}

bool RemoteTerminal::send_fault(uint8_t sub_addr,
                                MessagePriorities prio,
                                Status code)
{
    // Self-describing fault payload: byte 0 is the FaultDomain tag, byte 1 the
    // reason code. A protocol fault names the Protocol domain and carries a
    // Status value. The pair fits the smallest non-zero frame (size_2) exactly.
    uint8_t buf[2] = { static_cast<uint8_t>(FaultDomain::Protocol),
                       static_cast<uint8_t>(code) };
    return send_ack(sub_addr, prio, PayloadSizes::size_2, true, buf, sizeof(buf));
}

void RemoteTerminal::abort_rx()
{
    m_state    = RtState::Idle;
    m_resp_len = 0;
    m_transport.reset_rx();
}

void RemoteTerminal::dispatch_command(uint32_t now)
{
    const uint8_t sa = m_active_cmd_header.get_sub_address();
    ISubAddressHandler* h = m_handlers[sa];
    m_resp_len = 0;

    // Universal command: opcode 0x00 (GET_SERVICE_HASH) is answered here by
    // the framework, before the handler runs, with this sub-address's
    // per-service API hash. Every registered sub-address reports its hash
    // uniformly this way, with no per-handler code. The opcode byte is
    // always present: even a 0-arg read travels in the 2-byte size_2 slot,
    // so m_cmd_payload[0] is the opcode on both the inline and chunk paths.
    Status hs;
    if (m_cmd_payload[0] == kServiceHashOpcode) {
        const uint32_t sh = m_service_hash[sa];
        m_resp_payload[0] = static_cast<uint8_t>( sh        & 0xFFu);
        m_resp_payload[1] = static_cast<uint8_t>((sh >>  8) & 0xFFu);
        m_resp_payload[2] = static_cast<uint8_t>((sh >> 16) & 0xFFu);
        m_resp_payload[3] = static_cast<uint8_t>((sh >> 24) & 0xFFu);
        m_resp_len = 4;
        hs         = Status::Ok;
    } else {
        hs = h->handle(m_active_cmd_header, m_cmd_payload,
                       m_resp_payload, sizeof(m_resp_payload),
                       m_resp_len);
    }

    if (hs != Status::Ok) {
        // Tag handler detail with the App fault domain before sizing, so the
        // BC reads it as app/HAL detail rather than a Status code. Note this
        // can bump the frame up one size slot.
        prepend_app_fault_tag(m_resp_payload, sizeof(m_resp_payload),
                              m_resp_len);
    }

    PayloadSizes ps   = bytes_to_payload_size(m_resp_len);
    uint32_t wire_len = size_to_bytes(ps);
    for (uint32_t i = m_resp_len; i < wire_len; ++i) {
        m_resp_payload[i] = 0;
    }
    m_resp_len = wire_len;

    if (hs != Status::Ok) {
        // Bad Write/Read 2: fault response with App-domain handler detail.
        send_ack(sa, m_active_cmd_header.get_priority(), ps, true,
                 wire_len > 0 ? m_resp_payload : nullptr, wire_len);
        m_state = RtState::Idle;
        return;
    }

    if (wire_len <= kHeaderInlinePayloadBytes) {
        // Pure write (size-0 ack), or a 2-byte read whose response rides
        // inline in the header (Protocol T2): one frame and we're done.
        send_ack(sa, m_active_cmd_header.get_priority(), ps, false,
                 wire_len > 0 ? m_resp_payload : nullptr, wire_len);
        m_state = RtState::Idle;
        return;
    }

    // Read response: send header now, payload on the next tick.
    MessageHeader resp_hdr;
    resp_hdr.set_priority(m_active_cmd_header.get_priority());
    resp_hdr.set_command_flag(false);
    resp_hdr.set_size(ps);
    resp_hdr.set_fault_flag(false);
    resp_hdr.set_sub_address(sa);
    resp_hdr.set_payload_crc(payload_crc(m_resp_payload, wire_len));
    resp_hdr.update_header_crc();

    uint8_t buf[6];
    resp_hdr.serialize(buf, sizeof(buf));
    int32_t sr = m_transport.send(buf, 6);
    if (sr != 6) {
        m_state = RtState::Idle;
        return;
    }
    m_state       = RtState::TxRespPayload;
    m_deadline_ms = now + kFrameTimeoutMs;
}

void RemoteTerminal::tick()
{
    const uint32_t now = m_clock.now_ms();

    switch (m_state) {

    case RtState::Idle: {
        uint8_t buf[6];
        const int32_t r = m_transport.receive(buf, sizeof(buf));
        if (r <= 0) {
            return;
        }
        if (r != 6) {
            // Bad chunk size: a malformed frame. Count it and drop silently.
            m_fault_counter.increment();
            return;
        }
        MessageHeader hdr;
        if (!hdr.deserialize(buf)) {
            // Header CRC failed: count the malformed frame and drop silently.
            m_fault_counter.increment();
            return;
        }
        // Arrival instant of this command, taken before any further work. The
        // system handler uses it as the time-sync receive stamp (T2), so the
        // ack and payload legs stay out of the measured forward path.
        m_cmd_rx_ms = m_clock.now_ms();

        // Fault frame inbound from BC: resync signal. Drop any in-flight
        // state and return to Idle without emitting an ack.
        if (hdr.get_fault_flag()) {
            abort_rx();
            return;
        }

        const uint8_t sa = hdr.get_sub_address();
        if (sa == 0 || sa >= kMaxSubAddresses) {
            // Reserved / out-of-range sub-address: fault with the reason code.
            send_fault(sa, hdr.get_priority(), Status::InvalidSubAddress);
            abort_rx();
            return;
        }

        ISubAddressHandler* h = m_handlers[sa];
        if (h == nullptr) {
            // No handler registered for this sub-address (Bad Write/Read 1 path):
            // report InvalidSubAddress, the closest existing reason code.
            send_fault(sa, hdr.get_priority(), Status::InvalidSubAddress);
            abort_rx();
            return;
        }

        m_active_cmd_header = hdr;

        const uint32_t declared = hdr.get_size_bytes();
        if (declared == 0) {
            // Size-0 cmds collapse the protocol: no separate header-receipt
            // ack, the single response frame carries everything.
            //
            // The universal GET_SERVICE_HASH (opcode 0x00) is intentionally NOT
            // handled here: a true size-0 frame carries no payload, so there is
            // no opcode byte to inspect. The BC always sends the opcode byte
            // (a 0-arg read travels inline in the size_2 slot), so
            // GET_SERVICE_HASH arrives on the inline path below, never here.
            m_resp_len = 0;
            Status hs = h->handle(hdr, nullptr,
                                  m_resp_payload, sizeof(m_resp_payload),
                                  m_resp_len);
            bool fault = (hs != Status::Ok);
            if (fault) {
                // App-domain tag ahead of handler detail (see dispatch_command).
                prepend_app_fault_tag(m_resp_payload, sizeof(m_resp_payload),
                                      m_resp_len);
            }
            PayloadSizes ps    = bytes_to_payload_size(m_resp_len);
            uint32_t wire_len  = size_to_bytes(ps);
            for (uint32_t i = m_resp_len; i < wire_len; ++i) {
                m_resp_payload[i] = 0;
            }
            send_ack(sa, hdr.get_priority(), ps, fault,
                     wire_len > 0 ? m_resp_payload : nullptr, wire_len);
            return;
        }

        if (declared == kHeaderInlinePayloadBytes) {
            // Protocol T2: the 2-byte command is inline in the header, so the
            // RT already has all of it. No header-receipt ack; dispatch now.
            hdr.get_inline_payload(m_cmd_payload);
            dispatch_command(now);
            return;
        }

        // Ack the header, then transition to receive the cmd payload.
        if (!send_ack(sa, hdr.get_priority(), PayloadSizes::size_0,
                      false, nullptr, 0)) {
            // Transport failure on ack: drop and reset.
            abort_rx();
            return;
        }
        m_state       = RtState::RxPayload;
        m_deadline_ms = now + kFrameTimeoutMs;
        return;
    }

    case RtState::RxPayload: {
        const uint32_t expected = m_active_cmd_header.get_size_bytes();
        if (expected > sizeof(m_cmd_payload)) {
            send_fault(m_active_cmd_header.get_sub_address(),
                       m_active_cmd_header.get_priority(),
                       Status::PayloadTooLarge);
            m_state = RtState::Idle;
            return;
        }
        const int32_t r = m_transport.receive(m_cmd_payload, sizeof(m_cmd_payload));
        if (r < 0) {
            abort_rx();
            return;
        }
        if (r == 0) {
            if (deadline_expired(now, m_deadline_ms)) {
                // Command payload never arrived: count the timeout and resync.
                m_fault_counter.increment();
                abort_rx();
            }
            return;
        }
        if (static_cast<uint32_t>(r) != expected) {
            // Wrong-sized payload: the payload didn't validate against the
            // header, so report it as a payload CRC failure.
            m_fault_counter.increment();
            send_fault(m_active_cmd_header.get_sub_address(),
                       m_active_cmd_header.get_priority(),
                       Status::CrcFailPayload);
            m_state = RtState::Idle;
            return;
        }
        const uint16_t got = payload_crc(m_cmd_payload, expected);
        if (got != m_active_cmd_header.get_payload_crc()) {
            m_fault_counter.increment();
            send_fault(m_active_cmd_header.get_sub_address(),
                       m_active_cmd_header.get_priority(),
                       Status::CrcFailPayload);
            m_state = RtState::Idle;
            return;
        }

        dispatch_command(now);
        return;
    }

    case RtState::TxRespPayload: {
        const int32_t r = m_transport.send(m_resp_payload, m_resp_len);
        if (r == static_cast<int32_t>(m_resp_len)) {
            m_state = RtState::Idle;
            return;
        }
        if (r < 0) {
            m_state = RtState::Idle;
            return;
        }
        if (deadline_expired(now, m_deadline_ms)) {
            m_state = RtState::Idle;
        }
        return;
    }
    }
}
