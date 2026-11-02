// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_message_header.cpp
/// @brief Unit tests for the MessageHeader class
///
/// Built via CMake (CMakeLists.txt) in this directory. Uses the catch2 v3 amalgamated
/// framework; catch_amalgamated.cpp provides main() so this file just declares
/// TEST_CASEs.

// Test framework include
#include "thirdparty/catch_amalgamated.hpp"

// Code under test
#include "../communication/message_header.h"
#include "../common/crc.h"

#include <stdint.h>
#include <cstring>


namespace {

/// All payload sizes in enum-declaration order, paired with the byte count
/// each one represents. Drives the size round-trip test.
struct SizeCase {
    PayloadSizes value;
    uint32_t bytes;
};

/// @brief Map payload enum to actual byte sizes
constexpr SizeCase k_all_sizes[] = {
    { PayloadSizes::size_0,    0 },
    { PayloadSizes::size_2,    2 },
    { PayloadSizes::size_4,    4 },
    { PayloadSizes::size_6,    6 },
    { PayloadSizes::size_8,    8 },
    { PayloadSizes::size_10,   10 },
    { PayloadSizes::size_12,   12 },
    { PayloadSizes::size_14,   14 },
    { PayloadSizes::size_16,   16 },
    { PayloadSizes::size_32,   32 },
    { PayloadSizes::size_64,   64 },
    { PayloadSizes::size_128,  128 },
    { PayloadSizes::size_256,  256 },
    { PayloadSizes::size_512,  512 },
    { PayloadSizes::size_1024, 1024 },
    { PayloadSizes::size_2048, 2048 },
};

/// @brief Iterative list of priorities
constexpr MessagePriorities k_all_priorities[] = {
    MessagePriorities::Critical,
    MessagePriorities::High,
    MessagePriorities::Medium,
    MessagePriorities::Low,
};

/// @brief Populate every field in a header with non-zero, distinct values and
///        finalise its CRC. Used by tests that need a known-valid header.
void populate_full(MessageHeader& h)
{
    h.set_priority(MessagePriorities::Medium);
    h.set_command_flag(true);
    h.set_size(PayloadSizes::size_128);
    h.set_fault_flag(false);
    h.set_sub_address(0x42);
    h.set_payload_crc(0xABCD);
    h.update_header_crc();
}

} // namespace


/// @test A freshly constructed header has every field zero. The CRC field is
///       also zero, which cannot match the CRC-16/MODBUS of four zero bytes,
///       so check_header_crc() must report false until update_header_crc()
///       is called.
TEST_CASE("Default-constructed header has all fields zero", "[message_header][ctor]")
{
    MessageHeader h;

    REQUIRE(h.get_priority()      == MessagePriorities::Critical); // enum value 0
    REQUIRE(h.get_command_flag()  == false);
    REQUIRE(h.get_size()          == PayloadSizes::size_0);
    REQUIRE(h.get_size_bytes()    == 0u);
    REQUIRE(h.get_fault_flag()    == true);  // bit polarity: 0 stored = fault present
    REQUIRE(h.get_sub_address()   == 0u);
    REQUIRE(h.get_payload_crc()   == 0u);
    REQUIRE(h.get_header_crc()    == 0u);
    REQUIRE(h.check_header_crc()  == false);
}


/// @test set_priority / get_priority round-trip for every defined priority.
TEST_CASE("Priority round-trips for all four values", "[message_header][priority]")
{
    for (MessagePriorities p : k_all_priorities) {
        MessageHeader h;
        h.set_priority(p);
        REQUIRE(h.get_priority() == p);
    }
}


/// @test Command flag round-trips, and toggling it does not disturb the
///       priority bits stored in the same byte.
TEST_CASE("Command flag round-trips", "[message_header][command]")
{
    MessageHeader h;

    h.set_command_flag(true);
    REQUIRE(h.get_command_flag() == true);

    h.set_command_flag(false);
    REQUIRE(h.get_command_flag() == false);

    // Cross-field independence: setting/clearing the command bit must not
    // disturb priority, which shares byte 0.
    h.set_priority(MessagePriorities::High);
    h.set_command_flag(true);
    REQUIRE(h.get_priority() == MessagePriorities::High);
    h.set_command_flag(false);
    REQUIRE(h.get_priority() == MessagePriorities::High);
}


/// @test Every PayloadSizes value round-trips through set_size / get_size and
///       reports the expected byte count from get_size_bytes.
TEST_CASE("Size round-trips for every PayloadSizes value", "[message_header][size]")
{
    for (const auto& sc : k_all_sizes) {
        DYNAMIC_SECTION("size enum 0x" << std::hex << int(static_cast<uint8_t>(sc.value))) {
            MessageHeader h;
            h.set_size(sc.value);
            REQUIRE(h.get_size()       == sc.value);
            REQUIRE(h.get_size_bytes() == sc.bytes);
        }
    }
}


/// @test The fault bit is stored with inverted polarity (0 = fault present,
///       1 = no fault). Confirm by serializing and inspecting byte 0.
TEST_CASE("Fault flag polarity matches the documented bit value", "[message_header][fault]")
{
    uint8_t buf[6];

    MessageHeader fault;
    fault.set_fault_flag(true);
    fault.update_header_crc();
    REQUIRE(fault.serialize(buf, sizeof(buf)) == true);
    REQUIRE((buf[0] & 0x01) == 0x00); // fault present -> bit 0 cleared
    REQUIRE(fault.get_fault_flag() == true);

    MessageHeader ok;
    ok.set_fault_flag(false);
    ok.update_header_crc();
    REQUIRE(ok.serialize(buf, sizeof(buf)) == true);
    REQUIRE((buf[0] & 0x01) == 0x01); // no fault -> bit 0 set
    REQUIRE(ok.get_fault_flag() == false);
}


/// @test Sub-address round-trips across a handful of values covering the
///       low, mid, and high ends of the uint8_t range.
TEST_CASE("Sub-address round-trips across full uint8 range", "[message_header][subaddr]")
{
    constexpr uint8_t samples[] = { 0x00, 0x01, 0x42, 0xFF };
    for (uint8_t addr : samples) {
        MessageHeader h;
        h.set_sub_address(addr);
        REQUIRE(h.get_sub_address() == addr);
    }
}


/// @test set_payload_crc stores the 16-bit CRC little-endian (low byte at
///       index 2, high byte at index 3), and get_payload_crc round-trips it.
TEST_CASE("Payload CRC round-trips and stores little-endian", "[message_header][payload_crc]")
{
    MessageHeader h;
    h.set_payload_crc(0xABCD);
    h.update_header_crc();

    REQUIRE(h.get_payload_crc() == 0xABCD);

    uint8_t buf[6];
    REQUIRE(h.serialize(buf, sizeof(buf)) == true);
    REQUIRE(buf[2] == 0xCD); // low byte
    REQUIRE(buf[3] == 0xAB); // high byte
}


/// @test update_header_crc populates the CRC field so that a subsequent
///       check_header_crc returns true; mutating any field afterwards
///       invalidates the CRC.
TEST_CASE("update_header_crc + check_header_crc form a valid pair", "[message_header][header_crc]")
{
    MessageHeader h;
    populate_full(h);
    REQUIRE(h.check_header_crc() == true);

    // Mutating a covered field invalidates the stored CRC.
    h.set_sub_address(0x99);
    REQUIRE(h.check_header_crc() == false);
}


/// @test Re-applying a setter with the same value must leave every other
///       getter unchanged. Guards against unrelated bits being clobbered
///       inside byte 0.
TEST_CASE("Setters do not corrupt unrelated fields", "[message_header][independence]")
{
    MessageHeader h;
    populate_full(h);

    const auto p_orig    = h.get_priority();
    const auto c_orig    = h.get_command_flag();
    const auto s_orig    = h.get_size();
    const auto sb_orig   = h.get_size_bytes();
    const auto f_orig    = h.get_fault_flag();
    const auto a_orig    = h.get_sub_address();
    const auto pcrc_orig = h.get_payload_crc();

    // Re-apply each setter with its current value.
    h.set_priority(p_orig);
    h.set_command_flag(c_orig);
    h.set_size(s_orig);
    h.set_fault_flag(f_orig);
    h.set_sub_address(a_orig);
    h.set_payload_crc(pcrc_orig);

    REQUIRE(h.get_priority()     == p_orig);
    REQUIRE(h.get_command_flag() == c_orig);
    REQUIRE(h.get_size()         == s_orig);
    REQUIRE(h.get_size_bytes()   == sb_orig);
    REQUIRE(h.get_fault_flag()   == f_orig);
    REQUIRE(h.get_sub_address()  == a_orig);
    REQUIRE(h.get_payload_crc()  == pcrc_orig);
    // Idempotent setters must leave the CRC valid.
    REQUIRE(h.check_header_crc() == true);
}


/// @test Serializing a populated header into a buffer and deserialising it
///       into a fresh header yields identical field values and a valid CRC.
TEST_CASE("Serialize round-trips through deserialize", "[message_header][serialize]")
{
    MessageHeader src;
    populate_full(src);

    uint8_t buf[6] = { 0 };
    REQUIRE(src.serialize(buf, sizeof(buf)) == true);

    MessageHeader dst;
    REQUIRE(dst.deserialize(buf) == true);

    REQUIRE(dst.get_priority()     == src.get_priority());
    REQUIRE(dst.get_command_flag() == src.get_command_flag());
    REQUIRE(dst.get_size()         == src.get_size());
    REQUIRE(dst.get_size_bytes()   == src.get_size_bytes());
    REQUIRE(dst.get_fault_flag()   == src.get_fault_flag());
    REQUIRE(dst.get_sub_address()  == src.get_sub_address());
    REQUIRE(dst.get_payload_crc()  == src.get_payload_crc());
    REQUIRE(dst.get_header_crc()   == src.get_header_crc());
    REQUIRE(dst.check_header_crc() == true);
}


/// @test serialize must report failure when the destination buffer is too
///       small to hold the 6-byte header. A sentinel-filled buffer also
///       confirms the call does not partially write the buffer.
TEST_CASE("Serialize fails when buffer is too small", "[message_header][serialize][edge]")
{
    MessageHeader h;
    populate_full(h);

    uint8_t buf[6];
    std::memset(buf, 0xA5, sizeof(buf));

    REQUIRE(h.serialize(buf, 5) == false);
    // Sentinels must still be intact.
    for (uint32_t i = 0; i < sizeof(buf); ++i) {
        REQUIRE(buf[i] == 0xA5);
    }
}


/// @test serialize must report failure when the stored header CRC does not
///       match the header contents (caller forgot update_header_crc()).
TEST_CASE("Serialize fails when header CRC is stale", "[message_header][serialize][edge]")
{
    MessageHeader h;
    h.set_priority(MessagePriorities::High);
    h.set_command_flag(true);
    h.set_size(PayloadSizes::size_16);
    h.set_fault_flag(false);
    h.set_sub_address(0x33);
    h.set_payload_crc(0x1234);
    // Intentionally no update_header_crc() here.

    uint8_t buf[6];
    REQUIRE(h.serialize(buf, sizeof(buf)) == false);
}


/// @test deserialize must reject a buffer whose header CRC does not match
///       the covered bytes, e.g. when a single bit in bytes 0..3 is flipped.
TEST_CASE("Deserialize fails on corrupted buffer", "[message_header][deserialize][edge]")
{
    MessageHeader src;
    populate_full(src);

    uint8_t buf[6];
    REQUIRE(src.serialize(buf, sizeof(buf)) == true);

    // Flip a bit in the covered region (byte 1, sub-address).
    buf[1] ^= 0x01;

    MessageHeader dst;
    REQUIRE(dst.deserialize(buf) == false);
}


/// @test deserialize must accept a hand-constructed buffer whose CRC was
///       computed independently from the field bytes, and reconstruct every
///       getter correctly. This guards against the implementation relying
///       on its own serialize output for the CRC layout.
TEST_CASE("Deserialize succeeds on a known-good byte pattern", "[message_header][deserialize]")
{
    // Field choices: priority Medium (0x02 << 6 = 0x80),
    //                command = true (0x20),
    //                large frame + size index 3 (size_128) -> 0x10 | (0x03<<1) = 0x16,
    //                fault flag = false -> bit 0 set (0x01).
    // Byte 0 = 0x80 | 0x20 | 0x16 | 0x01 = 0xB7.
    // Sub address 0x42, payload CRC 0xABCD -> 0xCD, 0xAB.
    uint8_t buf[6];
    buf[0] = 0xB7;
    buf[1] = 0x42;
    buf[2] = 0xCD;
    buf[3] = 0xAB;
    // Compute the expected CRC with the same implementation the production
    // code uses, so the test tracks the compile-time selection.
    const uint16_t header_crc = kMessageHeaderUseLookupCrc
        ? calc_crc16_lt(buf, 4)
        : calc_crc16(buf, 4);
    buf[4] = static_cast<uint8_t>(header_crc & 0xFF);
    buf[5] = static_cast<uint8_t>((header_crc >> 8) & 0xFF);

    MessageHeader h;
    REQUIRE(h.deserialize(buf) == true);

    REQUIRE(h.get_priority()     == MessagePriorities::Medium);
    REQUIRE(h.get_command_flag() == true);
    REQUIRE(h.get_size()         == PayloadSizes::size_128);
    REQUIRE(h.get_size_bytes()   == 128u);
    REQUIRE(h.get_fault_flag()   == false);
    REQUIRE(h.get_sub_address()  == 0x42);
    REQUIRE(h.get_payload_crc()  == 0xABCD);
    REQUIRE(h.get_header_crc()   == header_crc);
    REQUIRE(h.check_header_crc() == true);
}


TEST_CASE("Inline payload round-trips through header bytes 2-3", "[message_header][t2]")
{
    MessageHeader h;
    h.set_priority(MessagePriorities::Medium);
    h.set_command_flag(true);
    h.set_size(PayloadSizes::size_2);
    h.set_fault_flag(false);
    h.set_sub_address(3);
    const uint8_t in[2] = { 0x12, 0xAB };
    h.set_inline_payload(in);
    h.update_header_crc();

    uint8_t buf[6];
    REQUIRE(h.serialize(buf, sizeof(buf)));
    REQUIRE(buf[2] == 0x12);   // Wire order: byte 2 = payload[0].
    REQUIRE(buf[3] == 0xAB);

    MessageHeader back;
    REQUIRE(back.deserialize(buf));
    uint8_t out[2] = { 0, 0 };
    back.get_inline_payload(out);
    REQUIRE(out[0] == 0x12);
    REQUIRE(out[1] == 0xAB);
    // The same bytes read as the little-endian payload-CRC field.
    REQUIRE(back.get_payload_crc() == 0xAB12);

    // The header CRC covers the inline payload: corrupting it fails deserialize.
    buf[3] ^= 0x01;
    MessageHeader bad;
    REQUIRE_FALSE(bad.deserialize(buf));
}

TEST_CASE("has_payload_chunk is false only for sizes 0 and 2", "[message_header][t2]")
{
    MessageHeader h;
    h.set_size(PayloadSizes::size_0);
    REQUIRE_FALSE(h.has_payload_chunk());
    h.set_size(PayloadSizes::size_2);
    REQUIRE_FALSE(h.has_payload_chunk());
    h.set_size(PayloadSizes::size_4);
    REQUIRE(h.has_payload_chunk());
    h.set_size(PayloadSizes::size_2048);
    REQUIRE(h.has_payload_chunk());
}
