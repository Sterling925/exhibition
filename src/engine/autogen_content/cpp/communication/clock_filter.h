// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef CLOCK_FILTER_H
#define CLOCK_FILTER_H

#include <stdint.h>

#include "sync.h"


/// @file clock_filter.h
/// @brief NTP-style sliding min-delay clock filter for the RT's sync samples.
/// @details
/// Each SYNC_FINAL exchange yields one (offset, path-delay) measurement. The
/// per-exchange delay is dominated by whatever queueing/handler asymmetry that
/// particular round trip happened to hit, and that same asymmetry biases the
/// offset it carries. NTP's insight is that the sample with the *lowest* delay
/// is the one least contaminated by such jitter, so its offset is the most
/// trustworthy estimate currently available.
///
/// This filter keeps a sliding register of the last @ref kStages samples and,
/// on demand, returns the one with the minimum path delay (ties broken toward
/// the most recent sample, so a clean constant-delay stream always advances).
/// The selected sample -- not the raw newest one -- is what the system handler
/// disciplines into now_bc_ms() via the OffsetSmoother. The filter does not
/// smooth or fit anything itself; it only *selects*, leaving the frequency
/// estimation to the OffsetSmoother downstream.
///
/// Like the OffsetSmoother and PhaseAnchor it is free of any clock or HAL
/// dependency and is exercised in isolation on the host. Offsets are carried in
/// the same packed thousandths-of-a-unit form as the rest of sync.{h,cpp}
/// (offset_pack_micro / offset_unpack_micro), so no precision is lost in the
/// round trip through the ring.
class ClockFilter {
    public:

    /// @brief Number of recent samples the filter selects across.
    /// @details Matches NTP's eight-stage clock filter: a window long enough to
    ///          almost always contain a low-jitter sample, short enough that the
    ///          selected sample is never badly stale at the sync cadence.
    static constexpr uint32_t kStages = 8u;

    /// @brief One selected sample, decoded back into (whole, fractional) pairs.
    struct Selected {
        uint32_t abscissa_ms;   ///< Local tick (T2) the sample was measured at.
        int32_t  offset_whole;  ///< Whole-millisecond offset, BC - RT (signed).
        int32_t  offset_frac;   ///< Fractional thousandths of the offset (signed).
        uint32_t delay_whole;   ///< Whole-millisecond one-way path delay (>= 0).
        uint32_t delay_frac;    ///< Fractional thousandths of the path delay.
    };

    ClockFilter() { reset(); }

    /// @brief Discard the register; the next select() has nothing to return.
    void reset()
    {
        m_count    = 0u;
        m_head     = 0u;
        m_next_seq = 0u;
    }

    /// @brief Fold one freshly measured sample into the register.
    /// @param abscissa_ms Local tick (T2) the sample was measured at.
    /// @param off_whole Whole-millisecond offset, BC - RT (signed).
    /// @param off_frac Fractional thousandths of the offset (signed, same sign as
    ///        @p off_whole).
    /// @param delay_whole Whole-millisecond one-way path delay (>= 0).
    /// @param delay_frac Fractional thousandths of the path delay (0..999).
    /// @details Evicts the oldest sample once the register is full. The insertion
    ///          sequence number is retained per slot so select() can break a
    ///          delay tie toward the most recently pushed sample even after the
    ///          ring has wrapped.
    void push(uint32_t abscissa_ms, int32_t off_whole, int32_t off_frac,
              uint32_t delay_whole, uint32_t delay_frac)
    {
        m_slot[m_head].abscissa_ms  = abscissa_ms;
        m_slot[m_head].offset_micro = offset_pack_micro(off_whole, off_frac);
        m_slot[m_head].delay_micro  = delay_whole * 1000u + delay_frac;
        m_slot[m_head].seq          = m_next_seq++;
        m_head = (m_head + 1u) % kStages;
        if (m_count < kStages) {
            ++m_count;
        }
    }

    /// @brief Select the minimum-delay sample currently held.
    /// @param[out] out The selected sample, decoded into (whole, fractional) pairs.
    /// @return false when the register is empty (nothing selected), true otherwise.
    /// @details Scans the in-register samples for the smallest packed delay; a tie
    ///          is resolved in favour of the larger insertion sequence (the most
    ///          recently pushed), so a constant-delay stream keeps selecting the
    ///          freshest sample and the downstream stale-output gate never trips.
    bool select(Selected& out) const
    {
        if (m_count == 0u) {
            return false;
        }
        uint32_t best = 0u;
        for (uint32_t i = 1u; i < m_count; ++i) {
            if (m_slot[i].delay_micro < m_slot[best].delay_micro ||
                (m_slot[i].delay_micro == m_slot[best].delay_micro &&
                 m_slot[i].seq > m_slot[best].seq)) {
                best = i;
            }
        }
        const Entry& e = m_slot[best];
        out.abscissa_ms = e.abscissa_ms;
        offset_unpack_micro(e.offset_micro, out.offset_whole, out.offset_frac);
        out.delay_whole = e.delay_micro / 1000u;
        out.delay_frac  = e.delay_micro % 1000u;
        return true;
    }

    /// @brief Number of samples currently held (<= kStages).
    uint32_t count() const { return m_count; }

    private:

    /// @brief One register slot: a sample plus its insertion order.
    struct Entry {
        uint32_t abscissa_ms;   ///< Local tick (T2) the sample was measured at.
        int32_t  offset_micro;  ///< Packed offset (thousandths of a unit, signed).
        uint32_t delay_micro;   ///< Packed one-way path delay (thousandths, >= 0).
        uint32_t seq;           ///< Monotonic insertion sequence (newest = largest).
    };

    Entry    m_slot[kStages];   ///< Ring of the most recent samples.
    uint32_t m_count;           ///< Samples currently held (<= kStages).
    uint32_t m_head;            ///< Next write index in the ring.
    uint32_t m_next_seq;        ///< Next insertion sequence number to assign.
};


#endif // CLOCK_FILTER_H
