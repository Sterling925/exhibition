// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_clock_filter.cpp
/// @brief Unit tests for the sliding min-delay clock filter.
/// @details
/// Exercises communication/clock_filter.h in isolation: min-delay selection,
/// the newest-on-ties tie-break, eviction once the eight-stage ring wraps, and
/// the offset (whole, fractional) round trip through the packed register. No
/// clock or HAL dependency, so these run on the host under catch2.

#include "thirdparty/catch_amalgamated.hpp"

#include "../communication/clock_filter.h"


namespace {

/// @brief Push a whole-millisecond sample (no fractional parts) for brevity.
void push_ms(ClockFilter& f, uint32_t abscissa, int32_t offset, uint32_t delay)
{
    f.push(abscissa, offset, 0, delay, 0u);
}

} // namespace


/// An empty filter selects nothing.
TEST_CASE("ClockFilter: empty register selects nothing", "[clock_filter]") {
    ClockFilter f;
    ClockFilter::Selected sel;
    REQUIRE(f.count() == 0u);
    REQUIRE_FALSE(f.select(sel));
}

/// A single sample is always the selection, decoded back intact.
TEST_CASE("ClockFilter: a single sample is selected verbatim", "[clock_filter]") {
    ClockFilter f;
    f.push(1000u, -3, -500, 2u, 250u);   // offset -3.5 ms, delay 2.25 ms
    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    REQUIRE(sel.abscissa_ms  == 1000u);
    REQUIRE(sel.offset_whole == -3);
    REQUIRE(sel.offset_frac  == -500);
    REQUIRE(sel.delay_whole  == 2u);
    REQUIRE(sel.delay_frac   == 250u);
}

/// Among several samples the one with the smallest path delay wins, regardless
/// of insertion order, and it carries its own abscissa and offset.
TEST_CASE("ClockFilter: selects the minimum-delay sample", "[clock_filter]") {
    ClockFilter f;
    push_ms(f, 100u, 10, 9u);    // high delay
    push_ms(f, 200u, 11, 3u);    // <-- lowest delay
    push_ms(f, 300u, 12, 7u);    // middling
    push_ms(f, 400u, 13, 5u);

    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    REQUIRE(sel.delay_whole  == 3u);
    REQUIRE(sel.abscissa_ms  == 200u);
    REQUIRE(sel.offset_whole == 11);
}

/// Fractional thousandths participate in the delay comparison, so a sample that
/// is smaller only in its fractional part still wins.
TEST_CASE("ClockFilter: fractional delay breaks the ordering", "[clock_filter]") {
    ClockFilter f;
    f.push(10u, 1, 0, 4u, 300u);   // 4.300 ms
    f.push(20u, 2, 0, 4u, 100u);   // 4.100 ms  <-- lower
    f.push(30u, 3, 0, 4u, 900u);   // 4.900 ms

    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    REQUIRE(sel.abscissa_ms == 20u);
    REQUIRE(sel.delay_whole == 4u);
    REQUIRE(sel.delay_frac  == 100u);
}

/// Equal delays are broken toward the most recently pushed sample, so a
/// constant-delay stream always reports the freshest measurement.
TEST_CASE("ClockFilter: equal delays prefer the newest sample", "[clock_filter]") {
    ClockFilter f;
    push_ms(f, 100u, 1, 5u);
    push_ms(f, 200u, 2, 5u);
    push_ms(f, 300u, 3, 5u);   // newest, same delay

    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    REQUIRE(sel.abscissa_ms  == 300u);
    REQUIRE(sel.offset_whole == 3);
}

/// Once more than kStages samples are pushed the oldest are evicted, so a low
/// delay that has scrolled out of the window no longer wins.
TEST_CASE("ClockFilter: evicts samples beyond the eight-stage window", "[clock_filter]") {
    ClockFilter f;
    // The very first sample has the globally lowest delay (1 ms)...
    push_ms(f, 0u, 0, 1u);
    // ...followed by a full window of higher-delay samples that push it out.
    for (uint32_t i = 1u; i <= ClockFilter::kStages; ++i) {
        push_ms(f, i * 10u, static_cast<int32_t>(i), 5u + i);
    }
    REQUIRE(f.count() == ClockFilter::kStages);

    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    // Nine samples were pushed, so only the very first (the 1 ms sample) is
    // evicted. The lowest *remaining* delay is 6 ms (i == 1 -> delay 5 + 1), at
    // abscissa 10 -- not the scrolled-out 1 ms sample at abscissa 0.
    REQUIRE(sel.delay_whole == 6u);
    REQUIRE(sel.abscissa_ms == 10u);
    REQUIRE(sel.abscissa_ms != 0u);
}

/// After wrap, the newest-on-ties tie-break still resolves to a sample inserted
/// in the wrapped region (sequence numbers, not slot indices, order recency).
TEST_CASE("ClockFilter: tie-break survives a ring wrap", "[clock_filter]") {
    ClockFilter f;
    // Fill, then overwrite the whole ring with equal-delay samples; the last one
    // pushed must win the tie even though it lands in an early slot.
    for (uint32_t i = 0u; i < 2u * ClockFilter::kStages; ++i) {
        push_ms(f, 1000u + i, static_cast<int32_t>(i), 4u);
    }
    ClockFilter::Selected sel;
    REQUIRE(f.select(sel));
    const uint32_t last_abscissa = 1000u + (2u * ClockFilter::kStages - 1u);
    REQUIRE(sel.abscissa_ms == last_abscissa);
}
