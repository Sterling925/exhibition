// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "message_header.h"
#include "../common/crc.h"

#include <cstring>


namespace {

/// Byte-0 bit layout (MSB-to-LSB):
///   [priority:2][command:1][large_frame:1][size:3][fault:1]
constexpr uint8_t kPriorityMask   = 0xC0; // bits 7..6
constexpr uint8_t kPriorityShift  = 6;
constexpr uint8_t kCommandBit     = 0x20; // bit 5
constexpr uint8_t kLargeFrameBit  = 0x10; // bit 4
constexpr uint8_t kSizeMask       = 0x0E; // bits 3..1
constexpr uint8_t kSizeShift      = 1;
constexpr uint8_t kFaultBit       = 0x01; // bit 0  (1 = OK, 0 = fault)

/// Byte indices in the packed 6-byte header.
constexpr uint32_t kIdxFlags         = 0;
constexpr uint32_t kIdxSubAddr       = 1;
constexpr uint32_t kIdxPayloadCrcLo  = 2;
constexpr uint32_t kIdxPayloadCrcHi  = 3;
constexpr uint32_t kIdxHeaderCrcLo   = 4;
constexpr uint32_t kIdxHeaderCrcHi   = 5;

constexpr uint32_t kHeaderSize    = 6; // Header size in bytes
constexpr uint32_t kCrcCoverSize  = 4; // header CRC is taken over bytes 0..3

/// Payload byte counts, indexed by the 3-bit size field. The large-frame bit
/// selects between the two tables.
constexpr uint32_t kSmallSizes[8] = { 0, 2, 4, 6, 8, 10, 12, 14 };
constexpr uint32_t kLargeSizes[8] = { 16, 32, 64, 128, 256, 512, 1024, 2048 };

/// Dispatch to the CRC implementation selected at compile time by
/// kMessageHeaderUseLookupCrc (see message_header.h).
inline uint16_t header_crc(uint8_t const* data, uint32_t length)
{
    if constexpr (kMessageHeaderUseLookupCrc) {
        return calc_crc16_lt(data, length);
    } else {
        return calc_crc16(data, length);
    }
}

} // namespace


MessageHeader::MessageHeader()
{
    static_assert(sizeof(m_header) == kHeaderSize, "Header size mismatch");
    std::memset(m_header, 0, sizeof(m_header));
}

///////////////////////////////////////////////////////////////////////////////////
// setters

void MessageHeader::set_priority(MessagePriorities p)
{
    uint8_t b = m_header[kIdxFlags] & static_cast<uint8_t>(~kPriorityMask);
    b |= static_cast<uint8_t>((static_cast<uint8_t>(p) & 0x03) << kPriorityShift);
    m_header[kIdxFlags] = b;
}

void MessageHeader::set_command_flag(bool f)
{
    if (f) {
        m_header[kIdxFlags] |= kCommandBit;
    } else {
        m_header[kIdxFlags] &= static_cast<uint8_t>(~kCommandBit);
    }
}

void MessageHeader::set_size(PayloadSizes s)
{
    const uint8_t raw = static_cast<uint8_t>(s);
    uint8_t b = m_header[kIdxFlags] & static_cast<uint8_t>(~(kLargeFrameBit | kSizeMask));
    // Large-frame bit lives at 0x10 in both the enum and the byte, so it
    // copies across directly. The 3-bit size shifts up by one.
    b |= static_cast<uint8_t>(raw & kLargeFrameBit);
    b |= static_cast<uint8_t>((raw & 0x07) << kSizeShift);
    m_header[kIdxFlags] = b;
}

void MessageHeader::set_fault_flag(bool fault)
{
    // Bit polarity per header doc: 0 = fault, 1 = no fault.
    if (fault) {
        m_header[kIdxFlags] &= static_cast<uint8_t>(~kFaultBit);
    } else {
        m_header[kIdxFlags] |= kFaultBit;
    }
}

void MessageHeader::set_sub_address(uint8_t addr)
{
    m_header[kIdxSubAddr] = addr;
}

void MessageHeader::set_payload_crc(uint16_t crc)
{
    m_header[kIdxPayloadCrcLo] = static_cast<uint8_t>(crc & 0xFF);
    m_header[kIdxPayloadCrcHi] = static_cast<uint8_t>((crc >> 8) & 0xFF);
}

void MessageHeader::set_inline_payload(uint8_t const* payload)
{
    m_header[kIdxPayloadCrcLo] = payload[0];
    m_header[kIdxPayloadCrcHi] = payload[1];
}

void MessageHeader::update_header_crc()
{
    const uint16_t crc = header_crc(m_header, kCrcCoverSize);
    m_header[kIdxHeaderCrcLo] = static_cast<uint8_t>(crc & 0xFF);
    m_header[kIdxHeaderCrcHi] = static_cast<uint8_t>((crc >> 8) & 0xFF);
}

bool MessageHeader::deserialize(uint8_t const*const buffer)
{
    if (buffer == nullptr) {
        return false;
    }
    std::memcpy(m_header, buffer, kHeaderSize);
    return check_header_crc();
}

///////////////////////////////////////////////////////////////////////////////////
// getters

uint8_t MessageHeader::get_priority_byte() const {
    return m_header[kIdxFlags];
}

MessagePriorities MessageHeader::get_priority() const
{
    const uint8_t bits = static_cast<uint8_t>((m_header[kIdxFlags] & kPriorityMask) >> kPriorityShift);
    return static_cast<MessagePriorities>(bits);
}

bool MessageHeader::get_command_flag() const
{
    return (m_header[kIdxFlags] & kCommandBit) != 0;
}

PayloadSizes MessageHeader::get_size() const
{
    const uint8_t flags = m_header[kIdxFlags];
    const uint8_t large = static_cast<uint8_t>(flags & kLargeFrameBit);             // 0x00 or 0x10
    const uint8_t size3 = static_cast<uint8_t>((flags & kSizeMask) >> kSizeShift);  // 0..7
    return static_cast<PayloadSizes>(large | size3);
}

uint32_t MessageHeader::get_size_bytes() const
{
    const uint8_t flags = m_header[kIdxFlags];
    const uint8_t size3 = static_cast<uint8_t>((flags & kSizeMask) >> kSizeShift);
    if ((flags & kLargeFrameBit) != 0) {
        return kLargeSizes[size3];
    }
    return kSmallSizes[size3];
}

bool MessageHeader::get_fault_flag() const
{
    // Bit polarity per header doc: 0 = fault, 1 = no fault.
    return (m_header[kIdxFlags] & kFaultBit) == 0;
}

uint8_t MessageHeader::get_sub_address() const
{
    return m_header[kIdxSubAddr];
}

uint16_t MessageHeader::get_payload_crc() const
{
    return static_cast<uint16_t>(m_header[kIdxPayloadCrcLo])
         | static_cast<uint16_t>(static_cast<uint16_t>(m_header[kIdxPayloadCrcHi]) << 8);
}

void MessageHeader::get_inline_payload(uint8_t* out) const
{
    out[0] = m_header[kIdxPayloadCrcLo];
    out[1] = m_header[kIdxPayloadCrcHi];
}

bool MessageHeader::has_payload_chunk() const
{
    return get_size_bytes() > kHeaderInlinePayloadBytes;
}

uint16_t MessageHeader::get_header_crc() const
{
    return static_cast<uint16_t>(m_header[kIdxHeaderCrcLo])
         | static_cast<uint16_t>(static_cast<uint16_t>(m_header[kIdxHeaderCrcHi]) << 8);
}

bool MessageHeader::check_header_crc() const
{
    return header_crc(m_header, kCrcCoverSize) == get_header_crc();
}

bool MessageHeader::serialize(uint8_t *const buffer, uint32_t buffer_max_size) const
{
    if (buffer == nullptr || buffer_max_size < kHeaderSize) {
        return false;
    }
    if (!check_header_crc()) {
        return false;
    }
    std::memcpy(buffer, m_header, kHeaderSize);
    return true;
}
