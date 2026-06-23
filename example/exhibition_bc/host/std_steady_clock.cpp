// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "std_steady_clock.h"


StdSteadyClock::StdSteadyClock()
    : m_origin(std::chrono::steady_clock::now())
{
}


uint32_t StdSteadyClock::now_ms() const
{
    using namespace std::chrono;
    auto const elapsed = duration_cast<milliseconds>(
        steady_clock::now() - m_origin);
    return static_cast<uint32_t>(elapsed.count() & 0xFFFFFFFFull);
}


uint32_t StdSteadyClock::now_bc_ms() const
{
    // The BC is the reference clock; BC time equals local time.
    return now_ms();
}


void StdSteadyClock::set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms)
{
    // The BC has no offset relative to itself.
    (void)whole_ms;
    (void)fractional_ms;
}


void StdSteadyClock::set_bc_discipline(const uint32_t anchor_ms, const int32_t whole_ms,
                                       const int32_t fractional_ms, const int32_t skew_ppb)
{
    // The BC is its own reference: no offset and no skew to apply.
    (void)anchor_ms;
    (void)whole_ms;
    (void)fractional_ms;
    (void)skew_ppb;
}


void StdSteadyClock::get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const
{
    whole_ms      = 0;
    fractional_ms = 0;
}


int32_t StdSteadyClock::bc_skew_ppb() const
{
    return 0;
}


bool StdSteadyClock::has_sync() const
{
    return true;
}


void StdSteadyClock::set_has_sync(const bool synchronized)
{
    // The BC's sync state is fixed (always synchronized to itself).
    (void)synchronized;
}

