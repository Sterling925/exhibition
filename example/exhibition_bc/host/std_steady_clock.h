// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef STD_STEADY_CLOCK_H
#define STD_STEADY_CLOCK_H

#include <chrono>

#include "../communication/i_clock.h"


/// @file std_steady_clock.h
/// @brief Desktop IClock implementation backed by std::chrono::steady_clock.
/// @details
/// now_ms() reports milliseconds since the clock instance was constructed,
/// truncated to 32 bits. The BusController does its deadline arithmetic with
/// wrap-safe subtraction so the truncation is harmless even across a wrap
/// (which takes ~49.7 days).
class StdSteadyClock : public IClock {
    public:

    /// @brief Construct; latches the origin instant.
    StdSteadyClock();
    ~StdSteadyClock() override = default;

    /// @brief Milliseconds since construction, modulo 2^32.
    uint32_t now_ms() const override;

    /// @brief Identical to now_ms(): the BC is its own time reference, so there
    ///        is no offset to apply.
    uint32_t now_bc_ms() const override;

    /// @brief No-op: the BC has no offset relative to itself.
    void set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms) override;

    /// @brief No-op: the BC is its own reference, so it has no offset or skew.
    void set_bc_discipline(const uint32_t anchor_ms, const int32_t whole_ms,
                           const int32_t fractional_ms, const int32_t skew_ppb) override;

    /// @brief Always reports a zero offset (BC is the reference clock).
    void get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const override;

    /// @brief Always zero: the BC has no skew relative to itself.
    int32_t bc_skew_ppb() const override;

    /// @brief Always true: the BC clock is, by definition, synchronized to itself.
    bool has_sync() const override;

    /// @brief No-op: the BC's sync state is fixed.
    void set_has_sync(const bool synchronized) override;

    private:

    std::chrono::steady_clock::time_point m_origin;
};


#endif // STD_STEADY_CLOCK_H
