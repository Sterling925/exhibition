// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef PHASE_ANCHOR_H
#define PHASE_ANCHOR_H

#include <stdint.h>
#include <cassert>

#include "sync.h"


/// @file phase_anchor.h
/// @brief Long-baseline skew estimator feeding ONLY the SYNC_ADJUST reload
///        governor (Path B, hardware).
/// @details
/// The sliding-window OffsetSmoother estimates skew over a FIXED baseline (its
/// window span, e.g. 32 samples at a ~1.4 s sync cadence is ~44 s) and gates the
/// slope to zero until the offset motion it predicts across that span clears
/// kSkewSignalMs (1.5 ms). That makes the smallest skew it can ever report
/// roughly kSkewSignalMs / window_span -- about 34 ppm at the cadence above --
/// while the reload governor's deadband is only ~6 ppm (one reload tick). Any
/// residual skew between those two floors is therefore permanently invisible to
/// the governor: the window slides, the baseline never grows, and the reload
/// stalls a few ticks short of lock.
///
/// This class closes that gap for the governor only. It snapshots one
/// disciplined offset sample (the smoother's intercept, already least-squares
/// smoothed) and lets the baseline GROW across telemetry phases until the
/// integrated offset motion clears a much smaller signal threshold. The average
/// skew over that baseline is then handed to next_tick_reload(). Because the
/// baseline keeps growing while the signal is below threshold, ANY real
/// residual skew eventually becomes measurable; phantom skews at the threshold
/// level are slower than the governor deadband, so noise can never move the
/// reload (see kSignalMicro).
///
/// Path A (the smoother's discipline of now_bc_ms()) is unaffected: this class
/// neither filters offsets nor reports a discipline -- it only answers the
/// once-per-phase question "what was the average skew since the last accepted
/// measurement?".
///
/// The class is deliberately free of any clock or HAL dependency so it can be
/// exercised in isolation on the host.
class PhaseAnchor {
    public:

    /// @brief Offset motion the baseline must show before the skew is trusted,
    ///        in thousandths of a millisecond (the packed micro unit).
    /// @details The samples fed in are the smoother's disciplined intercepts,
    ///          which are least-squares averages of a full window and carry only
    ///          ~0.1 ms of residual noise (vs the +/-0.5 ms quantization of a raw
    ///          sample), so 0.5 ms is a ~5x margin. It is also self-limiting
    ///          against phantom steps: at the ~91 s adjust cadence a
    ///          threshold-level artifact implies 0.5 ms / 91 s ~= 5.5 ppm, which
    ///          is inside the governor's ~6 ppm deadband -- noise at this level
    ///          can never move the reload.
    static constexpr int32_t kSignalMicro = 500;

    /// @brief Minimum baseline span, in milliseconds, before any estimate.
    /// @details Mirrors the smoother's kMinSpanForSkewMs conditioning floor: a
    ///          skew over a near-zero baseline is ill-conditioned.
    static constexpr int32_t kMinBaselineMs = 5000;

    /// @brief Baseline span, in milliseconds, after which a below-threshold
    ///        snapshot is dropped instead of kept.
    /// @details Less than kSignalMicro of motion over an hour is < 0.14 ppm --
    ///          some 40x inside the governor deadband, i.e. locked. Dropping the
    ///          snapshot there bounds the wrap-safe int32 anchor delta and the
    ///          staleness of the baseline; a re-seeded baseline re-detects any
    ///          new drift within a phase or two.
    static constexpr int32_t kMaxBaselineMs = 3600000;

    PhaseAnchor()
    {
        m_have_latest   = false;
        m_have_snapshot = false;
        m_latest_anchor = 0u;
        m_snap_anchor   = 0u;
        m_latest_micro  = 0;
        m_snap_micro    = 0;
    }

    /// @brief Record the latest disciplined offset point (call on every SYNC_FINAL).
    /// @param anchor_ms Local tick the offset is referenced to
    ///        (OffsetSmoother::Discipline::anchor_ms, the latest T2).
    /// @param offset_whole Whole-millisecond part of the disciplined offset (signed).
    /// @param offset_frac Fractional thousandths of the offset (signed, same sign
    ///        as @p offset_whole).
    /// @details The point is packed to micro units (thousandths of a ms) so the
    ///          delta math is exact integer arithmetic. If no baseline snapshot is
    ///          held (first call ever, or after an accepted measurement or an
    ///          invalidate_snapshot()), this point seeds the new baseline origin.
    void note_measurement(uint32_t anchor_ms, int32_t offset_whole, int32_t offset_frac)
    {
        const int32_t micro = offset_pack_micro(offset_whole, offset_frac);
        m_latest_anchor = anchor_ms;
        m_latest_micro  = micro;
        m_have_latest   = true;
        if (!m_have_snapshot) {
            m_snap_anchor   = anchor_ms;
            m_snap_micro    = micro;
            m_have_snapshot = true;
        }
    }

    /// @brief Produce the long-baseline average skew for one SYNC_ADJUST.
    /// @param[out] skew_ppb_out Average skew since the snapshot in parts-per-billion;
    ///        0 when the offset motion is still below kSignalMicro.
    /// @return true when an estimate exists (the caller feeds it to the governor,
    ///         possibly as 0); false when no snapshot is held or the baseline is
    ///         shorter than kMinBaselineMs (the caller should fall back to the
    ///         smoother's windowed skew, e.g. for the very first adjust).
    /// @details Snapshot lifecycle: below threshold the snapshot is KEPT so the
    ///          baseline keeps growing (the accumulation property that fixes the
    ///          stall) unless it has reached kMaxBaselineMs (locked; dropped).
    ///          Once the threshold is cleared, the measured skew is returned and
    ///          the snapshot is dropped so the next note_measurement() re-seeds
    ///          the baseline.
    bool adjust_skew_ppb(int32_t& skew_ppb_out)
    {
        skew_ppb_out = 0;
        if (!m_have_snapshot || !m_have_latest) {
            return false;
        }
        // Wrap-safe signed span of local time covered by the baseline. Anchors are
        // monotonic local ticks, so the span is non-negative by construction.
        const int32_t delta_t_ms = static_cast<int32_t>(m_latest_anchor - m_snap_anchor);
        assert(delta_t_ms >= 0);
        if (delta_t_ms < kMinBaselineMs) {
            return false;
        }

        const int64_t delta_off_micro =
            static_cast<int64_t>(m_latest_micro) - static_cast<int64_t>(m_snap_micro);
        const int64_t magnitude = (delta_off_micro >= 0) ? delta_off_micro
                                                         : -delta_off_micro;
        if (magnitude < static_cast<int64_t>(kSignalMicro)) {
            // Below the noise floor: report "no measurable skew" but KEEP the
            // snapshot so the baseline keeps growing -- a real residual integrates
            // more offset motion every phase and eventually clears the threshold.
            if (delta_t_ms >= kMaxBaselineMs) {
                invalidate_snapshot();   // locked-in; bound the baseline integers
            }
            return true;
        }

        // Average skew over the baseline. Units: (micro / 1000 ms) / delta_t ms
        // * 1e9 ppb == micro * 1e6 / delta_t. |delta_off_micro| < 2^32 so the
        // product fits int64 with ample headroom; rounded half away from zero
        // (same convention as next_tick_reload()).
        const int64_t num  = delta_off_micro * 1000000LL;
        const int64_t den  = static_cast<int64_t>(delta_t_ms);
        const int64_t half = den / 2;
        const int64_t skew = (num >= 0) ? (num + half) / den : (num - half) / den;
        skew_ppb_out = (skew > INT32_MAX) ? INT32_MAX
                     : (skew < INT32_MIN) ? INT32_MIN
                     : static_cast<int32_t>(skew);
        invalidate_snapshot();   // accepted: the next note_measurement() re-seeds
        return true;
    }

    /// @brief Drop the baseline snapshot; the next note_measurement() re-seeds it.
    /// @details Must also be called whenever the SysTick reload is actually
    ///          reprogrammed: the old baseline was measured under the old tick
    ///          rate and no longer describes the running clock.
    void invalidate_snapshot()
    {
        m_have_snapshot = false;
    }

    /// @brief True while a baseline snapshot is held (test / diagnostic hook).
    bool has_snapshot() const { return m_have_snapshot; }

    private:

    /// @brief True once note_measurement() has been called at least once.
    bool m_have_latest;

    /// @brief True while a baseline snapshot is held.
    bool m_have_snapshot;

    /// @brief Local tick of the most recent disciplined offset point.
    uint32_t m_latest_anchor;

    /// @brief Local tick of the baseline snapshot (the baseline origin).
    uint32_t m_snap_anchor;

    /// @brief Most recent disciplined offset, packed micro units (thousandths of a ms).
    int32_t m_latest_micro;

    /// @brief Snapshot offset, packed micro units (thousandths of a ms).
    int32_t m_snap_micro;
};


#endif // PHASE_ANCHOR_H
