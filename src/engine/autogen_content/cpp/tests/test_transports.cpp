// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_transports.cpp
/// @brief Unit tests for the IFrameTransport receive() framing + resync logic.
/// @details
/// Uses the catch2 v3 amalgamated framework (main() lives in
/// catch_amalgamated.cpp).
///
/// The framing/resync algorithm is shared verbatim by all three transports
/// (UART, USB-CDC, host serial); they differ only in the byte-source
/// primitive. The bulk of the coverage drives the USB-CDC ring transport,
/// which is fully self-contained (no HAL/OS) and lets a test feed an arbitrary
/// byte stream via deposit_rx(). A POSIX-only case additionally exercises the
/// host SerialFrameTransport over a pty so the real ::read() bulk path is
/// covered end to end.

#include "thirdparty/catch_amalgamated.hpp"

#include "../transports/usb_cdc_frame_transport.h"
#include "../communication/message_header.h"

#include <cstring>
#include <vector>


namespace {

/// @brief Serialize a valid header for the given payload size.
/// @details The transport only validates the header CRC (never the payload
///          CRC), so the payload CRC field is left at zero.
std::vector<uint8_t> make_header(PayloadSizes size,
                                 bool command = true,
                                 uint8_t sub_addr = 1)
{
    MessageHeader h;
    h.set_priority(MessagePriorities::High);
    h.set_command_flag(command);
    h.set_size(size);
    h.set_fault_flag(false);
    h.set_sub_address(sub_addr);
    h.set_payload_crc(0);
    h.update_header_crc();

    uint8_t buf[6];
    const bool ok = h.serialize(buf, sizeof(buf));
    REQUIRE(ok);
    return std::vector<uint8_t>(buf, buf + 6);
}

/// @brief A USB-CDC transport with a dummy transmit function (receive-only).
uint8_t dummy_transmit(uint8_t*, uint16_t) { return 0; /* USBD_OK */ }

} // namespace


/// t1: A complete header followed by its payload, deposited in one shot, is
/// returned as two separate, byte-accurate chunks, after which the ring is
/// empty.
TEST_CASE("t1", "[usb_cdc][framing]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> hdr = make_header(PayloadSizes::size_4);
    const std::vector<uint8_t> payload = {0xDE, 0xAD, 0xBE, 0xEF};

    t.deposit_rx(hdr.data(), static_cast<uint32_t>(hdr.size()));
    t.deposit_rx(payload.data(), static_cast<uint32_t>(payload.size()));

    uint8_t out[2048];

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr.data(), 6) == 0);

    REQUIRE(t.receive(out, sizeof(out)) == 4);
    REQUIRE(std::memcmp(out, payload.data(), 4) == 0);

    // Nothing left.
    REQUIRE(t.receive(out, sizeof(out)) == 0);
}


/// t2: Bytes arriving in fragments across several receive() calls accumulate
/// correctly: a partial chunk yields 0, and the full chunk is returned only
/// once the last byte lands. Covers both the header and the payload stages.
TEST_CASE("t2", "[usb_cdc][framing]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> hdr = make_header(PayloadSizes::size_4);
    const std::vector<uint8_t> payload = {0x11, 0x22, 0x33, 0x44};

    uint8_t out[2048];

    // Header in two fragments.
    t.deposit_rx(hdr.data(), 3);
    REQUIRE(t.receive(out, sizeof(out)) == 0);
    t.deposit_rx(hdr.data() + 3, 3);
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr.data(), 6) == 0);

    // Payload in two fragments.
    t.deposit_rx(payload.data(), 2);
    REQUIRE(t.receive(out, sizeof(out)) == 0);
    t.deposit_rx(payload.data() + 2, 2);
    REQUIRE(t.receive(out, sizeof(out)) == 4);
    REQUIRE(std::memcmp(out, payload.data(), 4) == 0);
}


/// t3: A garbage byte ahead of a valid header is recovered: the first six
/// bytes fail the header CRC, the oldest byte is dropped, the single missing
/// byte is re-requested, and the now-aligned header parses cleanly. The
/// trailing payload is then delivered normally.
TEST_CASE("t3", "[usb_cdc][resync]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> hdr = make_header(PayloadSizes::size_4);
    const std::vector<uint8_t> payload = {0xA0, 0xB1, 0xC2, 0xD3};

    std::vector<uint8_t> stream;
    stream.push_back(0x00);                 // leading garbage byte
    stream.insert(stream.end(), hdr.begin(), hdr.end());
    stream.insert(stream.end(), payload.begin(), payload.end());

    t.deposit_rx(stream.data(), static_cast<uint32_t>(stream.size()));

    uint8_t out[2048];

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr.data(), 6) == 0);

    REQUIRE(t.receive(out, sizeof(out)) == 4);
    REQUIRE(std::memcmp(out, payload.data(), 4) == 0);
}


/// t4: A size-0 header carries no payload; after returning it the transport
/// goes straight back to collecting the next header (it does not stall waiting
/// for a payload), so a second size-0 header is returned in turn.
TEST_CASE("t4", "[usb_cdc][framing]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> h1 = make_header(PayloadSizes::size_0, false, 1);
    const std::vector<uint8_t> h2 = make_header(PayloadSizes::size_0, false, 2);

    uint8_t out[2048];

    t.deposit_rx(h1.data(), 6);
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, h1.data(), 6) == 0);

    t.deposit_rx(h2.data(), 6);
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, h2.data(), 6) == 0);
}


/// t5: Two complete header+payload transactions deposited back to back are
/// returned as four correctly ordered, byte-accurate chunks with no desync.
TEST_CASE("t5", "[usb_cdc][framing]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> h1 = make_header(PayloadSizes::size_2, true, 1);
    const std::vector<uint8_t> p1 = {0x01, 0x02};
    const std::vector<uint8_t> h2 = make_header(PayloadSizes::size_4, true, 2);
    const std::vector<uint8_t> p2 = {0x03, 0x04, 0x05, 0x06};

    std::vector<uint8_t> stream;
    stream.insert(stream.end(), h1.begin(), h1.end());
    stream.insert(stream.end(), p1.begin(), p1.end());
    stream.insert(stream.end(), h2.begin(), h2.end());
    stream.insert(stream.end(), p2.begin(), p2.end());

    t.deposit_rx(stream.data(), static_cast<uint32_t>(stream.size()));

    uint8_t out[2048];

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, h1.data(), 6) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 2);
    REQUIRE(std::memcmp(out, p1.data(), 2) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, h2.data(), 6) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 4);
    REQUIRE(std::memcmp(out, p2.data(), 4) == 0);
}


/// t7: reset_rx() recovers a truncated LARGE payload (case 1). A header is
/// received (framer enters Payload state), a few payload bytes arrive and are
/// pulled into the frame buffer, then the peer aborts. The scheduler abandons
/// the frame by calling reset_rx(); the stale partial is dropped and the next
/// frame is delivered intact.
TEST_CASE("t7", "[usb_cdc][resync]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    // Header declaring an 8-byte payload (larger than a 6-byte header).
    const std::vector<uint8_t> hdr1 = make_header(PayloadSizes::size_8,
                                                  true, /*sub_addr=*/1);
    t.deposit_rx(hdr1.data(), 6);

    uint8_t out[2048];
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr1.data(), 6) == 0);
    // Framer is now in Payload state, expecting 8 bytes.

    // A partial payload arrives, is pulled into the frame buffer, then stops.
    const uint8_t partial[3] = { 0xAA, 0xBB, 0xCC };
    t.deposit_rx(partial, 3);
    REQUIRE(t.receive(out, sizeof(out)) == 0);   // 3 < 8, still waiting

    // Scheduler times out and abandons the frame.
    t.reset_rx();

    // A fresh frame is delivered cleanly, with no leftover corruption.
    const std::vector<uint8_t> hdr2 = make_header(PayloadSizes::size_4,
                                                  true, /*sub_addr=*/2);
    const std::vector<uint8_t> payload2 = { 0x01, 0x02, 0x03, 0x04 };
    t.deposit_rx(hdr2.data(), 6);
    t.deposit_rx(payload2.data(), 4);

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr2.data(), 6) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 4);
    REQUIRE(std::memcmp(out, payload2.data(), 4) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 0);
}


/// t8: reset_rx() recovers a truncated SMALL payload (case 3) — the size the
/// old buffer-size heuristic could not detect (a 4-byte payload is smaller than
/// a 6-byte header read). Same flow as t7 but with a 4-byte payload truncated
/// after one byte; recovery is identical.
TEST_CASE("t8", "[usb_cdc][resync]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> hdr1 = make_header(PayloadSizes::size_4,
                                                  true, /*sub_addr=*/1);
    t.deposit_rx(hdr1.data(), 6);

    uint8_t out[2048];
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr1.data(), 6) == 0);
    // Framer is now in Payload state, expecting 4 bytes.

    const uint8_t partial[1] = { 0x99 };
    t.deposit_rx(partial, 1);
    REQUIRE(t.receive(out, sizeof(out)) == 0);   // 1 < 4, still waiting

    t.reset_rx();                                 // scheduler abandons the frame

    const std::vector<uint8_t> hdr2 = make_header(PayloadSizes::size_2,
                                                  true, /*sub_addr=*/2);
    const std::vector<uint8_t> payload2 = { 0x77, 0x88 };
    t.deposit_rx(hdr2.data(), 6);
    t.deposit_rx(payload2.data(), 2);

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr2.data(), 6) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 2);
    REQUIRE(std::memcmp(out, payload2.data(), 2) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 0);
}


/// t9: reset_rx() is a harmless no-op when the framer is already idle (in
/// Header state) — this is the path exercised by cases 2 & 4, where a full
/// payload was received but failed its CRC at the scheduler, so the scheduler
/// calls reset_rx() while framing is already aligned. The next frame is
/// unaffected.
TEST_CASE("t9", "[usb_cdc][resync]")
{
    UsbCdcFrameTransport t(&dummy_transmit);

    const std::vector<uint8_t> hdr1 = make_header(PayloadSizes::size_4,
                                                  true, /*sub_addr=*/1);
    const std::vector<uint8_t> p1 = { 0x10, 0x20, 0x30, 0x40 };
    t.deposit_rx(hdr1.data(), 6);
    t.deposit_rx(p1.data(), 4);

    uint8_t out[2048];
    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(t.receive(out, sizeof(out)) == 4);   // full payload delivered; framer back to Header

    // Scheduler detected a payload-CRC failure and calls reset_rx() while the
    // framer is already idle: must be a no-op.
    t.reset_rx();

    const std::vector<uint8_t> hdr2 = make_header(PayloadSizes::size_2,
                                                  true, /*sub_addr=*/2);
    const std::vector<uint8_t> p2 = { 0x55, 0x66 };
    t.deposit_rx(hdr2.data(), 6);
    t.deposit_rx(p2.data(), 2);

    REQUIRE(t.receive(out, sizeof(out)) == 6);
    REQUIRE(std::memcmp(out, hdr2.data(), 6) == 0);
    REQUIRE(t.receive(out, sizeof(out)) == 2);
    REQUIRE(std::memcmp(out, p2.data(), 2) == 0);
}


// The host SerialFrameTransport drives a real OS file descriptor, so it is
// only exercised on POSIX where a pty pair gives us a termios-capable device
// to point open() at and a master end to inject bytes from.
#ifndef _WIN32

#include "../../cpp_bc/host/serial_frame_transport.h"

#include <cstdlib>
#include <fcntl.h>
#include <unistd.h>


namespace {

/// @brief Poll receive() a bounded number of times until a chunk arrives.
/// @return The chunk length, or 0 if none materialized within the budget.
int32_t poll_receive(SerialFrameTransport& t, uint8_t* out, uint32_t max_len)
{
    for (int attempt = 0; attempt < 1000; ++attempt) {
        const int32_t r = t.receive(out, max_len);
        if (r != 0) {
            return r;
        }
    }
    return 0;
}

} // namespace


/// t6: End-to-end framing over a real pty. Bytes written into the pty master
/// are reassembled by SerialFrameTransport (via the POSIX bulk ::read() path)
/// into the expected header and payload chunks, and a leading garbage byte is
/// resynced away just as in the ring-buffer case.
TEST_CASE("t6", "[serial][posix]")
{
    int master = ::posix_openpt(O_RDWR | O_NOCTTY);
    if (master < 0 || ::grantpt(master) != 0 || ::unlockpt(master) != 0) {
        if (master >= 0) ::close(master);
        WARN("pty unavailable in this environment; skipping serial test");
        return;
    }
    char const* slave_name = ::ptsname(master);
    REQUIRE(slave_name != nullptr);

    SerialFrameTransport t;
    REQUIRE(t.open(slave_name, 115200));

    const std::vector<uint8_t> hdr = make_header(PayloadSizes::size_4);
    const std::vector<uint8_t> payload = {0x5A, 0x6B, 0x7C, 0x8D};

    uint8_t out[2048];

    SECTION("clean header + payload")
    {
        REQUIRE(::write(master, hdr.data(), 6) == 6);
        REQUIRE(::write(master, payload.data(), 4) == 4);

        REQUIRE(poll_receive(t, out, sizeof(out)) == 6);
        REQUIRE(std::memcmp(out, hdr.data(), 6) == 0);
        REQUIRE(poll_receive(t, out, sizeof(out)) == 4);
        REQUIRE(std::memcmp(out, payload.data(), 4) == 0);
    }

    SECTION("leading garbage byte resyncs")
    {
        const uint8_t garbage = 0x00;
        REQUIRE(::write(master, &garbage, 1) == 1);
        REQUIRE(::write(master, hdr.data(), 6) == 6);
        REQUIRE(::write(master, payload.data(), 4) == 4);

        REQUIRE(poll_receive(t, out, sizeof(out)) == 6);
        REQUIRE(std::memcmp(out, hdr.data(), 6) == 0);
        REQUIRE(poll_receive(t, out, sizeof(out)) == 4);
        REQUIRE(std::memcmp(out, payload.data(), 4) == 0);
    }

    t.close();
    ::close(master);
}

#endif // !_WIN32
