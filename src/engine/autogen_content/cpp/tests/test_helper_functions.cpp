// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_helper_functions.cpp
/// @brief Unit tests for the little-endian pack/unpack helpers in
///        common/helper_functions.cpp
///
/// Built via CMake (CMakeLists.txt) in this directory. Uses the catch2 v3 amalgamated
/// framework; catch_amalgamated.cpp provides main() so this file just declares
/// TEST_CASEs.

// Test framework include
#include "thirdparty/catch_amalgamated.hpp"

// Code under test
#include "../common/helper_functions.h"

#include <stdint.h>


/// @test emplace_u16_le writes the low byte first, then the high byte.
///       Checks the documented little-endian layout for representative and
///       boundary values.
TEST_CASE("emplace_u16_le writes little-endian byte order", "[helpers][emplace][u16]")
{
    uint8_t buf[2];

    SECTION("zero") {
        emplace_u16_le(buf, 0x0000u);
        REQUIRE(buf[0] == 0x00u);
        REQUIRE(buf[1] == 0x00u);
    }
    SECTION("all ones") {
        emplace_u16_le(buf, 0xFFFFu);
        REQUIRE(buf[0] == 0xFFu);
        REQUIRE(buf[1] == 0xFFu);
    }
    SECTION("distinct bytes") {
        emplace_u16_le(buf, 0xABCDu);
        REQUIRE(buf[0] == 0xCDu); // low byte first
        REQUIRE(buf[1] == 0xABu); // high byte second
    }
}


/// @test emplace_u32_le writes the four bytes least-significant first.
TEST_CASE("emplace_u32_le writes little-endian byte order", "[helpers][emplace][u32]")
{
    uint8_t buf[4];

    SECTION("zero") {
        emplace_u32_le(buf, 0x00000000u);
        REQUIRE(buf[0] == 0x00u);
        REQUIRE(buf[1] == 0x00u);
        REQUIRE(buf[2] == 0x00u);
        REQUIRE(buf[3] == 0x00u);
    }
    SECTION("all ones") {
        emplace_u32_le(buf, 0xFFFFFFFFu);
        REQUIRE(buf[0] == 0xFFu);
        REQUIRE(buf[1] == 0xFFu);
        REQUIRE(buf[2] == 0xFFu);
        REQUIRE(buf[3] == 0xFFu);
    }
    SECTION("distinct bytes") {
        emplace_u32_le(buf, 0x12345678u);
        REQUIRE(buf[0] == 0x78u); // least significant first
        REQUIRE(buf[1] == 0x56u);
        REQUIRE(buf[2] == 0x34u);
        REQUIRE(buf[3] == 0x12u); // most significant last
    }
}


/// @test extract_u16_le reconstructs a uint16_t from little-endian bytes.
TEST_CASE("extract_u16_le reads little-endian byte order", "[helpers][extract][u16]")
{
    const uint8_t zero[2]     = { 0x00u, 0x00u };
    const uint8_t ones[2]     = { 0xFFu, 0xFFu };
    const uint8_t distinct[2] = { 0xCDu, 0xABu };

    REQUIRE(extract_u16_le(zero)     == 0x0000u);
    REQUIRE(extract_u16_le(ones)     == 0xFFFFu);
    REQUIRE(extract_u16_le(distinct) == 0xABCDu);
}


/// @test extract_u32_le reconstructs a uint32_t from little-endian bytes.
TEST_CASE("extract_u32_le reads little-endian byte order", "[helpers][extract][u32]")
{
    const uint8_t zero[4]     = { 0x00u, 0x00u, 0x00u, 0x00u };
    const uint8_t ones[4]     = { 0xFFu, 0xFFu, 0xFFu, 0xFFu };
    const uint8_t distinct[4] = { 0x78u, 0x56u, 0x34u, 0x12u };

    REQUIRE(extract_u32_le(zero)     == 0x00000000u);
    REQUIRE(extract_u32_le(ones)     == 0xFFFFFFFFu);
    REQUIRE(extract_u32_le(distinct) == 0x12345678u);
}


/// @test Round-trip: emplacing a value then extracting it returns the
///       original, for both widths across a spread of values.
TEST_CASE("emplace then extract round-trips the original value", "[helpers][roundtrip]")
{
    constexpr uint16_t k_u16_vals[] = { 0x0000u, 0x0001u, 0x00FFu, 0x0100u, 0xABCDu, 0xFFFFu };
    for (const uint16_t v : k_u16_vals) {
        uint8_t buf[2];
        emplace_u16_le(buf, v);
        REQUIRE(extract_u16_le(buf) == v);
    }

    constexpr uint32_t k_u32_vals[] = {
        0x00000000u, 0x00000001u, 0x000000FFu, 0x0000FF00u,
        0x00FF0000u, 0xFF000000u, 0x12345678u, 0xFFFFFFFFu,
    };
    for (const uint32_t v : k_u32_vals) {
        uint8_t buf[4];
        emplace_u32_le(buf, v);
        REQUIRE(extract_u32_le(buf) == v);
    }
}


/// @test Offset writes must not clobber neighbouring bytes. Emplace into the
///       middle of a sentinel-filled buffer and confirm only the targeted
///       bytes change, and that extracting from the same offset returns the
///       value just written.
TEST_CASE("emplace at an offset leaves neighbouring bytes untouched", "[helpers][offset]")
{
    SECTION("u16 in the middle") {
        uint8_t buf[6];
        for (uint8_t& b : buf) { b = 0xA5u; }

        emplace_u16_le(buf + 2, 0x1234u);

        REQUIRE(buf[0] == 0xA5u);
        REQUIRE(buf[1] == 0xA5u);
        REQUIRE(buf[2] == 0x34u);
        REQUIRE(buf[3] == 0x12u);
        REQUIRE(buf[4] == 0xA5u);
        REQUIRE(buf[5] == 0xA5u);

        REQUIRE(extract_u16_le(buf + 2) == 0x1234u);
    }

    SECTION("u32 in the middle") {
        uint8_t buf[8];
        for (uint8_t& b : buf) { b = 0xA5u; }

        emplace_u32_le(buf + 2, 0xDEADBEEFu);

        REQUIRE(buf[0] == 0xA5u);
        REQUIRE(buf[1] == 0xA5u);
        REQUIRE(buf[2] == 0xEFu);
        REQUIRE(buf[3] == 0xBEu);
        REQUIRE(buf[4] == 0xADu);
        REQUIRE(buf[5] == 0xDEu);
        REQUIRE(buf[6] == 0xA5u);
        REQUIRE(buf[7] == 0xA5u);

        REQUIRE(extract_u32_le(buf + 2) == 0xDEADBEEFu);
    }
}
