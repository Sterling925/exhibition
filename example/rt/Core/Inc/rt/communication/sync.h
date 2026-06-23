// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SYNC_H
#define SYNC_H

#include <stdint.h>

/// @file sync.h
/// @brief Time-synchronization helpers (NTP four-timestamp offset/delay over the bus).
/// @details
/// Demonstrates that the Exhibition bus protocol can carry a clock-sync scheme.
/// The Bus Controller runs a two-command exchange against an RT's system handler
/// (sub-address 1):
///   - SYNC_PROBE : BC sends T1 (its clock at send); the RT records T2 (its clock
///                  when it received the probe) and T3 (its clock just before it
///                  sends the reply), and replies with both. The BC notes T4 (its
///                  clock when the reply arrives).
///   - SYNC_FINAL : BC sends T4; the RT computes and returns the path delay and
///                  clock offset from T1, T2, T3 and T4.
/// Keeping the two server-side instants distinct lets the RT's own handling time
/// be subtracted out -- the standard NTP form (see compute_sync_fixed):
///   path_delay = ((T4 - T1) - (T3 - T2)) / 2       // one-way (RTT/2)
///   offset     = ((T1 - T2) + (T4 - T3)) / 2       // BC clock minus RT clock
/// The offset is the correction the RT *adds* to its local tick to recover BC
/// time (see apply_bc_offset); it is positive when the BC clock leads the RT.
/// On the RT each exchange's (offset, delay) feeds a sliding 8-deep min-delay
/// clock filter (see clock_filter.h); the lowest-delay sample of the recent
/// window is the one disciplined into now_bc_ms(), the way NTP picks the least
/// jitter-contaminated measurement.
///
/// This module is intentionally free of any clock or HAL dependency: the
/// receive instants are passed in by the caller (SyncState), so the formulas
/// and serialization are unit-testable on the host.


/// @brief Time-unit code: the magnitude of the negative power of ten that one
///        unit represents in seconds (a unit equals pow(10, -value) seconds).
/// @details
/// Milliseconds == 3 because 1 ms == 10^-3 s. The code travels in every sync
/// command so both ends agree on the meaning of the integer timestamps and the
/// fixed-point results. Exhibition's clocks are millisecond systicks, so
/// `Millisecond` is what is used in practice; the field keeps the scheme
/// unambiguous if a higher-resolution clock is substituted later.
enum class TimeUnits : uint8_t {
    Centisecond = 2,
    Millisecond = 3,
    Microsecond = 6,
    Nanosecond  = 9,
    Picosecond  = 12,
};


/// @brief Wire sizes (in bytes) of the sync frames.
constexpr uint32_t kSyncCmdBytes        = 6;   ///< [opcode, units, uint32 value] for SYNC_PROBE / SYNC_FINAL.
constexpr uint32_t kSyncProbeRespBytes  = 8;   ///< T2 and T3 as two uint32s (NTP receive + send instants).
constexpr uint32_t kSyncFinalCoreBytes  = 16;  ///< Four words (path + offset, whole + frac) from on_final().
constexpr uint32_t kSyncFinalRespBytes  = 16;  ///< Four words (path + offset) == kSyncFinalCoreBytes; fits the inline payload slot.
constexpr uint32_t kSyncAdjustCmdBytes  = 1;   ///< SYNC_ADJUST command: opcode only (the RT acts on the telemetry it already gathered).
constexpr uint32_t kSyncAdjustRespBytes = 8;   ///< SYNC_ADJUST reply: averaged skew (int32) + applied SysTick reload (uint32), little-endian.


/// @brief SysTick-reload governor tuning (see next_tick_reload()).
/// @details The RT disciplines its SysTick reload toward the value that cancels the
///          phase-averaged BC-vs-RT skew, but the move is *smoothed*: each SYNC_ADJUST
///          applies only a fraction (kReloadGainNum/kReloadGainDen) of the gap to the ideal
///          reload, so the clocks converge over several telemetry/adjust cycles instead of
///          in one big jump. The result is always held within +/-kReloadBandPercent of
///          nominal.
constexpr int32_t  kReloadDeadbandPpb  = 6000;  ///< ~1 reload tick @168 MHz (1e9 / 168000 ≈ 5952 ppb/tick): skews below the timer's own resolution are ignored (stops dithering; covers cold start).
                                                 ///< NOTE: calibrated for 168 MHz. At a different SystemCoreClock the ppb per tick is
                                                 ///<   1e9 / (SystemCoreClock / 1000); resize this constant proportionally.
constexpr uint32_t kReloadGainNum      = 3u;    ///< Smoothing-gain numerator: each adjust moves kReloadGainNum/kReloadGainDen of the way to the ideal reload.
constexpr uint32_t kReloadGainDen      = 4u;    ///< Smoothing-gain denominator (gain = 3/4). The residual shrinks by (1 - gain) per phase, so 3/4 quarters it each cycle -- fast lock. This gain is safe because the OffsetSmoother SNR gate (kSkewSignalMs) only ever feeds the governor a trustworthy, correct-sign skew, so a bigger step can never be a confident step the WRONG way. Capped below 1/1 so a single adjust never reaches the ideal in one move (preserves the strict partial-step the reload e2e test relies on, and leaves headroom for the lagged-window estimate).
constexpr uint32_t kReloadBandPercent  = 10u;   ///< Hard clamp: reload may not leave +/- this percent of nominal.


/// @brief Fixed-point results of one synchronization exchange.
/// @details
/// Each quantity is a (whole, fractional) pair expressed in the exchange's time
/// units. The fractional part is thousandths of a unit (the remainder times
/// 1000), so a path delay of 0.5 ms encodes as {0, 500}. Path delay is
/// non-negative. The clock offset is the BC clock minus the RT clock (the
/// correction the RT adds to its local tick to recover BC time): positive when
/// the BC clock leads, negative when the RT systick leads. Its whole and
/// fractional parts are two's-complement and always carry the same sign, so a
/// value of -0.5 encodes unambiguously as {0, -500} even though the whole part
/// is zero.
struct SyncFixed {
    uint32_t path_delay_whole;  ///< Whole part of mean path delay (>= 0).
    uint32_t path_delay_frac;   ///< Fractional thousandths of path delay (0..999).
    int32_t  offset_whole;      ///< Whole part of clock offset, BC - RT (signed).
    int32_t  offset_frac;       ///< Fractional thousandths of offset (-999..999, sign matches whole).
};


/// @brief Compute path delay and clock offset from the four timestamps.
/// @param t1 BC clock when SYNC_PROBE was sent.
/// @param t2 RT clock when SYNC_PROBE was received.
/// @param t3 RT clock just before the SYNC_PROBE response was sent.
/// @param t4 BC clock when the SYNC_PROBE response arrived.
/// @return Fixed-point path delay and clock offset (BC - RT).
/// @details
/// NTP four-timestamp form: the one-way path_delay = ((t4 - t1) - (t3 - t2)) / 2
/// and the BC-minus-RT offset = ((t1 - t2) + (t4 - t3)) / 2. Removing (t3 - t2)
/// takes the RT's handling time out of the round trip. Timestamp differences are formed
/// with wrap-safe signed subtraction before conversion to double, so a
/// millisecond counter that rolls over between captures still yields the correct
/// (small) delta. The math runs in double for clarity and is then split into the
/// fixed-point pairs the wire format carries.
SyncFixed compute_sync_fixed(uint32_t t1, uint32_t t2, uint32_t t3, uint32_t t4);


/// @brief Reconstruct a non-negative fixed-point pair to a double.
/// @param whole Whole part in time units.
/// @param frac Fractional thousandths of a unit (0..999).
/// @return whole + frac / 1000.
double sync_decode_unsigned(uint32_t whole, uint32_t frac);


/// @brief Reconstruct a signed fixed-point pair to a double.
/// @param whole Whole part in time units (signed).
/// @param frac Fractional thousandths of a unit (signed, same sign as whole).
/// @return whole + frac / 1000.
double sync_decode_signed(int32_t whole, int32_t frac);


/// @brief Split a signed value into a (whole, thousandths-fractional) pair, both
///        truncated toward zero so they share one sign.
/// @param value Signed value in time units (e.g. a regression-predicted offset).
/// @param[out] whole_out Whole part in time units (signed).
/// @param[out] frac_out Fractional thousandths of a unit (signed, same sign as
///        @p whole_out).
/// @details Matches the SyncFixed sign invariant (e.g. -0.5 -> {0, -500}). Exposed
///          so the clock-discipline estimator can serialize a double offset into the
///          same fixed-point form the wire format and the clock store.
void offset_split_signed(double value, int32_t& whole_out, int32_t& frac_out);


/// @brief Apply a fixed-point BC clock offset to a local millisecond counter.
/// @param local_ms Local monotonic millisecond counter (e.g. IClock::now_ms()).
/// @param offset_whole Whole-millisecond part of the BC offset (signed).
/// @param offset_frac Thousandths-of-a-millisecond fractional part (signed, same
///        sign as @p offset_whole); used only to round to the nearest whole ms.
/// @return @p local_ms shifted by the offset.
/// @details
/// The shift is computed entirely with wrap-safe unsigned (modulo-2^32)
/// arithmetic, so it is well-defined for every input and handles the two awkward
/// cases correctly: a negative offset (BC clock behind the local systick) and a
/// counter rollover. Adding the unsigned reinterpretation of a negative value is
/// equivalent to subtracting it modulo 2^32, so no signed arithmetic (and its
/// overflow hazards) is involved. The fractional part nudges the result by one
/// millisecond when its magnitude reaches 500 thousandths (nearest-ms rounding).
uint32_t apply_bc_offset(uint32_t local_ms, int32_t offset_whole, int32_t offset_frac);


/// @brief Apply a rate-corrected (offset + skew) BC clock discipline to a local
///        millisecond counter.
/// @param local_ms Local monotonic millisecond counter (e.g. IClock::now_ms()).
/// @param anchor_ms Local counter value at which the offset was evaluated (the
///        discipline's reference instant; usually the T2 of the latest sync).
/// @param offset_whole Whole-millisecond part of the BC offset at the anchor (signed).
/// @param offset_frac Thousandths-of-a-millisecond fractional part at the anchor
///        (signed, same sign as @p offset_whole).
/// @param skew_ppb Clock skew in parts-per-billion: over @p elapsed milliseconds of
///        local time the offset grows by skew_ppb * elapsed / 1e9 milliseconds
///        (signed; positive when the BC clock runs faster than the local clock).
/// @return @p local_ms expressed on the BC clock: the phase shift of
///         apply_bc_offset() plus the frequency term, which keeps the estimate
///         accurate between syncs instead of drifting at the skew rate.
/// @details The frequency term uses wrap-safe elapsed-time arithmetic and is
///          evaluated in int64 (so it cannot overflow for any int32 inputs), rounded
///          to the nearest whole millisecond, and folded in with the same
///          modulo-2^32 add as apply_bc_offset(). With @p skew_ppb == 0 this is
///          exactly apply_bc_offset(local_ms, offset_whole, offset_frac).
uint32_t apply_bc_discipline(uint32_t local_ms, uint32_t anchor_ms,
                             int32_t offset_whole, int32_t offset_frac,
                             int32_t skew_ppb);


/// @brief Bounded SysTick-reload governor: take one smoothed partial step toward the
///        reload that cancels the phase-averaged BC-vs-RT skew.
/// @param current_reload Current SysTick reload as a full tick count (the
///        HAL_SYSTICK_Config argument, e.g. 168000 -- not the LOAD-1 register value).
/// @param nominal_reload Nominal reload (SystemCoreClock / 1000 on target); defines the
///        +/-kReloadBandPercent clamp. A nominal of 0 makes the call a no-op.
/// @param skew_ppb Phase-averaged skew estimate (BC gain per local ms, see
///        offset_smoother.h). Positive means the BC gains on the RT (the RT runs slow),
///        so the RT must tick faster and the reload is DECREASED; negative lengthens it.
/// @return The reload to program next: @p current_reload moved a fraction
///         (kReloadGainNum/kReloadGainDen) of the way toward round(current * (1 - skew_ppb /
///         1e9)) -- a smoothed partial step, not the full correction, so the clocks
///         converge over several telemetry/adjust cycles -- then clamped to
///         +/-kReloadBandPercent of @p nominal_reload, and left unchanged entirely when
///         |skew_ppb| < kReloadDeadbandPpb. Pure integer math (no HAL), host-unit-tested.
uint32_t next_tick_reload(uint32_t current_reload, uint32_t nominal_reload, int32_t skew_ppb);


/// @brief Pack a signed (whole, thousandths-fractional) offset into a single
///        signed micro-unit value.
/// @param whole Whole part in time units (signed).
/// @param frac Fractional thousandths of a unit (signed, same sign as @p whole).
/// @return whole * 1000 + frac (i.e. the offset expressed in thousandths of a
///         unit). Combining the two fields lets the smoothing filter operate on
///         one coherent quantity instead of averaging whole and fractional parts
///         independently (which loses the carry between them and can flip sign at
///         a whole-unit boundary).
int32_t offset_pack_micro(int32_t whole, int32_t frac);


/// @brief Unpack a signed micro-unit value back into a (whole, fractional) pair.
/// @param micro Offset in thousandths of a unit (as produced by offset_pack_micro).
/// @param[out] whole_out Whole part in time units (signed).
/// @param[out] frac_out Fractional thousandths (signed, same sign as @p whole_out).
/// @details Truncating integer division and remainder give the whole and
///          fractional parts the same sign, matching the SyncFixed invariant.
void offset_unpack_micro(int32_t micro, int32_t& whole_out, int32_t& frac_out);


/// @brief RT-side state for one in-progress synchronization exchange.
/// @details
/// The two sync commands arrive as separate transactions, so the RT caches the
/// instants it observes (T2 receive, T3 response-send) plus the BC-supplied T1
/// between them. The caller (the system handler) passes the receive instant
/// `rx_now` and the send instant `tx_now`, keeping this class free of any clock
/// or HAL dependency. Each `on_*()` method serializes its response little-endian
/// into `resp` and returns the response length in bytes. The members persist
/// between exchanges; only a SYNC_FINAL consumes them, so an interrupted exchange
/// simply gets overwritten by the next SYNC_PROBE.
class SyncState {
    public:

    SyncState();

    /// @brief Handle SYNC_PROBE: cache units and T1, capture T2 = rx_now and
    ///        T3 = tx_now.
    /// @param units Time-unit code supplied by the BC.
    /// @param t1 BC clock at send, from the command payload.
    /// @param rx_now RT clock at receipt (becomes T2).
    /// @param tx_now RT clock just before the reply is serialized (becomes T3).
    /// @param resp Response buffer; T2 then T3 are written as uint32 little-endian.
    /// @return Response length in bytes (kSyncProbeRespBytes).
    uint32_t on_probe(uint8_t units, uint32_t t1, uint32_t rx_now, uint32_t tx_now,
                      uint8_t* resp);

    /// @brief Handle SYNC_FINAL: compute results from cached T1, T2, T3 and T4.
    /// @param t4 BC clock when the SYNC_PROBE response arrived, from the payload.
    /// @param resp Response buffer; the four SyncFixed words are written LE.
    /// @param[out] result_out The computed fixed-point path delay and offset,
    ///        so the caller can apply the offset (e.g. store it in the clock)
    ///        without re-parsing the serialized response.
    /// @return Number of core bytes written (kSyncFinalCoreBytes). The system handler
    ///         overwrites the two offset words with the disciplined offset it applied
    ///         and reports kSyncFinalRespBytes (equal to kSyncFinalCoreBytes; no reload
    ///         word is appended).
    uint32_t on_final(uint32_t t4, uint8_t* resp, SyncFixed& result_out);

    /// @brief Time-unit code last supplied by the BC (defaults to Millisecond).
    uint8_t units() const { return m_units; }

    /// @brief RT receive instant (T2) cached by the most recent SYNC_PROBE.
    /// @details The clock-discipline estimator uses this as the regression abscissa
    ///          -- the local tick the offset sample was measured at.
    uint32_t t2() const { return m_t2; }

    /// @brief RT response-send instant (T3) cached by the most recent SYNC_PROBE.
    uint32_t t3() const { return m_t3; }

    private:

    uint8_t  m_units;   ///< Time-unit code from the most recent SYNC_PROBE.
    uint32_t m_t1;      ///< BC send instant from the most recent SYNC_PROBE.
    uint32_t m_t2;      ///< RT receive instant of the most recent SYNC_PROBE.
    uint32_t m_t3;      ///< RT response-send instant of the most recent SYNC_PROBE.
};


#endif // SYNC_H
