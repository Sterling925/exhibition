// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_crcs.cpp
/// @brief Unit tests for the CRC-16/MODBUS implementations in common/crc.cpp
///
/// Built via CMake (CMakeLists.txt) in this directory. Uses the catch2 v3 amalgamated
/// framework; catch_amalgamated.cpp provides main() so this file just declares
/// TEST_CASEs.

// Test framework include
#include "thirdparty/catch_amalgamated.hpp"

// Code under test
#include "../common/crc.h"

#include <stdint.h>


namespace {

/// @brief One MODBUS test vector: input bytes + expected uint16_t CRC.
/// @note These expected values were
///       computed from the standard CRC-16/MODBUS reference algorithm and
///       cross-checked against an independent MODBUS CRC calculator.
struct ModbusVector {
    const uint8_t* data;
    uint32_t length;
    uint16_t expected_crc;
};

constexpr uint8_t k_vec_0[] = { 0x01, 0x03, 0x00, 0x00, 0x00, 0x0A };
constexpr uint8_t k_vec_1[] = { 0x01, 0x03, 0x00, 0x01, 0x00, 0x0A };
constexpr uint8_t k_vec_2[] = { 0x01, 0x03, 0x00, 0x02, 0x00, 0x0A };
constexpr uint8_t k_vec_3[] = { 0x01, 0x03, 0x00, 0x03, 0x00, 0x0A };
constexpr uint8_t k_vec_4[] = { 0x01, 0x03, 0x00, 0x04, 0x00, 0x0A };

constexpr ModbusVector k_documented_vectors[] = {
    { k_vec_0, sizeof(k_vec_0), 0xCDC5 },
    { k_vec_1, sizeof(k_vec_1), 0x0D94 },
    { k_vec_2, sizeof(k_vec_2), 0x0D64 },
    { k_vec_3, sizeof(k_vec_3), 0xCD35 },
    { k_vec_4, sizeof(k_vec_4), 0x0C84 },
};

} // namespace


/// @test Verifies calc_crc16 (bit-shift implementation) against the five
///       MODBUS test vectors documented in crc.h. Each vector is a separate
///       SECTION so a regression points at the specific failing input.
TEST_CASE("calc_crc16 matches documented MODBUS vectors", "[crc16][bitshift]")
{
    for (const auto& v : k_documented_vectors) {
        DYNAMIC_SECTION("vector starting with reg 0x" << std::hex << int(v.data[3])) {
            REQUIRE(calc_crc16(v.data, v.length) == v.expected_crc);
        }
    }
}


/// @test Verifies calc_crc16_lt (lookup-table implementation) against the
///       same five MODBUS test vectors. Both implementations must agree with
///       the published reference values.
TEST_CASE("calc_crc16_lt matches documented MODBUS vectors", "[crc16][lookup]")
{
    for (const auto& v : k_documented_vectors) {
        DYNAMIC_SECTION("vector starting with reg 0x" << std::hex << int(v.data[3])) {
            REQUIRE(calc_crc16_lt(v.data, v.length) == v.expected_crc);
        }
    }
}


/// @test Edge case: zero-length input. With no bytes processed the CRC must
///       remain the initial value 0xFFFF for both implementations.
TEST_CASE("CRC of zero-length input is the initial value", "[crc16][edge]")
{
    constexpr uint8_t dummy = 0x00; // valid pointer, but length is zero
    REQUIRE(calc_crc16(&dummy, 0) == 0xFFFF);
    REQUIRE(calc_crc16_lt(&dummy, 0) == 0xFFFF);
}


/// @test Edge case: single zero byte. Hand-computed expected value is 0x40BF
///       (initial 0xFFFF XOR 0x00 = 0xFFFF, then eight CRC-16/MODBUS steps).
///       Both implementations must produce 0x40BF.
TEST_CASE("CRC of single 0x00 byte is 0x40BF", "[crc16][edge]")
{
    constexpr uint8_t one_byte[] = { 0x00 };
    REQUIRE(calc_crc16(one_byte, 1) == 0x40BF);
    REQUIRE(calc_crc16_lt(one_byte, 1) == 0x40BF);
}


/// @test Cross-consistency: on a 64-byte pseudo-pattern buffer, the bit-shift
///       and lookup-table implementations must return identical values. This
///       guards against divergence between the two algorithms without
///       requiring an externally computed expected CRC.
TEST_CASE("calc_crc16 and calc_crc16_lt agree on a 64-byte pattern", "[crc16][consistency]")
{
    uint8_t buf[64];
    for (uint32_t i = 0; i < sizeof(buf); ++i) {
        buf[i] = static_cast<uint8_t>((i * 7u + 13u) & 0xFFu);
    }

    const uint16_t crc_bitshift = calc_crc16(buf, sizeof(buf));
    const uint16_t crc_lookup   = calc_crc16_lt(buf, sizeof(buf));

    REQUIRE(crc_bitshift == crc_lookup);
}
