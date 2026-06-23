// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef OFFSET_SMOOTHER_H
#define OFFSET_SMOOTHER_H

#include <stdint.h>

#include "sync.h"


/// @file offset_smoother.h
/// @brief Sliding-window clock-discipline estimator for the RT's BC clock offset.
/// @details
/// Each SYNC_FINAL exchange yields one instantaneous estimate of how far the RT
/// systick leads or lags the Bus Controller's clock. Because the two ends are
/// independent oscillators with slightly different rates, that offset is *not* a
/// constant -- it ramps at the clocks' relative skew (typically tens of ppm).
/// Storing the raw value, or smoothing it with a plain exponential moving average,
/// can only filter phase noise; neither can remove a frequency error, so the
/// corrected clock keeps drifting between syncs and the reported offset never
/// settles.
///
/// This estimator instead fits a straight line `offset ~= b + m*t` by ordinary
/// least squares over a sliding window of the most recent samples, where `t` is the
/// RT local tick the sample was measured at (the SYNC_PROBE receive instant T2) and
/// `offset` is the raw BC-minus-RT measurement. The slope `m` is the clock skew
/// (reported as parts-per-billion); the line predicts the offset at any instant, so
/// now_bc_ms() can apply a rate correction via apply_bc_discipline() (see sync.h)
/// and stay accurate between syncs instead of drifting.
///
/// The class is deliberately free of any clock or HAL dependency and is
/// dependency-injected into the system handler, so it can be exercised in isolation.
/// The most recent raw measurement is retained and exposed via raw().
class OffsetSmoother {
    public:

    /// @brief Number of samples retained in the regression window.
    /// @details The window must span enough wall-clock time for the skew-induced
    ///          offset change to rise above the +/-0.5 ms timestamp quantization: at
    ///          tens of ppm and a ~0.5 Hz sync rate, 32 samples (~1 minute) resolves
    ///          the slope well. A larger window yields a steadier skew estimate but
    ///          adapts more slowly; a smaller one tracks faster but is noisier.
    static constexpr uint32_t kWindow = 32u;

    /// @brief Minimum samples before a non-zero skew is reported.
    /// @details With fewer points the slope is dominated by quantization noise, so
    ///          the estimator reports the freshest offset with zero skew during cold
    ///          start -- matching the original seed-then-hold behavior.
    static constexpr uint32_t kMinForSkew = 4u;

    /// @brief Minimum local-time span the window must cover before a non-zero skew is
    ///        reported, in milliseconds.
    /// @details A least-squares slope over near-identical abscissae -- e.g. several syncs
    ///          issued back-to-back with no delay -- is ill-conditioned and yields a wild
    ///          skew that the SysTick governor would then chase. Requiring the samples to
    ///          span a real interval holds the skew at zero until the abscissae are
    ///          genuinely separated. At the demo's ~2 s sync cadence kMinForSkew samples
    ///          already exceed this, so it only suppresses degenerate bursts.
    static constexpr uint32_t kMinSpanForSkewMs = 5000u;

    /// @brief Minimum end-to-end offset motion the fitted line must explain, in
    ///        milliseconds, before a non-zero skew is reported (the SNR / noise floor).
    /// @details Each offset sample is built from whole-millisecond HAL_GetTick timestamps,
    ///          so it carries +/-0.5 ms quantization (one LSB). A least-squares slope is
    ///          only *sign*-trustworthy once the offset change it predicts across the whole
    ///          window, predicted_ramp = |slope| * window_span_ms, clears a few of those
    ///          LSBs. Below that the slope -- and crucially its sign -- is quantization
    ///          noise, which the SysTick governor would then chase the WRONG way (the
    ///          reported "adjusts in the wrong direction" symptom). 1.5 ms == 3 LSB. Unlike
    ///          the count/span gates above, this one adapts to the skew magnitude: a small,
    ///          slow skew (e.g. 30 ppm) must accumulate a long baseline (~50 s) before its
    ///          ramp clears the gate, while a large skew (e.g. 100 ppm) clears it in ~15 s
    ///          and locks sooner. It is the last of three gates: count (cold start) ->
    ///          span (conditioning) -> signal (this noise floor).
    static constexpr double kSkewSignalMs = 1.5;

    /// @brief One disciplined result: the offset at @ref anchor_ms, plus the skew.
    struct Discipline {
        uint32_t anchor_ms;    ///< Local tick the offset is referenced to (latest T2).
        int32_t  offset_whole; ///< Whole-millisecond offset at the anchor (signed).
        int32_t  offset_frac;  ///< Fractional thousandths of the offset (signed).
        int32_t  skew_ppb;     ///< Clock skew in parts-per-billion (BC gain per local ms).
    };

    OffsetSmoother()
    {
        reset();
        m_raw_whole = 0;
        m_raw_frac  = 0;
    }

    /// @brief Fold a newly measured offset into the window and refit the line.
    /// @param local_tick_t2 RT local tick the offset was measured at (the SYNC_PROBE
    ///        receive instant T2); the regression abscissa.
    /// @param meas_whole Whole part of the measured offset (signed time units).
    /// @param meas_frac Fractional thousandths of the measured offset (signed, same
    ///        sign as @p meas_whole).
    /// @param[out] out Disciplined offset (referenced to this newest sample) and the
    ///        estimated skew, ready to hand to IClock::set_bc_discipline().
    /// @details Until @ref kMinForSkew samples are present the result is the freshest
    ///          measured offset with zero skew, so cold start matches the original
    ///          seed-exact behavior. The raw measurement is always retained (raw()).
    void update(uint32_t local_tick_t2, int32_t meas_whole, int32_t meas_frac,
                Discipline& out)
    {
        m_raw_whole = meas_whole;
        m_raw_frac  = meas_frac;

        push(local_tick_t2, sync_decode_signed(meas_whole, meas_frac));

        out.anchor_ms = local_tick_t2;
        if (m_count < kMinForSkew || window_span_ms() < kMinSpanForSkewMs) {
            // Cold start, or a window whose samples are too closely spaced to resolve a
            // slope: report the freshest measurement, no rate term yet.
            out.offset_whole = meas_whole;
            out.offset_frac  = meas_frac;
            out.skew_ppb     = 0;
            return;
        }

        // Fit relative to the newest tick so the abscissae are small and bounded by
        // the window's time span (immune to a uint32 tick rollover). The predicted
        // offset at the anchor is then just the intercept (x_anchor == 0).
        double slope     = 0.0;
        double intercept = 0.0;
        fit(local_tick_t2, slope, intercept);

        // SNR gate: only trust the slope's sign once the offset change it predicts across
        // the window clears the +/-0.5 ms timestamp quantization by a few LSBs. Below this,
        // |slope| is noise and its sign is effectively random, so report the disciplined
        // intercept (phase) with ZERO skew rather than steer the governor the wrong way. A
        // larger true skew clears the gate at a shorter span (faster lock); a small one
        // waits for a longer baseline (correct). The intercept is applied either way, so
        // now_bc_ms() stays phase-aligned; only the unreliable rate term is withheld.
        const double span_ms        = static_cast<double>(window_span_ms());
        const double predicted_ramp = (slope >= 0.0 ? slope : -slope) * span_ms;

        offset_split_signed(intercept, out.offset_whole, out.offset_frac);
        out.skew_ppb = (predicted_ramp >= kSkewSignalMs)
                           ? round_to_i32(slope * 1.0e9)   // dimensionless rate -> ppb
                           : 0;
    }

    /// @brief Discard the window; the next update() restarts the fit.
    void reset()
    {
        m_count  = 0u;
        m_head   = 0u;
        m_seeded = false;
    }

    /// @brief Re-express the window's samples under a tick rate that changed by
    ///        @p rate_delta_ppb at local tick @p pivot_tick.
    /// @param pivot_tick Local tick at which the SysTick reload was reprogrammed
    ///        (the rate-change instant). Samples in the window are at or before it.
    /// @param rate_delta_ppb How much the offset slope changed at the pivot, in
    ///        parts-per-billion: (new_reload - old_reload) * 1e9 / old_reload.
    ///        Lengthening the tick (a larger reload) slows the local ms counter,
    ///        so the BC-minus-RT slope INCREASES by this amount.
    /// @details Without this, the window keeps pre-step samples that follow the
    ///          OLD slope, so for up to a full window after a reload step the
    ///          fitted skew is a stale blend -- which now_bc_ms() then applies on
    ///          top of the already-retuned hardware rate (the Path A / Path B
    ///          rate double-count). Pivoting every stored sample onto the new-rate
    ///          line, y_i += rate_delta * (tick_i - pivot) / 1e9, makes the very
    ///          next fit report the true residual slope. The sample AT the pivot
    ///          is unchanged (the offset there is rate-independent); pre-pivot
    ///          abscissae are negative, so a positive delta lowers older samples.
    void rebase_rate(uint32_t pivot_tick, int32_t rate_delta_ppb)
    {
        const double delta = static_cast<double>(rate_delta_ppb) / 1.0e9;
        for (uint32_t i = 0u; i < m_count; ++i) {
            const double x = static_cast<double>(
                static_cast<int32_t>(m_tick[i] - pivot_tick));   // wrap-safe, <= 0
            m_offset[i] += delta * x;
        }
    }

    /// @brief Has at least one measurement been folded in?
    bool seeded() const { return m_seeded; }

    /// @brief Most recent raw (instantaneous) measurement.
    /// @param[out] whole Whole part of the last measured offset.
    /// @param[out] frac Fractional thousandths of the last measured offset.
    void raw(int32_t& whole, int32_t& frac) const
    {
        whole = m_raw_whole;
        frac  = m_raw_frac;
    }

    private:

    /// @brief Append a sample, evicting the oldest once the window is full.
    void push(uint32_t tick, double offset_ms)
    {
        m_tick[m_head]   = tick;
        m_offset[m_head] = offset_ms;
        m_head = (m_head + 1u) % kWindow;
        if (m_count < kWindow) {
            ++m_count;
        }
        m_seeded = true;
    }

    /// @brief Local-time span covered by the samples currently in the window, in ms.
    /// @details Computed as max - min of the in-window ticks taken relative to the first
    ///          sample, using the same wrap-safe int32 subtraction as fit(), so a uint32
    ///          tick rollover inside the window is handled. Used to suppress an
    ///          ill-conditioned slope from near-identical (back-to-back) abscissae.
    uint32_t window_span_ms() const
    {
        if (m_count < 2u) {
            return 0u;
        }
        const uint32_t origin = m_tick[0];
        int32_t lo = 0;
        int32_t hi = 0;
        for (uint32_t i = 1u; i < m_count; ++i) {
            const int32_t x = static_cast<int32_t>(m_tick[i] - origin);
            if (x < lo) { lo = x; }
            if (x > hi) { hi = x; }
        }
        return static_cast<uint32_t>(hi - lo);
    }

    /// @brief Ordinary least-squares fit of offset against (tick - @p origin).
    /// @param origin Tick used as the relative-x zero (the newest sample).
    /// @param[out] slope_out Best-fit slope (skew, ms-offset per ms-local).
    /// @param[out] intercept_out Best-fit offset at x == 0 (i.e. at @p origin).
    /// @details Abscissae are taken relative to @p origin with wrap-safe int32
    ///          subtraction, so a uint32 tick rollover inside the window is handled.
    ///          Sums accumulate in double; with @ref kWindow small this is ample.
    void fit(uint32_t origin, double& slope_out, double& intercept_out) const
    {
        double sx = 0.0, sy = 0.0, sxx = 0.0, sxy = 0.0;
        const double n = static_cast<double>(m_count);
        for (uint32_t i = 0u; i < m_count; ++i) {
            const double x = static_cast<double>(
                static_cast<int32_t>(m_tick[i] - origin));
            const double y = m_offset[i];
            sx  += x;
            sy  += y;
            sxx += x * x;
            sxy += x * y;
        }
        const double denom = n * sxx - sx * sx;
        if (denom == 0.0) {           // Degenerate (all-equal x): report a flat line.
            slope_out     = 0.0;
            intercept_out = sy / n;
            return;
        }
        slope_out     = (n * sxy - sx * sy) / denom;
        intercept_out = (sy - slope_out * sx) / n;
    }

    /// @brief Round a double to the nearest int32, half away from zero.
    /// @details Kept local so this header pulls in no <cmath> dependency.
    static int32_t round_to_i32(double v)
    {
        return static_cast<int32_t>((v >= 0.0) ? (v + 0.5) : (v - 0.5));
    }

    /// @brief Ring buffer of sample abscissae (local ticks, ms).
    uint32_t m_tick[kWindow];

    /// @brief Ring buffer of sample ordinates (measured offset, ms).
    double m_offset[kWindow];

    /// @brief Number of samples currently in the window (<= kWindow).
    uint32_t m_count;

    /// @brief Next write index in the ring buffer.
    uint32_t m_head;

    /// @brief True once at least one sample has been folded in.
    bool m_seeded;

    /// @brief Whole part of the most recent raw measurement.
    int32_t m_raw_whole;

    /// @brief Fractional thousandths of the most recent raw measurement.
    int32_t m_raw_frac;
};


#endif // OFFSET_SMOOTHER_H
