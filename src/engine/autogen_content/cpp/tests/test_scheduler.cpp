// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_scheduler.cpp
/// @brief Unit tests for the BusController and RemoteTerminal classes
/// @details
/// Uses the catch2 v3 amalgamated framework (main() lives in
/// catch_amalgamated.cpp). Transport, clock, and sub-address handlers are
/// supplied as test fakes so the scheduler can be driven deterministically.

#include "thirdparty/catch_amalgamated.hpp"

#include "../communication/scheduler.h"
#include "../communication/message_header.h"
#include "../common/crc.h"

#include <cstring>
#include <deque>
#include <vector>


///////////////////////////////////////////////////////////////////////////////
// Fakes
///////////////////////////////////////////////////////////////////////////////

namespace {

/// @brief Settable IClock for tests.
class FakeClock : public IClock {
    public:
    uint32_t m_now_ms = 0;
    int32_t  m_off_whole = 0;
    int32_t  m_off_frac  = 0;
    bool     m_has_sync  = false;

    uint32_t now_ms() const override { return m_now_ms; }
    uint32_t now_bc_ms() const override
    {
        // Local tick shifted by the whole offset, with the fractional part
        // rounding to the nearest millisecond (mirrors the real apply_bc_offset).
        // Smoothing of the offset is the OffsetSmoother's job now, not the clock's.
        uint32_t shifted = m_now_ms + static_cast<uint32_t>(m_off_whole);
        if (m_off_frac >= 500) {
            shifted += 1u;
        } else if (m_off_frac <= -500) {
            shifted -= 1u;
        }
        return shifted;
    }
    void set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms) override
    {
        m_off_whole = whole_ms;
        m_off_frac  = fractional_ms;
    }
    void set_bc_discipline(const uint32_t, const int32_t whole_ms,
                           const int32_t fractional_ms, const int32_t) override
    {
        // The scheduler tests do not exercise skew; keep just the phase offset.
        m_off_whole = whole_ms;
        m_off_frac  = fractional_ms;
    }
    void get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const override
    {
        whole_ms      = m_off_whole;
        fractional_ms = m_off_frac;
    }
    int32_t bc_skew_ppb() const override { return 0; }
    bool has_sync() const override { return m_has_sync; }
    void set_has_sync(const bool synchronized) override { m_has_sync = synchronized; }
};


/// @brief Frame-level transport backed by std::deque buffers.
/// @details
/// If `peer` is set, every send() also pushes the bytes onto peer->rx_queue.
/// receive() pops the front of rx_queue. Failure injection: fail_send,
/// fail_recv, drop_next_recvs.
class FakeTransport : public IFrameTransport {
    public:
    std::deque<std::vector<uint8_t>> tx_queue;
    std::deque<std::vector<uint8_t>> rx_queue;
    FakeTransport* peer = nullptr;
    bool fail_send  = false;
    bool fail_recv  = false;
    int  drop_next_recvs = 0;
    int  reset_rx_calls = 0;   ///< Counts reset_rx() invocations from the scheduler.

    int32_t send(uint8_t const* bytes, uint32_t length) override
    {
        if (fail_send) return -1;
        std::vector<uint8_t> v(bytes, bytes + length);
        tx_queue.push_back(v);
        if (peer != nullptr) {
            peer->rx_queue.push_back(v);
        }
        return static_cast<int32_t>(length);
    }

    int32_t receive(uint8_t* out, uint32_t max_length) override
    {
        if (fail_recv) return -1;
        if (drop_next_recvs > 0) {
            if (!rx_queue.empty()) rx_queue.pop_front();
            --drop_next_recvs;
            return 0;
        }
        if (rx_queue.empty()) return 0;
        auto& front = rx_queue.front();
        uint32_t n = static_cast<uint32_t>(front.size());
        if (n > max_length) n = max_length;
        std::memcpy(out, front.data(), n);
        rx_queue.pop_front();
        return static_cast<int32_t>(n);
    }

    void reset_rx() override { ++reset_rx_calls; }
};


/// @brief Sub-address handler that records every call and returns canned data.
class RecordingHandler : public ISubAddressHandler {
    public:
    struct Call {
        MessageHeader        header;
        std::vector<uint8_t> payload;
    };
    std::vector<Call>    calls;
    Status               next_return = Status::Ok;
    std::vector<uint8_t> next_response;   // Bytes to return as resp.

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override
    {
        Call c;
        c.header = cmd_header;
        uint32_t n = cmd_header.get_size_bytes();
        if (cmd_payload != nullptr && n > 0) {
            c.payload.assign(cmd_payload, cmd_payload + n);
        }
        calls.push_back(c);

        uint32_t to_copy = static_cast<uint32_t>(next_response.size());
        if (to_copy > resp_buf_max) to_copy = resp_buf_max;
        if (to_copy > 0) {
            std::memcpy(resp_buf, next_response.data(), to_copy);
        }
        resp_len_out = to_copy;
        return next_return;
    }
};


/// @brief Captured arguments from a completion callback.
struct Completion {
    bool                 fired = false;
    Status               status = Status::Ok;
    MessageHeader        header;
    std::vector<uint8_t> payload;
};

/// @brief CompletionFn that writes into the user-supplied Completion struct.
void capture_completion(Status status,
                        const MessageHeader& resp_header,
                        uint8_t const* resp_payload,
                        uint32_t resp_len,
                        void* user)
{
    auto* c = static_cast<Completion*>(user);
    c->fired   = true;
    c->status  = status;
    c->header  = resp_header;
    if (resp_payload != nullptr && resp_len > 0) {
        c->payload.assign(resp_payload, resp_payload + resp_len);
    } else {
        c->payload.clear();
    }
}

/// @brief Drive BC and RT until completion fires or the iteration cap hits.
void drive_until_completion(BusController& bc,
                            RemoteTerminal& rt,
                            Completion& comp,
                            int max_iters = 40)
{
    for (int i = 0; i < max_iters; ++i) {
        bc.tick();
        rt.tick();
        if (comp.fired) return;
    }
}

/// @brief Build a fresh Request set up for a simple inline write.
Request make_inline_write(uint8_t sa,
                          MessagePriorities prio,
                          PayloadSizes ps,
                          uint8_t const* bytes,
                          uint32_t len)
{
    Request r{};
    r.sub_address  = sa;
    r.priority     = prio;
    r.is_read      = false;
    r.cmd_size     = ps;
    r.cmd_len      = len;
    r.cmd_ext      = nullptr;
    if (bytes != nullptr && len > 0) {
        std::memcpy(r.cmd_inline, bytes, len <= kInlinePayloadBytes ? len : kInlinePayloadBytes);
    }
    r.resp_ext     = nullptr;
    r.resp_buf_max = 0;
    return r;
}

} // namespace


///////////////////////////////////////////////////////////////////////////////
// BusController tests
///////////////////////////////////////////////////////////////////////////////

TEST_CASE("BC submit rejects sub-address 0", "[scheduler][bc][reject]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);

    Request req = make_inline_write(0, MessagePriorities::Medium,
                                    PayloadSizes::size_0, nullptr, 0);
    Completion comp;
    Status status_out = Status::Ok;
    REQUIRE_FALSE(bc.submit(req, capture_completion, &comp, status_out));
    REQUIRE(status_out == Status::InvalidSubAddress);
    REQUIRE_FALSE(comp.fired);
}

TEST_CASE("BC submit rejects when queue is full", "[scheduler][bc][queue]")
{
    FakeTransport t;       // No peer => nothing will ever ack; ticks won't start.
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    // Fill the queue with kBcQueueDepth distinct submissions.
    for (uint8_t i = 1; i <= kBcQueueDepth; ++i) {
        uint8_t payload[1] = { i };
        Request r = make_inline_write(i, MessagePriorities::Medium,
                                      PayloadSizes::size_2, payload, 2);
        // payload size enum says 2 bytes; we set len=2 (only first matters).
        r.cmd_inline[0] = i; r.cmd_inline[1] = 0;
        Status s = Status::Ok;
        REQUIRE(bc.submit(r, capture_completion, &comp, s));
    }
    REQUIRE(bc.pending_count() == kBcQueueDepth);

    Request extra = make_inline_write(kBcQueueDepth + 1,
                                      MessagePriorities::Medium,
                                      PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE_FALSE(bc.submit(extra, capture_completion, &comp, s));
    REQUIRE(s == Status::QueueFull);
}

TEST_CASE("BC queue serves higher priority first", "[scheduler][bc][priority]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    // Submit in order: Low (sa=3), Critical (sa=4), Medium (sa=5).
    Request lo  = make_inline_write(3, MessagePriorities::Low,
                                    PayloadSizes::size_0, nullptr, 0);
    Request cri = make_inline_write(4, MessagePriorities::Critical,
                                    PayloadSizes::size_0, nullptr, 0);
    Request med = make_inline_write(5, MessagePriorities::Medium,
                                    PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(lo,  capture_completion, &comp, s));
    REQUIRE(bc.submit(cri, capture_completion, &comp, s));
    REQUIRE(bc.submit(med, capture_completion, &comp, s));

    // First tick should pop Critical and send its header.
    bc.tick();
    REQUIRE(t.tx_queue.size() == 1);
    MessageHeader sent;
    REQUIRE(sent.deserialize(t.tx_queue.front().data()));
    REQUIRE(sent.get_sub_address() == 4);
    REQUIRE(sent.get_priority() == MessagePriorities::Critical);
}

TEST_CASE("BC happy-path write completes Ok over paired transport", "[scheduler][bc][write]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_return = Status::Ok;
    handler.next_response.clear();   // Pure write: no bytes back.
    rt.register_handler(9, &handler);

    uint8_t cmd[2] = { 0x12, 0x34 };
    Request r = make_inline_write(9, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(handler.calls.size() == 1);
    REQUIRE(handler.calls[0].payload.size() == 2);
    REQUIRE(handler.calls[0].payload[0] == 0x12);
    REQUIRE(handler.calls[0].payload[1] == 0x34);
}

TEST_CASE("BC happy-path read returns payload to callback", "[scheduler][bc][read]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_return = Status::Ok;
    handler.next_response = { 0xDE, 0xAD, 0xBE, 0xEF };
    rt.register_handler(11, &handler);

    uint8_t cmd[2] = { 0x55, 0x66 };
    Request r = make_inline_write(11, MessagePriorities::High,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes; // Use inline storage on BC side.

    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(comp.payload.size() == 4);
    REQUIRE(comp.payload[0] == 0xDE);
    REQUIRE(comp.payload[1] == 0xAD);
    REQUIRE(comp.payload[2] == 0xBE);
    REQUIRE(comp.payload[3] == 0xEF);
}

TEST_CASE("RT answers opcode 0 with the registered service hash, bypassing the handler",
          "[scheduler][rt][service-hash]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    // If the handler were (incorrectly) consulted it would return a different
    // 4-byte value; the assertions below prove the framework answered instead.
    handler.next_return   = Status::Ok;
    handler.next_response = { 0xFF, 0xFF, 0xFF, 0xFF };
    rt.register_handler(11, &handler, 0xDEADBEEFu);

    // A 0-arg read of GET_SERVICE_HASH: opcode 0x00 in payload byte 0 (the
    // second byte is size_2 padding, exactly as the BC stages a 0-arg read).
    uint8_t cmd[2] = { 0x00, 0x00 };
    Request r = make_inline_write(11, MessagePriorities::Low,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;

    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    // 0xDEADBEEF, little-endian.
    REQUIRE(comp.payload.size() == 4);
    REQUIRE(comp.payload[0] == 0xEF);
    REQUIRE(comp.payload[1] == 0xBE);
    REQUIRE(comp.payload[2] == 0xAD);
    REQUIRE(comp.payload[3] == 0xDE);
    // The handler must NOT have been consulted for opcode 0.
    REQUIRE(handler.calls.empty());
}

TEST_CASE("RT opcode 0 on an unregistered sub-address still faults InvalidSubAddress",
          "[scheduler][rt][service-hash]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);
    // No handler on sub-address 7: the null-handler check fires before the
    // opcode-0 intercept could run, so the RT must NOT fabricate a hash for a
    // service that does not exist.
    uint8_t cmd[2] = { 0x00, 0x00 };
    Request r = make_inline_write(7, MessagePriorities::Low,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;

    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::RemoteFault);
    REQUIRE(comp.payload.size() == 2);
    REQUIRE(comp.payload[0] == static_cast<uint8_t>(FaultDomain::Protocol));
    REQUIRE(comp.payload[1] == static_cast<uint8_t>(Status::InvalidSubAddress));
}

TEST_CASE("BC times out when RT never responds", "[scheduler][bc][timeout]")
{
    FakeTransport t;   // No peer; nothing ever ends up in rx.
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(2, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    // Tick #1: start + send header (state -> WaitCmdAck, deadline = 0 + 50).
    bc.tick();
    REQUIRE_FALSE(comp.fired);
    REQUIRE(t.tx_queue.size() == 1);   // First header on the wire.

    // First timeout is retried (not reported) and counted; the header is re-sent.
    c.m_now_ms = kFrameTimeoutMs + 1;
    bc.tick();
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                         // SendCmdHeader: re-send the header.
    REQUIRE(t.tx_queue.size() == 2);
    REQUIRE_FALSE(comp.fired);

    // Second timeout: the retry is spent, so the transaction finally fails.
    c.m_now_ms += kFrameTimeoutMs + 1;
    bc.tick();
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Timeout);
    REQUIRE(bc.fault_count() == 2);
    // On abort the scheduler resyncs the transport framer.
    REQUIRE(t.reset_rx_calls >= 1);

    // Slot should be back to Idle; a fresh submit should succeed.
    Request r2 = make_inline_write(3, MessagePriorities::Medium,
                                   PayloadSizes::size_0, nullptr, 0);
    Status s2 = Status::Ok;
    REQUIRE(bc.submit(r2, capture_completion, &comp, s2));
}

TEST_CASE("BC reports CrcFailHeader on corrupted ack", "[scheduler][bc][crc]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(6, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    // Build a valid ack header, then flip a byte to break the CRC.
    MessageHeader ack;
    ack.set_priority(MessagePriorities::Medium);
    ack.set_command_flag(false);
    ack.set_size(PayloadSizes::size_0);
    ack.set_fault_flag(false);
    ack.set_sub_address(6);
    ack.set_payload_crc(0);
    ack.update_header_crc();
    uint8_t buf[6];
    REQUIRE(ack.serialize(buf, sizeof(buf)));
    buf[0] ^= 0xFF;     // Corrupt the header CRC.
    const std::vector<uint8_t> bad_ack(buf, buf + 6);

    bc.tick();   // Sends header.
    t.rx_queue.push_back(bad_ack);
    bc.tick();   // First bad ack -> retried (counted), not reported.
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();   // Re-sends the header.
    t.rx_queue.push_back(bad_ack);
    bc.tick();   // Second bad ack -> retry spent -> CrcFailHeader.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::CrcFailHeader);
    REQUIRE(bc.fault_count() == 2);
}

TEST_CASE("BC reports CrcFailPayload on corrupted response payload", "[scheduler][bc][crc]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(8, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    // Build a response header announcing a 4-byte payload chunk (sizes >= 4
    // carry a payload CRC; size 2 would be inline) with CRC of [1,2,3,4].
    uint8_t good_payload[4] = { 0x01, 0x02, 0x03, 0x04 };
    uint16_t payload_c = calc_crc16_lt(good_payload, 4);
    MessageHeader rh;
    rh.set_priority(MessagePriorities::Medium);
    rh.set_command_flag(false);
    rh.set_size(PayloadSizes::size_4);
    rh.set_fault_flag(false);
    rh.set_sub_address(8);
    rh.set_payload_crc(payload_c);
    rh.update_header_crc();
    uint8_t hbuf[6];
    REQUIRE(rh.serialize(hbuf, sizeof(hbuf)));
    const std::vector<uint8_t> resp_hdr(hbuf, hbuf + 6);

    // A corrupted payload (different bytes -> CRC mismatch).
    uint8_t bad_payload[4] = { 0xFF, 0xFF, 0xFF, 0xFF };
    const std::vector<uint8_t> bad(bad_payload, bad_payload + 4);

    bc.tick();                       // Sends cmd header -> WaitRespHeader.
    t.rx_queue.push_back(resp_hdr);
    t.rx_queue.push_back(bad);
    bc.tick();                       // Receives response header.
    bc.tick();                       // Receives bad payload -> retried (counted).
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                       // Re-sends cmd header.
    t.rx_queue.push_back(resp_hdr);
    t.rx_queue.push_back(bad);
    bc.tick();                       // Receives response header.
    bc.tick();                       // Receives bad payload -> CrcFailPayload.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::CrcFailPayload);
    REQUIRE(bc.fault_count() == 2);
    // On a CRC-fail abort the scheduler also resyncs the transport framer.
    REQUIRE(t.reset_rx_calls >= 1);
}

TEST_CASE("BC propagates RT fault as RemoteFault", "[scheduler][bc][fault]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_return   = Status::HandlerError;
    handler.next_response = { 0xCA, 0xFE };
    rt.register_handler(13, &handler);

    uint8_t cmd[2] = { 0x01, 0x01 };   // opcode 0x01 (0x00 is the reserved GET_SERVICE_HASH)
    Request r = make_inline_write(13, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::RemoteFault);
    // The RT prepends the App fault-domain tag, so {0xCA,0xFE} becomes
    // {App,0xCA,0xFE} (3 B) and rounds up to the size-4 slot (last byte padded).
    REQUIRE(comp.payload.size() == 4);
    REQUIRE(comp.payload[0] == static_cast<uint8_t>(FaultDomain::App));
    REQUIRE(comp.payload[1] == 0xCA);
    REQUIRE(comp.payload[2] == 0xFE);
    REQUIRE(comp.payload[3] == 0x00);
    // A handler-level fault is parsed correctly: no retry, not counted.
    REQUIRE(handler.calls.size() == 1);
    REQUIRE(bc.fault_count() == 0);
}

TEST_CASE("BC surfaces an RT protocol fault's reason code in the payload", "[scheduler][bc][fault]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);
    // No handler registered for sub-address 13 -> RT faults InvalidSubAddress.

    uint8_t cmd[2] = { 0x01, 0x01 };   // opcode 0x01 (0x00 is the reserved GET_SERVICE_HASH)
    Request r = make_inline_write(13, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    // BC still reports the generic RemoteFault; the reason rides in the payload.
    REQUIRE(comp.status == Status::RemoteFault);
    // Protocol-domain fault: byte 0 names the domain, byte 1 is the Status code.
    REQUIRE(comp.payload.size() == 2);
    REQUIRE(comp.payload[0] == static_cast<uint8_t>(FaultDomain::Protocol));
    REQUIRE(comp.payload[1] == static_cast<uint8_t>(Status::InvalidSubAddress));
    // InvalidSubAddress is deterministic: no retry, not counted.
    REQUIRE(bc.fault_count() == 0);
}

TEST_CASE("BC does not retry an App-domain fault that collides with a Status code",
          "[scheduler][bc][fault][regression]")
{
    // Regression: before fault payloads were domain-tagged, the BC read byte 0
    // of every RemoteFault as a Status and retried when it equalled
    // CrcFailPayload. A handler whose first detail byte happened to be that
    // value triggered a spurious retry. The App tag now occupies byte 0, so a
    // handler fault can never be mistaken for a retryable protocol CRC failure.
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_return = Status::HandlerError;
    // First detail byte deliberately collides with CrcFailPayload's value.
    handler.next_response = { static_cast<uint8_t>(Status::CrcFailPayload) };
    rt.register_handler(13, &handler);

    uint8_t cmd[2] = { 0x01, 0x01 };   // opcode 0x01 (0x00 is the reserved GET_SERVICE_HASH)
    Request r = make_inline_write(13, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::RemoteFault);
    // {App, 0x07} -> byte 0 is App, not Protocol, so the retry test cannot match.
    REQUIRE(comp.payload.size() == 2);
    REQUIRE(comp.payload[0] == static_cast<uint8_t>(FaultDomain::App));
    REQUIRE(comp.payload[1] == static_cast<uint8_t>(Status::CrcFailPayload));
    REQUIRE(handler.calls.size() == 1);   // Called once: no spurious retry.
    REQUIRE(bc.fault_count() == 0);
}

TEST_CASE("BC handles a zero-detail handler fault", "[scheduler][bc][fault]")
{
    // A handler may fault without writing any detail bytes. The RT still emits a
    // well-formed, domain-tagged fault: {App, 0} in the size-2 slot.
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock     c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_return = Status::HandlerError;
    handler.next_response.clear();   // Fault with no detail bytes.
    rt.register_handler(13, &handler);

    uint8_t cmd[2] = { 0x01, 0x01 };   // opcode 0x01 (0x00 is the reserved GET_SERVICE_HASH)
    Request r = make_inline_write(13, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::RemoteFault);
    REQUIRE(comp.payload.size() == 2);
    REQUIRE(comp.payload[0] == static_cast<uint8_t>(FaultDomain::App));
    REQUIRE(comp.payload[1] == 0x00);
    REQUIRE(handler.calls.size() == 1);
    REQUIRE(bc.fault_count() == 0);
}

TEST_CASE("BC inline 16B and external 32B round-trip", "[scheduler][bc][inline]")
{
    SECTION("16-byte inline")
    {
        FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
        FakeClock c;
        BusController bc(tb, c);
        FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

        RecordingHandler handler;
        rt.register_handler(20, &handler);

        uint8_t cmd[16];
        for (uint8_t i = 0; i < 16; ++i) cmd[i] = i + 1;
        Request r = make_inline_write(20, MessagePriorities::Medium,
                                      PayloadSizes::size_16, cmd, 16);
        Completion comp;
        Status s = Status::Ok;
        REQUIRE(bc.submit(r, capture_completion, &comp, s));
        drive_until_completion(bc, rt, comp);
        REQUIRE(comp.fired);
        REQUIRE(comp.status == Status::Ok);
        REQUIRE(handler.calls.size() == 1);
        REQUIRE(handler.calls[0].payload.size() == 16);
        for (uint8_t i = 0; i < 16; ++i) {
            REQUIRE(handler.calls[0].payload[i] == i + 1);
        }
    }

    SECTION("32-byte external buffer")
    {
        FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
        FakeClock c;
        BusController bc(tb, c);
        FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

        RecordingHandler handler;
        rt.register_handler(21, &handler);

        uint8_t cmd[32];
        // +1 so byte 0 (the opcode) is never 0x00, the reserved GET_SERVICE_HASH.
        for (uint8_t i = 0; i < 32; ++i) cmd[i] = (i * 3 + 1) & 0xFF;
        Request r{};
        r.sub_address  = 21;
        r.priority     = MessagePriorities::Medium;
        r.is_read      = false;
        r.cmd_size     = PayloadSizes::size_32;
        r.cmd_len      = 32;
        r.cmd_ext      = cmd;
        r.resp_ext     = nullptr;
        r.resp_buf_max = 0;

        Completion comp;
        Status s = Status::Ok;
        REQUIRE(bc.submit(r, capture_completion, &comp, s));
        drive_until_completion(bc, rt, comp);
        REQUIRE(comp.fired);
        REQUIRE(comp.status == Status::Ok);
        REQUIRE(handler.calls.size() == 1);
        REQUIRE(handler.calls[0].payload.size() == 32);
        for (uint8_t i = 0; i < 32; ++i) {
            REQUIRE(handler.calls[0].payload[i] == ((i * 3 + 1) & 0xFF));
        }
    }
}

namespace {
/// Shared state for the reentrancy test.
struct ReentryCtx {
    BusController* bc = nullptr;
    int            second_submit_status = -1;
    Completion     first;
    Completion     second;
    bool           did_second_submit = false;
};

void reentry_first_cb(Status status,
                      const MessageHeader& hdr,
                      uint8_t const* payload,
                      uint32_t len,
                      void* user)
{
    capture_completion(status, hdr, payload, len, &static_cast<ReentryCtx*>(user)->first);
    auto* ctx = static_cast<ReentryCtx*>(user);
    if (ctx->did_second_submit) return;
    ctx->did_second_submit = true;
    Request r = make_inline_write(15, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    bool ok = ctx->bc->submit(r, capture_completion, &ctx->second, s);
    ctx->second_submit_status = ok ? 1 : 0;
}
} // namespace

TEST_CASE("Callback may submit a new Request", "[scheduler][bc][reentrancy]")
{
    FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
    FakeClock c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(14, &handler);
    rt.register_handler(15, &handler);

    ReentryCtx ctx;
    ctx.bc = &bc;

    Request first = make_inline_write(14, MessagePriorities::Medium,
                                      PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(first, reentry_first_cb, &ctx, s));

    // Drive long enough for both transactions to complete.
    for (int i = 0; i < 60 && !ctx.second.fired; ++i) {
        bc.tick();
        rt.tick();
    }
    REQUIRE(ctx.first.fired);
    REQUIRE(ctx.first.status == Status::Ok);
    REQUIRE(ctx.second_submit_status == 1);
    REQUIRE(ctx.second.fired);
    REQUIRE(ctx.second.status == Status::Ok);
}


///////////////////////////////////////////////////////////////////////////////
// RemoteTerminal tests
///////////////////////////////////////////////////////////////////////////////

TEST_CASE("RT register_handler validation", "[scheduler][rt][register]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);
    RecordingHandler h;

    REQUIRE_FALSE(rt.register_handler(0, &h));
    REQUIRE_FALSE(rt.register_handler(kMaxSubAddresses, &h));
    REQUIRE_FALSE(rt.register_handler(kMaxSubAddresses + 5, &h));
    REQUIRE(rt.register_handler(5, &h));
    REQUIRE(rt.register_handler(5, nullptr));  // Unregister.
}

namespace {

/// Build a serialized cmd header with valid CRC.
std::vector<uint8_t> build_cmd_header_bytes(uint8_t sa,
                                            MessagePriorities prio,
                                            PayloadSizes size,
                                            uint16_t payload_crc_v,
                                            bool fault = false,
                                            bool command = true)
{
    MessageHeader h;
    h.set_priority(prio);
    h.set_command_flag(command);
    h.set_size(size);
    h.set_fault_flag(fault);
    h.set_sub_address(sa);
    h.set_payload_crc(payload_crc_v);
    h.update_header_crc();
    std::vector<uint8_t> out(6);
    REQUIRE(h.serialize(out.data(), 6));
    return out;
}

/// Header bytes 2-3 value for an inline (size-2) payload {b0, b1}, in the form
/// build_cmd_header_bytes() takes it (stored little-endian, so b0 lands first).
uint16_t inline_word(uint8_t b0, uint8_t b1)
{
    return static_cast<uint16_t>(b0 | (static_cast<uint16_t>(b1) << 8));
}

/// Deserialize a sent header frame and return its inline payload bytes.
std::vector<uint8_t> inline_of(const std::vector<uint8_t>& frame)
{
    MessageHeader h;
    REQUIRE(frame.size() == 6);
    REQUIRE(h.deserialize(frame.data()));
    uint8_t b[2];
    h.get_inline_payload(b);
    return std::vector<uint8_t>(b, b + 2);
}

} // namespace

TEST_CASE("RT dispatches valid command to handler exactly once", "[scheduler][rt][happy]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    handler.next_return = Status::Ok;
    rt.register_handler(4, &handler);

    uint8_t payload[4] = { 0x77, 0x88, 0x99, 0xAA };
    uint16_t pcrc = calc_crc16_lt(payload, 4);
    auto hdr_bytes = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                            PayloadSizes::size_4, pcrc);
    t.rx_queue.push_back(hdr_bytes);
    t.rx_queue.push_back(std::vector<uint8_t>(payload, payload + 4));

    rt.tick();   // Header -> ack -> state RxPayload.
    rt.tick();   // Payload -> handler -> final ack.

    REQUIRE(handler.calls.size() == 1);
    REQUIRE(handler.calls[0].payload.size() == 4);
    REQUIRE(handler.calls[0].payload[0] == 0x77);
    REQUIRE(handler.calls[0].payload[3] == 0xAA);
    REQUIRE(t.tx_queue.size() == 2);   // Two acks.
    REQUIRE(rt_fc.count == 0);         // A clean command is not a fault.
}

TEST_CASE("RT dispatches an inline 2-byte command with no intermediate ack",
          "[scheduler][rt][inline]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    // Protocol T2: the payload rides in header bytes 2-3; no chunk follows.
    t.rx_queue.push_back(build_cmd_header_bytes(4, MessagePriorities::Medium,
                                                PayloadSizes::size_2,
                                                inline_word(0x77, 0x88)));
    rt.tick();   // Header carries the whole command -> handler -> final ack.

    REQUIRE(handler.calls.size() == 1);
    REQUIRE(handler.calls[0].payload.size() == 2);
    REQUIRE(handler.calls[0].payload[0] == 0x77);
    REQUIRE(handler.calls[0].payload[1] == 0x88);
    REQUIRE(t.tx_queue.size() == 1);   // Only the final size-0 ack.
    MessageHeader ack;
    REQUIRE(ack.deserialize(t.tx_queue.front().data()));
    REQUIRE(ack.get_size_bytes() == 0);
    REQUIRE_FALSE(ack.get_fault_flag());
}

TEST_CASE("RT answers an inline command with an inline 2-byte response",
          "[scheduler][rt][inline]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    handler.next_response = { 0x5A, 0xA5 };
    rt.register_handler(4, &handler);

    t.rx_queue.push_back(build_cmd_header_bytes(4, MessagePriorities::Medium,
                                                PayloadSizes::size_2,
                                                inline_word(0x01, 0x03)));
    rt.tick();

    // One response header whose bytes 2-3 are the payload; no chunk.
    REQUIRE(t.tx_queue.size() == 1);
    MessageHeader resp;
    REQUIRE(resp.deserialize(t.tx_queue.front().data()));
    REQUIRE(resp.get_size_bytes() == 2);
    REQUIRE_FALSE(resp.has_payload_chunk());
    REQUIRE(inline_of(t.tx_queue.front()) == std::vector<uint8_t>{ 0x5A, 0xA5 });
}

TEST_CASE("RT silently drops an inline command whose bytes were corrupted",
          "[scheduler][rt][inline][crc]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    // The header CRC covers the inline payload, so flipping a payload byte
    // fails the header check: no CrcFailPayload fault, just a silent drop.
    auto hdr = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                      PayloadSizes::size_2, inline_word(0x01, 0x02));
    hdr[3] ^= 0x40;
    t.rx_queue.push_back(hdr);
    rt.tick();

    REQUIRE(handler.calls.empty());
    REQUIRE(t.tx_queue.empty());
    REQUIRE(rt_fc.count == 1);
}

TEST_CASE("RT records the command header arrival time", "[scheduler][rt][sync]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    uint8_t payload[4] = { 0x01, 0x02, 0x03, 0x04 };
    t.rx_queue.push_back(build_cmd_header_bytes(4, MessagePriorities::Medium,
                                                PayloadSizes::size_4,
                                                calc_crc16_lt(payload, 4)));
    c.m_now_ms = 100;
    rt.tick();   // Header arrives at t=100 -> ack.
    t.rx_queue.push_back(std::vector<uint8_t>(payload, payload + 4));
    c.m_now_ms = 107;
    rt.tick();   // Payload arrives later; the stamp stays at the header.

    REQUIRE(handler.calls.size() == 1);
    REQUIRE(rt.cmd_header_rx_ms() == 100);
}

TEST_CASE("RT handler error produces fault frame", "[scheduler][rt][handler_err]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    handler.next_return   = Status::HandlerError;
    handler.next_response = { 0xEE, 0x55 };
    rt.register_handler(7, &handler);

    auto hdr_bytes = build_cmd_header_bytes(7, MessagePriorities::Medium,
                                            PayloadSizes::size_0, 0);
    t.rx_queue.push_back(hdr_bytes);
    rt.tick();

    REQUIRE(handler.calls.size() == 1);
    // Response is sent as two wire chunks: header frame, then payload frame.
    REQUIRE(t.tx_queue.size() == 2);
    MessageHeader resp;
    REQUIRE(resp.deserialize(t.tx_queue.front().data()));
    REQUIRE(resp.get_fault_flag() == true);
    // {0xEE,0x55} gains the App tag -> {App,0xEE,0x55} (3 B) -> size-4 slot.
    REQUIRE(resp.get_size_bytes() == 4);
    REQUIRE(t.tx_queue.back().size() == 4);
    REQUIRE(t.tx_queue.back()[0] == static_cast<uint8_t>(FaultDomain::App));
    REQUIRE(t.tx_queue.back()[1] == 0xEE);
    REQUIRE(t.tx_queue.back()[2] == 0x55);
    REQUIRE(t.tx_queue.back()[3] == 0x00);
    REQUIRE(rt_fc.count == 0);   // Handler errors are not message faults.
}

TEST_CASE("RT silently drops header with bad CRC", "[scheduler][rt][crc]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    auto bad = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                      PayloadSizes::size_0, 0);
    bad[0] ^= 0xFF;
    t.rx_queue.push_back(bad);
    rt.tick();

    REQUIRE(handler.calls.empty());
    REQUIRE(t.tx_queue.empty());
    REQUIRE(rt_fc.count == 1);   // A bad header CRC counts as a message fault.
}

TEST_CASE("RT rejects payload with bad CRC and returns to Idle", "[scheduler][rt][crc]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    // Header announces a 4-byte chunk with a CRC that won't match what we deliver.
    auto hdr_bytes = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                            PayloadSizes::size_4, 0xBEEF);
    t.rx_queue.push_back(hdr_bytes);
    rt.tick();   // Cmd header -> ack -> RxPayload.

    uint8_t bad_payload[4] = { 0x00, 0x00, 0x00, 0x00 };  // CRC != 0xBEEF.
    t.rx_queue.push_back(std::vector<uint8_t>(bad_payload, bad_payload + 4));
    rt.tick();   // Detect CRC fail, emit fault, return to Idle.

    REQUIRE(handler.calls.empty());
    // initial ack + inline fault header = 2 frames.
    REQUIRE(t.tx_queue.size() == 2);
    MessageHeader fault;
    REQUIRE(fault.deserialize(t.tx_queue[1].data()));
    REQUIRE(fault.get_fault_flag() == true);
    REQUIRE(fault.get_size_bytes() == 2);
    REQUIRE(inline_of(t.tx_queue.back()) ==
            std::vector<uint8_t>{ static_cast<uint8_t>(FaultDomain::Protocol),
                                  static_cast<uint8_t>(Status::CrcFailPayload) });
    REQUIRE(rt_fc.count == 1);   // A bad payload CRC counts as a message fault.
}

TEST_CASE("RT faults an unregistered sub-address with InvalidSubAddress", "[scheduler][rt][fault]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);
    // No handler registered for sub-address 9.

    auto hdr_bytes = build_cmd_header_bytes(9, MessagePriorities::Medium,
                                            PayloadSizes::size_0, 0);
    t.rx_queue.push_back(hdr_bytes);
    rt.tick();

    // A single fault header with the 2-byte status inline (no preceding ack).
    REQUIRE(t.tx_queue.size() == 1);
    MessageHeader fault;
    REQUIRE(fault.deserialize(t.tx_queue.front().data()));
    REQUIRE(fault.get_fault_flag() == true);
    REQUIRE(fault.get_size_bytes() == 2);
    REQUIRE(inline_of(t.tx_queue.front()) ==
            std::vector<uint8_t>{ static_cast<uint8_t>(FaultDomain::Protocol),
                                  static_cast<uint8_t>(Status::InvalidSubAddress) });
    REQUIRE(rt_fc.count == 0);   // An unhandled sub-address is not a message fault.
}

TEST_CASE("RT faults an out-of-range sub-address with InvalidSubAddress", "[scheduler][rt][fault]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    // 40 >= kMaxSubAddresses (32): out of range.
    auto hdr_bytes = build_cmd_header_bytes(40, MessagePriorities::Medium,
                                            PayloadSizes::size_0, 0);
    t.rx_queue.push_back(hdr_bytes);
    rt.tick();

    REQUIRE(t.tx_queue.size() == 1);
    MessageHeader fault;
    REQUIRE(fault.deserialize(t.tx_queue.front().data()));
    REQUIRE(fault.get_fault_flag() == true);
    REQUIRE(inline_of(t.tx_queue.front()) ==
            std::vector<uint8_t>{ static_cast<uint8_t>(FaultDomain::Protocol),
                                  static_cast<uint8_t>(Status::InvalidSubAddress) });
}

TEST_CASE("RT faults an oversized declared payload with PayloadTooLarge", "[scheduler][rt][fault]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    // Declared size 128 B exceeds kMaxPayloadBytes (64).
    auto hdr_bytes = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                            PayloadSizes::size_128, 0);
    t.rx_queue.push_back(hdr_bytes);
    rt.tick();   // Cmd header -> initial ack -> RxPayload.
    rt.tick();   // Detect oversize -> fault -> Idle.

    REQUIRE(handler.calls.empty());
    // initial ack + inline fault header = 2 frames.
    REQUIRE(t.tx_queue.size() == 2);
    MessageHeader fault;
    REQUIRE(fault.deserialize(t.tx_queue[1].data()));
    REQUIRE(fault.get_fault_flag() == true);
    REQUIRE(fault.get_size_bytes() == 2);
    REQUIRE(inline_of(t.tx_queue.back()) ==
            std::vector<uint8_t>{ static_cast<uint8_t>(FaultDomain::Protocol),
                                  static_cast<uint8_t>(Status::PayloadTooLarge) });
    REQUIRE(rt_fc.count == 0);   // An oversize declared payload is not counted.
}

TEST_CASE("RT resyncs transport when a command payload times out", "[scheduler][rt][recovery]")
{
    FakeTransport t;
    FakeClock c;
    FaultCounter rt_fc; RemoteTerminal rt(t, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(4, &handler);

    // Header announces a 4-byte payload chunk that never arrives.
    uint8_t payload[4] = { 0x01, 0x02, 0x03, 0x04 };
    uint16_t pcrc = calc_crc16_lt(payload, 4);
    auto hdr_bytes = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                            PayloadSizes::size_4, pcrc);
    t.rx_queue.push_back(hdr_bytes);

    rt.tick();                       // Header -> ack -> RxPayload (deadline = 0 + 50).
    REQUIRE(t.reset_rx_calls == 0);  // Still waiting on the payload; no abort yet.

    c.m_now_ms = kFrameTimeoutMs + 1;
    rt.tick();                       // Payload times out -> abort_rx -> reset_rx.
    REQUIRE(t.reset_rx_calls >= 1);
    REQUIRE(handler.calls.empty());
    REQUIRE(rt_fc.count == 1);   // A command-payload timeout counts as a fault.
}

TEST_CASE("BC momentary short read keeps waiting, no reset, then completes", "[scheduler][bc][partial]")
{
    FakeTransport t;
    FakeClock c;
    BusController bc(t, c);
    Completion comp;

    Request r = make_inline_write(8, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    bc.tick();   // Sends size-0 read cmd header -> WaitRespHeader.

    // Response header announcing a 4-byte payload chunk.
    uint8_t good_payload[4] = { 0x01, 0x02, 0x03, 0x04 };
    uint16_t pcrc = calc_crc16_lt(good_payload, 4);
    MessageHeader rh;
    rh.set_priority(MessagePriorities::Medium);
    rh.set_command_flag(false);
    rh.set_size(PayloadSizes::size_4);
    rh.set_fault_flag(false);
    rh.set_sub_address(8);
    rh.set_payload_crc(pcrc);
    rh.update_header_crc();
    uint8_t hbuf[6];
    REQUIRE(rh.serialize(hbuf, sizeof(hbuf)));
    t.rx_queue.push_back(std::vector<uint8_t>(hbuf, hbuf + 6));

    bc.tick();   // Receives response header -> RecvRespPayload.

    // Payload not yet available: receive() returns 0 with the deadline NOT
    // expired (clock not advanced) — a momentary short read (case 5). The
    // scheduler must keep waiting: no completion, and crucially no reset_rx().
    bc.tick();
    REQUIRE_FALSE(comp.fired);
    REQUIRE(t.reset_rx_calls == 0);

    // The payload now arrives; the transaction completes normally.
    t.rx_queue.push_back(std::vector<uint8_t>(good_payload, good_payload + 4));
    bc.tick();
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    // reset_rx() fires exactly once, on the (successful) completion — a no-op —
    // confirming it did NOT fire during the earlier short read.
    REQUIRE(t.reset_rx_calls == 1);
}


///////////////////////////////////////////////////////////////////////////////
// Automatic-retry + fault-counter tests
///////////////////////////////////////////////////////////////////////////////

namespace {

/// @brief Minimal handler that reports a shared FaultCounter's value as a
///        4-byte little-endian response. Stands in for the real SystemHandler's
///        GET_MESSAGE_FAULT_COUNTER opcode without pulling in its HAL deps.
class FaultCounterHandler : public ISubAddressHandler {
    public:
    explicit FaultCounterHandler(const FaultCounter& fc) : m_fc(fc) {}

    Status handle(const MessageHeader& /*cmd_header*/,
                  uint8_t const* /*cmd_payload*/,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override
    {
        if (resp_buf_max < 4) {
            return Status::HandlerError;
        }
        const uint32_t v = m_fc.value();
        resp_buf[0] = static_cast<uint8_t>( v        & 0xFFu);
        resp_buf[1] = static_cast<uint8_t>((v >>  8) & 0xFFu);
        resp_buf[2] = static_cast<uint8_t>((v >> 16) & 0xFFu);
        resp_buf[3] = static_cast<uint8_t>((v >> 24) & 0xFFu);
        resp_len_out = 4;
        return Status::Ok;
    }

    private:
    const FaultCounter& m_fc;
};

} // namespace


TEST_CASE("BC retries once on timeout then succeeds", "[scheduler][bc][retry][timeout]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(2, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    bc.tick();                          // Send header -> WaitCmdAck.
    REQUIRE(t.tx_queue.size() == 1);

    c.m_now_ms = kFrameTimeoutMs + 1;
    bc.tick();                          // Timeout -> retry (counted), no completion.
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                          // Re-send header.
    REQUIRE(t.tx_queue.size() == 2);

    // A valid ack now lands; the retried transaction completes Ok.
    auto ack = build_cmd_header_bytes(2, MessagePriorities::Medium,
                                      PayloadSizes::size_0, 0, false, false);
    t.rx_queue.push_back(ack);
    bc.tick();
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(bc.fault_count() == 1);     // Recovered, but the failed attempt still counts.
}


TEST_CASE("BC retries once on CrcFailHeader then succeeds", "[scheduler][bc][retry][crc]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(4, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    auto good_ack = build_cmd_header_bytes(4, MessagePriorities::Medium,
                                           PayloadSizes::size_0, 0, false, false);
    auto bad_ack  = good_ack;
    bad_ack[0] ^= 0xFF;                 // Corrupt the header CRC.

    bc.tick();                          // Send header.
    t.rx_queue.push_back(bad_ack);
    bc.tick();                          // Bad ack -> retry (counted).
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                          // Re-send header.
    t.rx_queue.push_back(good_ack);
    bc.tick();                          // Good ack -> Ok.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(bc.fault_count() == 1);
}


TEST_CASE("BC retries a CRC-class RemoteFault then succeeds", "[scheduler][bc][retry][fault]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(5, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    // RT protocol-domain fault whose reason is CrcFailPayload -> retryable.
    uint8_t fault_payload[2] = { static_cast<uint8_t>(FaultDomain::Protocol),
                                 static_cast<uint8_t>(Status::CrcFailPayload) };
    // Protocol T2: the 2-byte fault rides inline in the fault header.
    auto fault_hdr = build_cmd_header_bytes(5, MessagePriorities::Medium,
                                            PayloadSizes::size_2,
                                            inline_word(fault_payload[0], fault_payload[1]),
                                            true, false);

    bc.tick();                          // Send header -> WaitCmdAck.
    t.rx_queue.push_back(fault_hdr);
    bc.tick();                          // Recv inline fault -> retry (counted).
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                          // Re-send header.
    auto good_ack = build_cmd_header_bytes(5, MessagePriorities::Medium,
                                           PayloadSizes::size_0, 0, false, false);
    t.rx_queue.push_back(good_ack);
    bc.tick();                          // Good ack -> Ok.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(bc.fault_count() == 1);
}


TEST_CASE("BC does not retry a non-CRC RemoteFault", "[scheduler][bc][retry][fault]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    Request r = make_inline_write(5, MessagePriorities::Medium,
                                  PayloadSizes::size_0, nullptr, 0);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    // RT protocol-domain fault whose reason is InvalidSubAddress -> NOT retryable.
    uint8_t fault_payload[2] = { static_cast<uint8_t>(FaultDomain::Protocol),
                                 static_cast<uint8_t>(Status::InvalidSubAddress) };
    auto fault_hdr = build_cmd_header_bytes(5, MessagePriorities::Medium,
                                            PayloadSizes::size_2,
                                            inline_word(fault_payload[0], fault_payload[1]),
                                            true, false);

    bc.tick();
    t.rx_queue.push_back(fault_hdr);
    bc.tick();                          // Recv inline fault -> RemoteFault, no retry.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::RemoteFault);
    REQUIRE(bc.fault_count() == 0);
    REQUIRE(t.tx_queue.size() == 1);    // Only the original header was sent.
}


TEST_CASE("RT message-fault counter is read back over the bus", "[scheduler][rt][counter]")
{
    FakeTransport tb, tr;
    tb.peer = &tr;
    tr.peer = &tb;
    FakeClock      c;
    FaultCounter   shared_fc;
    BusController   bc(tb, c);
    RemoteTerminal rt(tr, c, shared_fc);

    FaultCounterHandler fch(shared_fc);
    rt.register_handler(1, &fch);

    // Simulate three RT-detected message faults.
    shared_fc.increment();
    shared_fc.increment();
    shared_fc.increment();

    // Read the counter: size_2 read carrying [opcode 0x09, 0] on sub-address 1.
    uint8_t cmd[2] = { 0x09, 0x00 };
    Request r = make_inline_write(1, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(comp.payload.size() == 4);
    const uint32_t got = static_cast<uint32_t>(comp.payload[0])
                       | (static_cast<uint32_t>(comp.payload[1]) <<  8)
                       | (static_cast<uint32_t>(comp.payload[2]) << 16)
                       | (static_cast<uint32_t>(comp.payload[3]) << 24);
    REQUIRE(got == 3u);
}


///////////////////////////////////////////////////////////////////////////////
// Protocol T2 inline (size-2) payload tests over a paired BC <-> RT link
///////////////////////////////////////////////////////////////////////////////

TEST_CASE("BC inline 2-byte write is a 2-frame exchange", "[scheduler][bc][t2]")
{
    FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
    FakeClock c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    rt.register_handler(6, &handler);

    uint8_t cmd[2] = { 0x01, 0x05 };   // e.g. GPIO SET pin 5
    Request r = make_inline_write(6, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(handler.calls.size() == 1);
    REQUIRE(handler.calls[0].payload == std::vector<uint8_t>{ 0x01, 0x05 });
    REQUIRE(tb.tx_queue.size() == 1);   // BC: inline command header only.
    REQUIRE(tr.tx_queue.size() == 1);   // RT: final ack only.
}

TEST_CASE("BC inline read with a 2-byte response is a 2-frame exchange", "[scheduler][bc][t2]")
{
    FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
    FakeClock c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_response = { 0x01 };   // 1 byte, padded to the size-2 slot.
    rt.register_handler(6, &handler);

    uint8_t cmd[2] = { 0x01, 0x03 };
    Request r = make_inline_write(6, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(comp.payload == std::vector<uint8_t>{ 0x01, 0x00 });
    REQUIRE(tb.tx_queue.size() == 1);
    REQUIRE(tr.tx_queue.size() == 1);   // Inline response header, no chunk.
}

TEST_CASE("BC inline read with a 4-byte response is a 3-frame exchange", "[scheduler][bc][t2]")
{
    FakeTransport tb, tr; tb.peer = &tr; tr.peer = &tb;
    FakeClock c;
    BusController bc(tb, c);
    FaultCounter rt_fc; RemoteTerminal rt(tr, c, rt_fc);

    RecordingHandler handler;
    handler.next_response = { 0xDE, 0xAD, 0xBE, 0xEF };
    rt.register_handler(6, &handler);

    uint8_t cmd[2] = { 0x01, 0x00 };   // 0-arg read: opcode + pad.
    Request r = make_inline_write(6, MessagePriorities::Medium,
                                  PayloadSizes::size_2, cmd, 2);
    r.is_read      = true;
    r.resp_buf_max = kInlinePayloadBytes;
    Completion comp;
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    drive_until_completion(bc, rt, comp);
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(comp.payload == std::vector<uint8_t>{ 0xDE, 0xAD, 0xBE, 0xEF });
    REQUIRE(tb.tx_queue.size() == 1);   // No intermediate ack round trip.
    REQUIRE(tr.tx_queue.size() == 2);   // Response header + payload chunk.
}

TEST_CASE("BC retries a size-4 command after an inline CrcFailPayload fault",
          "[scheduler][bc][t2][retry]")
{
    FakeTransport t;
    FakeClock     c;
    BusController bc(t, c);
    Completion    comp;

    uint8_t cmd[4] = { 0x01, 0x02, 0x03, 0x04 };
    Request r = make_inline_write(5, MessagePriorities::Medium,
                                  PayloadSizes::size_4, cmd, 4);
    Status s = Status::Ok;
    REQUIRE(bc.submit(r, capture_completion, &comp, s));

    auto ack = build_cmd_header_bytes(5, MessagePriorities::Medium,
                                      PayloadSizes::size_0, 0, false, false);
    auto fault_hdr = build_cmd_header_bytes(
        5, MessagePriorities::Medium, PayloadSizes::size_2,
        inline_word(static_cast<uint8_t>(FaultDomain::Protocol),
                    static_cast<uint8_t>(Status::CrcFailPayload)),
        true, false);

    bc.tick();                       // Header -> WaitCmdAck.
    t.rx_queue.push_back(ack);
    bc.tick();                       // Ack -> SendCmdPayload.
    bc.tick();                       // Payload sent -> WaitWriteAck.
    t.rx_queue.push_back(fault_hdr);
    bc.tick();                       // Inline CrcFailPayload -> retry.
    REQUIRE_FALSE(comp.fired);
    REQUIRE(bc.fault_count() == 1);

    bc.tick();                       // Re-send header.
    t.rx_queue.push_back(ack);
    bc.tick();
    bc.tick();                       // Re-send payload.
    t.rx_queue.push_back(ack);
    bc.tick();                       // Final ack -> Ok.
    REQUIRE(comp.fired);
    REQUIRE(comp.status == Status::Ok);
    REQUIRE(bc.fault_count() == 1);
}
