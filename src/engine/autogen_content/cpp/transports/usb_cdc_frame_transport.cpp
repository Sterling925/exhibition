// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "usb_cdc_frame_transport.h"

#include <cstring>

#include "../communication/message_header.h"


static constexpr uint32_t kHeaderBytes = 6;

// USBD return codes mirrored here so we do not pull `usbd_def.h` into the
// transport's compile unit. Matches `USBD_StatusTypeDef`.
static constexpr uint8_t kUsbdOk   = 0;
static constexpr uint8_t kUsbdBusy = 1;

// Max bytes per CDC IN transfer. The USB middleware can typically buffer up
// to one full packet at a time; chunking keeps us below typical bulk
// endpoint sizes (HS = 512 bytes, FS = 64 bytes). 64 is the safe lowest
// common denominator.
static constexpr uint16_t kTxChunkBytes = 64;

// How many times to retry a `USBD_BUSY` transmit before giving up. Each
// retry spins on the CPU; the upper layer is expected to call `send()` from
// the main loop only, so a short cap is appropriate.
static constexpr uint32_t kTxBusyRetries = 1000;


UsbCdcFrameTransport::UsbCdcFrameTransport(CdcTransmitFn transmit_fn)
    : m_transmit_fn(transmit_fn)
    , m_rx_state(RxState::Header)
    , m_expected_payload(0)
    , m_rx_filled(0)
    , m_head(0)
    , m_tail(0)
{
    std::memset(m_rx_buf, 0, sizeof(m_rx_buf));
    for (uint32_t i = 0; i < kRingSize; ++i) {
        m_ring[i] = 0;
    }
}


int32_t UsbCdcFrameTransport::send(uint8_t const* bytes, uint32_t length)
{
    if (m_transmit_fn == nullptr) {
        return -1;
    }
    uint32_t sent = 0;
    while (sent < length) {
        const uint32_t remaining = length - sent;
        const uint16_t chunk = (remaining > kTxChunkBytes)
            ? kTxChunkBytes
            : static_cast<uint16_t>(remaining);

        uint32_t retries = 0;
        uint8_t  status  = kUsbdBusy;
        while (retries < kTxBusyRetries) {
            status = m_transmit_fn(const_cast<uint8_t*>(bytes + sent), chunk);
            if (status == kUsbdOk) {
                break;
            }
            if (status != kUsbdBusy) {
                return -1;
            }
            retries += 1;
        }
        if (status != kUsbdOk) {
            return -1;
        }
        sent += chunk;
    }
    return static_cast<int32_t>(length);
}


void UsbCdcFrameTransport::reset_rx()
{
    m_rx_state         = RxState::Header;
    m_expected_payload = 0;
    m_rx_filled        = 0;
}


int32_t UsbCdcFrameTransport::receive(uint8_t* out, uint32_t max_length)
{
    const uint32_t needed =
        (m_rx_state == RxState::Header) ? kHeaderBytes : m_expected_payload;

    // Payload size parsed when a header completes in-loop below; reused after
    // the loop so the header is never deserialized twice.
    uint32_t hdr_payload = 0;

    // Drain bytes already buffered in the ring without blocking.
    while (m_rx_filled < needed) {
        // Pull exactly the bytes still missing for this chunk in one go: the
        // full header/payload when starting fresh, or a single byte after a
        // resync drop (see below). needed <= sizeof(m_rx_buf), so the
        // destination range is always in bounds.
        const uint32_t want = needed - m_rx_filled;
        const uint32_t got  = pop_ring_bytes(m_rx_buf + m_rx_filled, want);
        if (got == 0) {
            // Ring empty; report partial / nothing to caller.
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


void UsbCdcFrameTransport::deposit_rx(uint8_t const* buf, uint32_t len)
{
    if (buf == nullptr) {
        return;
    }
    for (uint32_t i = 0; i < len; ++i) {
        const uint32_t next_head = (m_head + 1) % kRingSize;
        if (next_head == m_tail) {
            // Ring full; drop the byte. Frame resync is the upper layer's
            // job via the header-CRC check in receive().
            return;
        }
        m_ring[m_head] = buf[i];
        m_head = next_head;
    }
}


bool UsbCdcFrameTransport::pop_ring_byte(uint8_t* out)
{
    // Cache volatile values to resolve static analysis warning
    // about non-deterministic order.
    const uint32_t tail_cache = m_tail;
    const uint32_t head_cache = m_head;

    if (tail_cache == head_cache) {
        return false;
    }
    *out = m_ring[m_tail];
    m_tail = (m_tail + 1) % kRingSize;
    return true;
}


uint32_t UsbCdcFrameTransport::pop_ring_bytes(uint8_t* dst, uint32_t want)
{
    uint32_t popped = 0;
    while (popped < want && pop_ring_byte(dst + popped)) {
        popped += 1;
    }
    return popped;
}
