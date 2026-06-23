// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_sync.cpp
/// @brief Unit tests for the time-synchronization formulas and message parsing.
/// @details
/// Exercises communication/sync.{h,cpp} in isolation: the PTP fixed-point
/// computation (compute_sync_fixed), the decode helpers, and the RT-side
/// SyncState message flow. No clock or HAL dependency — receive instants are
/// supplied directly — so these run on the host under catch2.

#include "thirdparty/catch_amalgamated.hpp"

#include "../communication/sync.h"
#include "../communication/offset_smoother.h"
#include "../communication/phase_anchor.h"
#include "../communication/i_clock.h"
#include "../common/helper_functions.h"


namespace {

/// @brief Host-side IClock double that shares the SysTickClock discipline math via
///        apply_bc_discipline(), so the storage-and-read contract the system handler
///        relies on (offset + skew) can be exercised without the HAL.
class FakeClock : public IClock {
    public:
    uint32_t m_now_ms = 0;

    uint32_t now_ms() const override { return m_now_ms; }

    uint32_t now_bc_ms() const override
    {
        return apply_bc_discipline(m_now_ms, m_anchor_ms, m_off_whole, m_off_frac, m_skew_ppb);
    }

    void set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms) override
    {
        m_off_whole = whole_ms;
        m_off_frac  = fractional_ms;
        m_anchor_ms = m_now_ms;
        m_skew_ppb  = 0;
    }

    void set_bc_discipline(const uint32_t anchor_ms, const int32_t whole_ms,
                           const int32_t fractional_ms, const int32_t skew_ppb) override
    {
        m_anchor_ms = anchor_ms;
        m_off_whole = whole_ms;
        m_off_frac  = fractional_ms;
        m_skew_ppb  = skew_ppb;
    }

    void get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const override
    {
        whole_ms      = m_off_whole;
        fractional_ms = m_off_frac;
    }

    int32_t bc_skew_ppb() const override { return m_skew_ppb; }

    bool has_sync() const override { return m_has_sync; }

    void set_has_sync(const bool synchronized) override { m_has_sync = synchronized; }

    private:
    int32_t  m_off_whole = 0;
    int32_t  m_off_frac  = 0;
    uint32_t m_anchor_ms = 0;
    int32_t  m_skew_ppb  = 0;
    bool     m_has_sync  = false;
};

} // namespace


///////////////////////////////////////////////////////////////////////////////
// compute_sync_fixed: the NTP four-timestamp path-delay / offset formula
///////////////////////////////////////////////////////////////////////////////

/// Whole-number case with a synchronous server (T3 == T2 -> reduces to the
/// classic three-timestamp form). T1=100, T2=T3=150, T4=210.
///   path_delay = ((210-100) - 0) / 2 = 55
///   offset     = ((100-150) + (210-150)) / 2 = (-50 + 60)/2 = 5   (BC - RT)
TEST_CASE("compute_sync_fixed: exact whole-number delay and offset", "[sync][formula]") {
    const SyncFixed r = compute_sync_fixed(100, 150, 150, 210);
    REQUIRE(r.path_delay_whole == 55u);
    REQUIRE(r.path_delay_frac  == 0u);
    REQUIRE(r.offset_whole     == 5);
    REQUIRE(r.offset_frac      == 0);
}

/// Half-unit case producing a fractional result and a positive whole-zero
/// offset. T1=0, T2=T3=0, T4=1:
///   path_delay = ((1-0) - 0) / 2 = 0.5 -> {0, 500}
///   offset     = ((0-0) + (1-0)) / 2 = 0.5 -> {0, +500}
/// The sign must survive even though the whole part is zero.
TEST_CASE("compute_sync_fixed: half-unit fraction and signed whole-zero offset", "[sync][formula]") {
    const SyncFixed r = compute_sync_fixed(0, 0, 0, 1);
    REQUIRE(r.path_delay_whole == 0u);
    REQUIRE(r.path_delay_frac  == 500u);
    REQUIRE(r.offset_whole     == 0);
    REQUIRE(r.offset_frac      == 500);
}

/// Positive offset with a non-zero whole part. T1=0, T2=T3=0, T4=3:
///   path_delay = ((3-0) - 0) / 2 = 1.5 -> {1, 500}
///   offset     = ((0-0) + (3-0)) / 2 = 1.5 -> {1, +500}   (BC leads the RT)
TEST_CASE("compute_sync_fixed: positive offset with whole and fractional parts", "[sync][formula]") {
    const SyncFixed r = compute_sync_fixed(0, 0, 0, 3);
    REQUIRE(r.path_delay_whole == 1u);
    REQUIRE(r.path_delay_frac  == 500u);
    REQUIRE(r.offset_whole     == 1);
    REQUIRE(r.offset_frac      == 500);
}

/// Negative whole offset (RT leads the BC). T1=0, T2=T3=3, T4=4:
///   path_delay = ((4-0) - 0) / 2 = 2 -> {2, 0}
///   offset     = ((0-3) + (4-3)) / 2 = (-3 + 1)/2 = -1 -> {-1, 0}
TEST_CASE("compute_sync_fixed: negative whole offset", "[sync][formula]") {
    const SyncFixed r = compute_sync_fixed(0, 3, 3, 4);
    REQUIRE(r.path_delay_whole == 2u);
    REQUIRE(r.path_delay_frac  == 0u);
    REQUIRE(r.offset_whole     == -1);
    REQUIRE(r.offset_frac      == 0);
}

/// A non-zero server hold time (T3 > T2) is subtracted out of the round trip and
/// shifts the offset toward the reverse leg. T1=0, T2=10, T3=30, T4=50:
///   path_delay = ((50-0) - (30-10)) / 2 = (50 - 20)/2 = 15
///   offset     = ((0-10) + (50-30)) / 2 = (-10 + 20)/2 = 5   (BC - RT)
/// The three-timestamp form (ignoring T3) would have read delay 25, offset 15 --
/// so subtracting the 20 ms hold time is what corrects both.
TEST_CASE("compute_sync_fixed: server hold time (T3 > T2) is removed", "[sync][formula]") {
    const SyncFixed r = compute_sync_fixed(0, 10, 30, 50);
    REQUIRE(r.path_delay_whole == 15u);
    REQUIRE(r.path_delay_frac  == 0u);
    REQUIRE(r.offset_whole     == 5);
    REQUIRE(r.offset_frac      == 0);
}

/// Wrap-safe deltas: timestamps straddle the uint32 rollover but the true
/// (small) differences still yield path_delay=13, offset=5 (synchronous server).
/// T1=0xFFFFFFF0, T2=T3=0xFFFFFFF8, T4=0x0A: T4-T1=26 -> delay 13; the
/// BC-minus-RT offset = ((T1-T2) + (T4-T3))/2 = (-8 + 18)/2 = 5.
TEST_CASE("compute_sync_fixed: tolerates uint32 counter wraparound", "[sync][formula][wrap]") {
    const SyncFixed r = compute_sync_fixed(0xFFFFFFF0u, 0xFFFFFFF8u, 0xFFFFFFF8u, 0x0000000Au);
    REQUIRE(r.path_delay_whole == 13u);
    REQUIRE(r.path_delay_frac  == 0u);
    REQUIRE(r.offset_whole     == 5);
    REQUIRE(r.offset_frac      == 0);
}


///////////////////////////////////////////////////////////////////////////////
// decode helpers
///////////////////////////////////////////////////////////////////////////////

TEST_CASE("sync_decode_unsigned / sync_decode_signed reconstruct doubles", "[sync][decode]") {
    REQUIRE(sync_decode_unsigned(50u, 0u)   == Catch::Approx(50.0));
    REQUIRE(sync_decode_unsigned(0u, 500u)  == Catch::Approx(0.5));
    REQUIRE(sync_decode_unsigned(1u, 500u)  == Catch::Approx(1.5));
    REQUIRE(sync_decode_signed(0, -500)     == Catch::Approx(-0.5));
    REQUIRE(sync_decode_signed(-1, -500)    == Catch::Approx(-1.5));
    REQUIRE(sync_decode_signed(1, 500)      == Catch::Approx(1.5));
}

/// Encoder/decoder round-trip: every compute_sync_fixed output decodes back to
/// the doubles the formula produced.
TEST_CASE("sync fixed-point encode/decode round-trips", "[sync][decode]") {
    const SyncFixed r = compute_sync_fixed(0, 0, 0, 3);   // delay 1.5, offset +1.5
    REQUIRE(sync_decode_unsigned(r.path_delay_whole, r.path_delay_frac) == Catch::Approx(1.5));
    REQUIRE(sync_decode_signed(r.offset_whole, r.offset_frac)           == Catch::Approx(1.5));
}


///////////////////////////////////////////////////////////////////////////////
// apply_bc_offset: folding a signed offset into a uint32 millisecond counter
///////////////////////////////////////////////////////////////////////////////

/// A zero offset is a no-op, and a positive whole offset advances the counter.
TEST_CASE("apply_bc_offset: zero and positive whole offsets", "[sync][offset]") {
    REQUIRE(apply_bc_offset(1000u, 0, 0)   == 1000u);
    REQUIRE(apply_bc_offset(1000u, 5, 0)   == 1005u);
    REQUIRE(apply_bc_offset(0u, 250, 0)    == 250u);
}

/// A negative offset must subtract: this is the case that naive signed casting
/// or a signed addition would get wrong. The unsigned modular arithmetic yields
/// the correct earlier timestamp.
TEST_CASE("apply_bc_offset: negative whole offset subtracts", "[sync][offset]") {
    REQUIRE(apply_bc_offset(1000u, -1, 0)   == 999u);
    REQUIRE(apply_bc_offset(1000u, -250, 0) == 750u);
    REQUIRE(apply_bc_offset(500u, -500, 0)  == 0u);
}

/// The thousandths-fractional part rounds to the nearest whole millisecond:
/// magnitude >= 500 nudges by one ms in the matching direction; below 500 is
/// dropped.
TEST_CASE("apply_bc_offset: fractional part rounds to nearest ms", "[sync][offset]") {
    REQUIRE(apply_bc_offset(1000u, 0, 500)   == 1001u);  // +0.5 rounds up
    REQUIRE(apply_bc_offset(1000u, 0, 999)   == 1001u);
    REQUIRE(apply_bc_offset(1000u, 0, 499)   == 1000u);  // below threshold
    REQUIRE(apply_bc_offset(1000u, 0, -500)  == 999u);   // -0.5 rounds down
    REQUIRE(apply_bc_offset(1000u, 0, -499)  == 1000u);
    // Whole and fractional combine: -2.5 ms.
    REQUIRE(apply_bc_offset(1000u, -2, -500) == 997u);
    // +1.5 ms.
    REQUIRE(apply_bc_offset(1000u, 1, 500)   == 1002u);
}

/// The shift is wrap-safe across the uint32 rollover in both directions.
TEST_CASE("apply_bc_offset: tolerates uint32 wraparound", "[sync][offset][wrap]") {
    REQUIRE(apply_bc_offset(0u, -1, 0)          == 0xFFFFFFFFu);  // 0 - 1 wraps down
    REQUIRE(apply_bc_offset(0xFFFFFFFFu, 1, 0)  == 0u);           // max + 1 wraps up
    REQUIRE(apply_bc_offset(0u, 0, -500)        == 0xFFFFFFFFu);  // rounding underflow wraps
}


///////////////////////////////////////////////////////////////////////////////
// SyncState: the RT-side message flow and serialization
///////////////////////////////////////////////////////////////////////////////

/// Drive the two commands through SyncState with explicit receive/send instants,
/// mirroring the whole-number formula case, and check every serialized field.
TEST_CASE("SyncState serializes T2, T3 and the final fixed-point result", "[sync][state]") {
    SyncState st;
    uint8_t resp[kSyncFinalRespBytes] = {0};

    // SYNC_PROBE: units=Millisecond, T1=100, rx (T2) = 150, tx (T3) = 150
    // (synchronous server, so the formula reduces to the three-timestamp case).
    uint32_t len = st.on_probe(static_cast<uint8_t>(TimeUnits::Millisecond), 100u, 150u, 150u, resp);
    REQUIRE(len == kSyncProbeRespBytes);
    REQUIRE(extract_u32_le(resp)     == 150u);   // T2
    REQUIRE(extract_u32_le(resp + 4) == 150u);   // T3
    REQUIRE(st.units() == static_cast<uint8_t>(TimeUnits::Millisecond));
    REQUIRE(st.t2() == 150u);
    REQUIRE(st.t3() == 150u);

    // SYNC_FINAL: T4 = 210. Expect delay 55, offset 5 (BC - RT).
    SyncFixed result;
    len = st.on_final(210u, resp, result);
    REQUIRE(len == kSyncFinalCoreBytes);   // on_final writes the core words; the handler appends the reload word
    REQUIRE(extract_u32_le(resp)      == 55u);   // path_delay_whole
    REQUIRE(extract_u32_le(resp + 4)  == 0u);    // path_delay_frac
    REQUIRE(static_cast<int32_t>(extract_u32_le(resp + 8))  == 5);   // offset_whole
    REQUIRE(static_cast<int32_t>(extract_u32_le(resp + 12)) == 0);   // offset_frac

    // The out-parameter mirrors the serialized result so the caller (the system
    // handler) can apply the offset without re-parsing the response bytes.
    REQUIRE(result.path_delay_whole == 55u);
    REQUIRE(result.path_delay_frac  == 0u);
    REQUIRE(result.offset_whole     == 5);
    REQUIRE(result.offset_frac      == 0);
}

/// A second exchange overwrites the cached timestamps, so SyncState can run
/// back-to-back syncs without a reset. Here T1=1000, T2=1002, T4=1007 ->
/// delay = (1007-1000)/2 = 3.5, offset = (1000+1007)/2 - 1002 = 1.5.
TEST_CASE("SyncState can be reused for a second exchange", "[sync][state]") {
    SyncState st;
    uint8_t resp[kSyncFinalRespBytes] = {0};

    SyncFixed result;
    st.on_probe(static_cast<uint8_t>(TimeUnits::Millisecond), 1u, 1u, 1u, resp);   // junk first run
    st.on_final(1u, resp, result);

    st.on_probe(static_cast<uint8_t>(TimeUnits::Millisecond), 1000u, 1002u, 1002u, resp);
    const uint32_t len = st.on_final(1007u, resp, result);
    REQUIRE(len == kSyncFinalCoreBytes);   // on_final writes the core words; the handler appends the reload word
    REQUIRE(extract_u32_le(resp)     == 3u);     // path_delay_whole
    REQUIRE(extract_u32_le(resp + 4) == 500u);   // path_delay_frac
    REQUIRE(static_cast<int32_t>(extract_u32_le(resp + 8))  == 1);   // offset_whole
    REQUIRE(static_cast<int32_t>(extract_u32_le(resp + 12)) == 500); // offset_frac
    REQUIRE(result.offset_whole == 1);           // out-param matches the serialized offset
    REQUIRE(result.offset_frac  == 500);
}


///////////////////////////////////////////////////////////////////////////////
// Offset storage: the contract the system handler implements on SYNC_FINAL
///////////////////////////////////////////////////////////////////////////////

/// Mirror the system handler's SYNC_FINAL step: run an exchange, take the
/// on_final out-parameter, and store it in the clock. The clock must then report
/// the offset, flag itself synchronized, and fold the offset into now_bc_ms().
/// T1=0, T2=0, T4=1 yields offset {0, +500} (BC - RT), which rounds now_bc_ms()
/// up by one millisecond. A flipped offset sign would store {0, -500} and run the
/// corrected clock one millisecond slow -- the sign this test pins down.
TEST_CASE("SYNC_FINAL offset is stored in the clock and shifts now_bc_ms", "[sync][clock]") {
    SyncState st;
    FakeClock clock;
    clock.m_now_ms = 1000u;
    uint8_t resp[kSyncFinalRespBytes] = {0};

    REQUIRE_FALSE(clock.has_sync());
    REQUIRE(clock.now_bc_ms() == 1000u);   // no offset applied before sync

    st.on_probe(static_cast<uint8_t>(TimeUnits::Millisecond), 0u, 0u, 0u, resp);
    SyncFixed result;
    st.on_final(1u, resp, result);

    // The system handler applies the offset exactly like this.
    clock.set_bc_offset(result.offset_whole, result.offset_frac);
    clock.set_has_sync(true);

    REQUIRE(clock.has_sync());
    int32_t whole = 0;
    int32_t frac  = 0;
    clock.get_bc_offset(whole, frac);
    REQUIRE(whole == 0);
    REQUIRE(frac  == 500);
    // +500 thousandths rounds to +1 ms: 1000 + 1 = 1001.
    REQUIRE(clock.now_bc_ms() == 1001u);
}


///////////////////////////////////////////////////////////////////////////////
// Micro-unit helpers: the fixed-point representation the smoother filters on
///////////////////////////////////////////////////////////////////////////////

/// pack/unpack must round-trip and keep the whole and fractional parts sharing
/// one sign, for positive and negative offsets alike.
TEST_CASE("offset pack/unpack round-trips with a single sign", "[sync][smooth]") {
    int32_t whole = 0;
    int32_t frac  = 0;

    REQUIRE(offset_pack_micro(1, 250) == 1250);
    offset_unpack_micro(1250, whole, frac);
    REQUIRE(whole == 1);
    REQUIRE(frac  == 250);

    // Negative offset: both parts come back negative (matches SyncFixed).
    REQUIRE(offset_pack_micro(-1, -250) == -1250);
    offset_unpack_micro(-1250, whole, frac);
    REQUIRE(whole == -1);
    REQUIRE(frac  == -250);

    // Sub-unit magnitude keeps a zero whole and a signed fractional part.
    offset_unpack_micro(-500, whole, frac);
    REQUIRE(whole == 0);
    REQUIRE(frac  == -500);
}

///////////////////////////////////////////////////////////////////////////////
// apply_bc_discipline: rate-corrected (offset + skew) clock mapping
///////////////////////////////////////////////////////////////////////////////

/// With zero skew the result is exactly apply_bc_offset() (phase only); the anchor
/// is irrelevant in that case.
TEST_CASE("apply_bc_discipline with zero skew equals apply_bc_offset", "[sync][discipline]") {
    REQUIRE(apply_bc_discipline(1000u, 1000u, 5, 0, 0)     == 1005u);
    REQUIRE(apply_bc_discipline(1000u, 0u,    0, 500, 0)   == 1001u);  // +0.5 ms rounds up
    REQUIRE(apply_bc_discipline(1000u, 1000u, -2, -500, 0) == 997u);   // -2.5 ms
}

/// The skew term grows the correction linearly with the time since the anchor:
/// skew_ppb * (local - anchor) / 1e9, rounded to the nearest millisecond.
TEST_CASE("apply_bc_discipline adds the skew correction over elapsed time", "[sync][discipline]") {
    // 100000 ppb == 100 ppm == 0.1 ms per 1000 ms.
    REQUIRE(apply_bc_discipline(10000u, 0u, 0, 0, 100000) == 10001u);  // 0.1ms*10 = +1 ms
    REQUIRE(apply_bc_discipline(50000u, 0u, 0, 0, 100000) == 50005u);  // +5 ms
    REQUIRE(apply_bc_discipline(0u,     0u, 7, 0, 100000) == 7u);      // zero elapsed at anchor
    REQUIRE(apply_bc_discipline(50000u, 0u, 0, 0, -100000) == 49995u); // negative skew subtracts
    REQUIRE(apply_bc_discipline(50000u, 0u, 3, 0, 100000) == 50008u);  // +3 phase, +5 skew
}

/// The skew term rounds to the nearest millisecond and uses wrap-safe elapsed time
/// (a future anchor yields a negative correction).
TEST_CASE("apply_bc_discipline rounds the skew term and is wrap-safe", "[sync][discipline][wrap]") {
    REQUIRE(apply_bc_discipline(4000u, 0u, 0, 0, 100000) == 4000u);  // 0.4 ms rounds to 0
    REQUIRE(apply_bc_discipline(6000u, 0u, 0, 0, 100000) == 6001u);  // 0.6 ms rounds to +1
    // local just past zero, anchor ahead: elapsed = -50000 ms -> -5 ms, wraps below 0.
    REQUIRE(apply_bc_discipline(3u, 50003u, 0, 0, 100000) == (3u - 5u));
}


///////////////////////////////////////////////////////////////////////////////
// OffsetSmoother: the injected estimator the system handler feeds on SYNC_FINAL
///////////////////////////////////////////////////////////////////////////////

/// The first measurement seeds the estimate exactly (offset adopted, skew zero) so
/// the clock starts aligned, and the anchor is the sample's local tick (T2).
TEST_CASE("OffsetSmoother seeds exactly on the first sync with zero skew", "[sync][smooth]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    REQUIRE_FALSE(sm.seeded());
    sm.update(1000u, 3, 250, d);
    REQUIRE(sm.seeded());
    REQUIRE(d.anchor_ms    == 1000u);
    REQUIRE(d.offset_whole == 3);
    REQUIRE(d.offset_frac  == 250);
    REQUIRE(d.skew_ppb     == 0);
}

/// A steady offset over a full window fits a flat line: the offset is recovered and
/// the skew is exactly zero.
TEST_CASE("OffsetSmoother on a steady offset reports zero skew", "[sync][smooth]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    // Local tick advances 2000 ms per sync; the measured offset is a constant -12 ms.
    for (uint32_t k = 0u; k < 20u; ++k) {
        sm.update(1000u + k * 2000u, -12, 0, d);
    }
    REQUIRE(d.offset_whole == -12);
    REQUIRE(d.offset_frac  == 0);
    REQUIRE(d.skew_ppb     == 0);
}

/// The headline feature: a linearly ramping offset (a real clock skew) is recovered
/// as a skew in parts-per-billion, and the disciplined offset is the line's value at
/// the anchor -- not a lagging average as a plain EMA would give.
TEST_CASE("OffsetSmoother recovers a known skew from a ramping offset", "[sync][smooth]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    const double skew = 0.00005;     // ms offset per ms local == 50 ppm == 50000 ppb
    uint32_t anchor = 0u;
    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        const uint32_t t   = 100000u + k * 2000u;
        const double   off = 100.0 + skew * static_cast<double>(t);
        int32_t w = 0, f = 0;
        offset_split_signed(off, w, f);
        anchor = t;
        sm.update(t, w, f, d);
    }

    // Recovered skew within a few hundred ppb of the truth (quantization-limited).
    REQUIRE(d.skew_ppb == Catch::Approx(50000).margin(500));
    REQUIRE(d.anchor_ms == anchor);
    // The disciplined offset matches the injected line evaluated at the anchor.
    const double predicted = 100.0 + skew * static_cast<double>(anchor);
    REQUIRE(sync_decode_signed(d.offset_whole, d.offset_frac)
            == Catch::Approx(predicted).margin(0.05));
}

/// The raw, instantaneous measurement stays accessible even though the disciplined
/// value is what gets stored, and reset() returns the estimator to unseeded so the
/// next update() re-seeds exactly.
TEST_CASE("OffsetSmoother exposes the last raw measurement and resets", "[sync][smooth]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    sm.update(2000u, 2, 0, d);
    sm.update(4000u, 8, 500, d);
    int32_t raw_whole = 0;
    int32_t raw_frac  = 0;
    sm.raw(raw_whole, raw_frac);
    REQUIRE(raw_whole == 8);             // raw is the latest measurement, not the fit
    REQUIRE(raw_frac  == 500);

    sm.reset();
    REQUIRE_FALSE(sm.seeded());
    sm.update(6000u, -4, -250, d);       // re-seeds exactly after reset (zero skew)
    REQUIRE(d.offset_whole == -4);
    REQUIRE(d.offset_frac  == -250);
    REQUIRE(d.skew_ppb     == 0);
}

/// Guards the back-to-back-sync regression: even with at least kMinForSkew samples, a
/// window whose local-tick abscissae are bunched within less than kMinSpanForSkewMs makes
/// the least-squares slope ill-conditioned, so the estimator must report zero skew rather
/// than a wild value (which the SysTick governor would then chase). Once the samples span
/// past the threshold the skew is resolved normally.
TEST_CASE("OffsetSmoother suppresses skew from a too-short window span", "[sync][smooth]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    // Six samples only 100 ms apart (span 500 ms << kMinSpanForSkewMs) on a steep ramp a
    // naive fit would read as an enormous skew. Count exceeds kMinForSkew, so the span gate
    // -- not the count gate -- is what holds the skew at zero.
    for (uint32_t k = 0u; k < 6u; ++k) {
        sm.update(10000u + k * 100u, static_cast<int32_t>(k), 0, d);
    }
    REQUIRE(d.skew_ppb == 0);

    // The same ramp sampled 2000 ms apart spans well past the threshold and resolves to a
    // non-zero skew (offset rising one ms per 2000 ms of local time).
    sm.reset();
    for (uint32_t k = 0u; k < 8u; ++k) {
        sm.update(100000u + k * 2000u, static_cast<int32_t>(k), 0, d);
    }
    REQUIRE(d.skew_ppb != 0);
}

/// SNR gate (kSkewSignalMs): a realistic ~30 ppm crystal skew moves the offset only a few
/// tenths of a millisecond over a short baseline -- below the +/-0.5 ms timestamp
/// quantization -- so the least-squares slope, while it can be fit, is not yet sign-trustworthy
/// and must NOT be handed to the governor. The estimator reports ZERO skew at a ~10 s span
/// (predicted ramp ~0.3 ms << 1.5 ms) and only releases a correct-sign skew once the baseline
/// is long enough for the ramp to clear the gate. This is the adaptive part of the fix: small
/// skews wait for a longer window, large ones lock sooner.
TEST_CASE("OffsetSmoother gates a small skew until the ramp clears quantization",
          "[sync][smooth][snr]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    const double skew = 0.00003;   // 30 ppm == 30000 ppb; BC gains on RT (RT runs slow)

    // Short baseline: 6 samples 2000 ms apart -> ~10 s span. True ramp ~0.3 ms, below the
    // 1.5 ms signal floor, so the slope's sign is not yet trusted -> reported skew is zero.
    for (uint32_t k = 0u; k < 6u; ++k) {
        const uint32_t t   = 100000u + k * 2000u;
        const double   off = 5.0 + skew * static_cast<double>(t);
        int32_t w = 0, f = 0;
        offset_split_signed(off, w, f);
        sm.update(t, w, f, d);
    }
    REQUIRE(d.skew_ppb == 0);                                  // gated: ramp below the floor

    // Long baseline: a full window (~62 s span). Now the ramp is ~1.9 ms, clears the gate,
    // and the recovered skew has the correct sign and magnitude.
    sm.reset();
    uint32_t anchor = 0u;
    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        const uint32_t t   = 100000u + k * 2000u;
        const double   off = 5.0 + skew * static_cast<double>(t);
        int32_t w = 0, f = 0;
        offset_split_signed(off, w, f);
        anchor = t;
        sm.update(t, w, f, d);
    }
    REQUIRE(d.anchor_ms == anchor);
    REQUIRE(d.skew_ppb > 0);                                   // correct sign (RT slow)
    REQUIRE(d.skew_ppb == Catch::Approx(30000).margin(1500));  // correct magnitude
}

/// The core of the reported "adjusts the wrong direction" bug: with NO real skew, a single
/// +/-1 ms (one-LSB) quantization artifact across a multi-second window makes a naive fit read
/// ~18000 ppb -- three times the deadband, and of an arbitrary sign. The signal gate must hold
/// the reported skew at zero so the governor is never steered by quantization noise.
TEST_CASE("OffsetSmoother reports zero skew from pure quantization noise",
          "[sync][smooth][snr]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    // Constant true offset of 0 ms with two samples quantized to +1 LSB. Span ~12 s; a naive
    // slope is ~tens of ppm, but the predicted end-to-end ramp is well under 1 LSB (< 1.5 ms),
    // so the gate must report zero.
    const int32_t noisy[7] = {0, 1, 0, 0, 1, 0, 0};
    for (uint32_t k = 0u; k < 7u; ++k) {
        sm.update(100000u + k * 2000u, noisy[k], 0, d);
    }
    REQUIRE(d.skew_ppb == 0);
}


///////////////////////////////////////////////////////////////////////////////
// next_tick_reload: the bounded SysTick-reload governor
///////////////////////////////////////////////////////////////////////////////

/// Sign is the crux of the feature: skew_ppb > 0 means the BC gains on the RT (the RT
/// runs slow), so the RT must tick FASTER and the reload must DECREASE; a negative skew
/// lengthens it. The smoothed governor moves a fraction (kReloadGainNum/kReloadGainDen,
/// here 3/4) of the way to the ideal, so a +/-1000 ppm skew on a 168000 reload (ideal
/// 167832 / 168168, a 168-tick gap) moves a 3/4 step of 126 ticks in the correct direction.
TEST_CASE("next_tick_reload: positive skew shortens the reload, negative lengthens it", "[sync][reload][sign]") {
    REQUIRE(next_tick_reload(168000u, 168000u,  1000000) == 167874u);  // RT slow -> faster, 3/4-step -126
    REQUIRE(next_tick_reload(168000u, 168000u, -1000000) == 168126u);  // RT fast -> slower, 3/4-step +126
}

/// Inside the deadband (about one reload tick of resolution, kReloadDeadbandPpb) the
/// register is untouched -- this stops dithering at the resolution floor and makes cold
/// start (skew_ppb == 0) a no-op. At the deadband edge it acts by exactly one tick.
TEST_CASE("next_tick_reload: deadband holds within one tick of resolution", "[sync][reload][deadband]") {
    REQUIRE(next_tick_reload(168000u, 168000u, 0)     == 168000u);   // cold start
    REQUIRE(next_tick_reload(168000u, 168000u,  5999) == 168000u);
    REQUIRE(next_tick_reload(168000u, 168000u, -5999) == 168000u);
    REQUIRE(next_tick_reload(168000u, 168000u,  6000) == 167999u);   // edge -> one tick
    REQUIRE(next_tick_reload(168000u, 168000u, -6000) == 168001u);
}

/// A skew far above the deadband no longer moves a single tick: the governor takes a
/// smoothed *partial* step -- about kReloadGainNum/kReloadGainDen of the way to the reload
/// that would fully cancel the skew -- so one big correction is spread over several adjust
/// cycles instead of applied at once. (Convergence of the closed loop, where the measured
/// skew shrinks after each step, is exercised separately below.)
TEST_CASE("next_tick_reload: takes a smoothed partial step toward the ideal", "[sync][reload][gain]") {
    const uint32_t nominal = 168000u;
    const int32_t  skew    = 1000000;   // +1000 ppm, far above the deadband
    const uint32_t ideal   = 167832u;   // 168000 - round(168000 * 1e6 / 1e9) = 168000 - 168
    const uint32_t once    = next_tick_reload(nominal, nominal, skew);
    REQUIRE(once == 167874u);                       // 168000 - 126: three-quarters of the 168-tick gap
    REQUIRE(once < nominal);                        // correct direction (RT slow -> shorten)
    REQUIRE(once > ideal);                          // partial: did not reach the full correction
    REQUIRE((nominal - once) < (nominal - ideal));  // moved less than the whole gap
}

/// The reload may never leave +/-10% of nominal, however long a large skew persists.
TEST_CASE("next_tick_reload: clamps to +/-10% of nominal", "[sync][reload][clamp]") {
    const uint32_t nominal = 168000u;
    const uint32_t lo = nominal - nominal / 10u;   // 151200
    const uint32_t hi = nominal + nominal / 10u;   // 184800

    uint32_t r = nominal;
    for (int i = 0; i < 100000; ++i) { r = next_tick_reload(r, nominal, 1000000); }
    REQUIRE(r == lo);
    REQUIRE(next_tick_reload(lo, nominal, 1000000) == lo);   // cannot go below the band

    r = nominal;
    for (int i = 0; i < 100000; ++i) { r = next_tick_reload(r, nominal, -1000000); }
    REQUIRE(r == hi);
    REQUIRE(next_tick_reload(hi, nominal, -1000000) == hi);  // nor above it
}

/// Closed-loop behavior: model the residual skew shrinking as the reload already
/// compensates part of the oscillator's true error. With the RT running slow by 30 ppm
/// the loop drives the reload to the value that cancels it (~167995) and then stops,
/// because the remaining residual falls inside the deadband. This is the convergence the
/// hardware discipline relies on -- it settles, it does not hunt.
TEST_CASE("next_tick_reload: closed loop converges and settles in the deadband", "[sync][reload][converge]") {
    const int32_t  injected_ppb = 30000;   // RT slow by 30 ppm if uncorrected
    const uint32_t nominal      = 168000u;
    uint32_t reload = nominal;
    for (int i = 0; i < 200; ++i) {
        const int32_t compensated = static_cast<int32_t>(
            (static_cast<int64_t>(nominal) - static_cast<int64_t>(reload)) * 1000000000 / nominal);
        const int32_t residual = injected_ppb - compensated;
        reload = next_tick_reload(reload, nominal, residual);
    }
    REQUIRE(reload >= 167994u);
    REQUIRE(reload <= 167996u);
    // Settled: the residual at the final reload is inside the deadband, so it holds.
    const int32_t compensated = static_cast<int32_t>(
        (static_cast<int64_t>(nominal) - static_cast<int64_t>(reload)) * 1000000000 / nominal);
    REQUIRE(next_tick_reload(reload, nominal, injected_ppb - compensated) == reload);
}


///////////////////////////////////////////////////////////////////////////////
// OffsetSmoother::rebase_rate: pivoting the window onto a new tick rate
///////////////////////////////////////////////////////////////////////////////

/// A reload step changes the true offset slope at the step instant. Without a
/// rebase the window blends the old and new slopes for up to a full window (the
/// Path A / Path B rate double-count); after rebase_rate() the very next fit must
/// report the NEW residual slope. Old rate: +80 ppm. Step removes 50 ppm
/// (rate_delta = -50000 ppb), so the post-step truth is +30 ppm -- chosen so the
/// predicted ramp over the ~64 s window (1.9 ms) clears the 1.5 ms SNR gate.
TEST_CASE("OffsetSmoother rebase_rate makes the next fit report the residual slope",
          "[sync][smooth][rebase]") {
    OffsetSmoother sm;
    OffsetSmoother::Discipline d;

    const double old_skew = 0.00008;   // +80 ppm before the reload step
    const double new_skew = 0.00003;   // +30 ppm after (step removed 50 ppm)

    // Fill a whole window on the old-rate line.
    uint32_t t = 0u;
    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        t = 100000u + k * 2000u;
        const double off = 100.0 + old_skew * static_cast<double>(t);
        int32_t w = 0, f = 0;
        offset_split_signed(off, w, f);
        sm.update(t, w, f, d);
    }
    REQUIRE(d.skew_ppb == Catch::Approx(80000).margin(1000));   // old slope resolved

    // The reload step happens 1 s after the last sample; the true offset line is
    // continuous at the pivot and continues at the new slope.
    const uint32_t pivot = t + 1000u;
    sm.rebase_rate(pivot, -50000);
    const double off_at_pivot = 100.0 + old_skew * static_cast<double>(pivot);

    // One single post-step sample is enough: all rebased samples already lie on
    // the new-rate line, so the fit reports the residual immediately -- no blend.
    const uint32_t t2  = pivot + 2000u;
    const double  off2 = off_at_pivot + new_skew * static_cast<double>(t2 - pivot);
    int32_t w = 0, f = 0;
    offset_split_signed(off2, w, f);
    sm.update(t2, w, f, d);
    REQUIRE(d.skew_ppb == Catch::Approx(30000).margin(1000));
}

/// The rebase pivots AT pivot_tick: the offset there is rate-independent, so the
/// fitted line must still pass through the pre-step value at the pivot, for a
/// rate delta of either sign. Built on a flat line so the post-rebase intercept
/// is easy to predict: a +100 ppm delta tilts the history down behind the pivot
/// and the next fit reports +100 ppm while still anchoring 10.0 at the pivot.
TEST_CASE("OffsetSmoother rebase_rate pivots exactly at the step instant",
          "[sync][smooth][rebase]") {
    for (const int32_t delta_ppb : {100000, -100000}) {
        OffsetSmoother sm;
        OffsetSmoother::Discipline d;

        uint32_t t = 0u;
        for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
            t = 100000u + k * 2000u;
            sm.update(t, 10, 0, d);    // flat: constant +10 ms offset, zero skew
        }
        REQUIRE(d.skew_ppb == 0);

        const uint32_t pivot = t;      // step lands exactly on the newest sample
        sm.rebase_rate(pivot, delta_ppb);

        // Continue on the new-rate truth: still 10.0 at the pivot, sloped after.
        const double  slope = static_cast<double>(delta_ppb) / 1.0e9;
        const uint32_t t2   = pivot + 2000u;
        const double  off2  = 10.0 + slope * static_cast<double>(t2 - pivot);
        int32_t w = 0, f = 0;
        offset_split_signed(off2, w, f);
        sm.update(t2, w, f, d);

        REQUIRE(d.skew_ppb == Catch::Approx(delta_ppb).margin(1000));
        // Intercept at the new anchor sits on the line through (pivot, 10.0).
        REQUIRE(sync_decode_signed(d.offset_whole, d.offset_frac)
                == Catch::Approx(off2).margin(0.05));
    }
}


///////////////////////////////////////////////////////////////////////////////
// PhaseAnchor: the long-baseline skew the SYNC_ADJUST governor consumes
///////////////////////////////////////////////////////////////////////////////

/// Before any measurement there is nothing to estimate; a single point seeds the
/// snapshot but spans no baseline, so the caller must still fall back to the
/// smoother's windowed skew (return false).
TEST_CASE("PhaseAnchor has no estimate before a usable baseline exists",
          "[sync][anchor]") {
    PhaseAnchor pa;
    int32_t skew = 123;

    REQUIRE_FALSE(pa.has_snapshot());
    REQUIRE_FALSE(pa.adjust_skew_ppb(skew));

    pa.note_measurement(100000u, 0, 0);
    REQUIRE(pa.has_snapshot());
    REQUIRE_FALSE(pa.adjust_skew_ppb(skew));        // zero-span baseline

    // A second point inside kMinBaselineMs is still too short to condition.
    pa.note_measurement(100000u + PhaseAnchor::kMinBaselineMs - 1u, 0, 100);
    REQUIRE_FALSE(pa.adjust_skew_ppb(skew));
}

/// One full adjust baseline: 1.665 ms of offset motion over 91 s is the
/// stall-run residual (-18.3 ppm mirrored positive). skew = round(1665e3 micro
/// * 1e3 / 91000 ms) with half-away-from-zero rounding -> exactly 18297 ppb,
/// and the negative mirror must round symmetrically.
TEST_CASE("PhaseAnchor measures the residual skew over one adjust baseline",
          "[sync][anchor]") {
    int32_t skew = 0;

    PhaseAnchor pos;
    pos.note_measurement(100000u, 0, 0);
    pos.note_measurement(191000u, 1, 665);          // +1.665 ms after 91 s
    REQUIRE(pos.adjust_skew_ppb(skew));
    REQUIRE(skew == 18297);

    PhaseAnchor neg;
    neg.note_measurement(100000u, 0, 0);
    neg.note_measurement(191000u, -1, -665);        // -1.665 ms after 91 s
    REQUIRE(neg.adjust_skew_ppb(skew));
    REQUIRE(skew == -18297);
}

/// At unit level: a residual whose per-phase offset motion is below kSignalMicro
/// must report zero WITHOUT dropping the snapshot, so the
/// baseline keeps growing and the same skew becomes measurable one phase later.
/// 18 ppm moves 0.36 ms in 20 s (below the 0.5 ms floor) but 0.72 ms in 40 s.
TEST_CASE("PhaseAnchor keeps the snapshot below threshold so a small skew accumulates",
          "[sync][anchor]") {
    PhaseAnchor pa;
    int32_t skew = 123;

    pa.note_measurement(100000u, 0, 0);
    pa.note_measurement(120000u, 0, 360);           // 18 ppm * 20 s = 0.36 ms
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 0);                             // below the noise floor...
    REQUIRE(pa.has_snapshot());                     // ...but the baseline survives

    pa.note_measurement(140000u, 0, 720);           // same 18 ppm, now 40 s deep
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 18000);                         // round(720e6 / 40000): measurable
    REQUIRE_FALSE(pa.has_snapshot());               // accepted: baseline re-seeds
}

/// After an accepted measurement the baseline restarts at the next point, so a
/// follow-up estimate spans only post-reseed motion -- never the pre-reseed origin.
TEST_CASE("PhaseAnchor reseeds the baseline after an accepted measurement",
          "[sync][anchor]") {
    PhaseAnchor pa;
    int32_t skew = 0;

    pa.note_measurement(100000u, 0, 0);
    pa.note_measurement(110000u, 1, 0);             // +1 ms in 10 s -> 100 ppm
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 100000);
    REQUIRE_FALSE(pa.has_snapshot());
    REQUIRE_FALSE(pa.adjust_skew_ppb(skew));        // no baseline until the next note

    pa.note_measurement(150000u, 1, 0);             // new origin: (150000, +1.0 ms)
    REQUIRE(pa.has_snapshot());
    pa.note_measurement(160000u, 1, 600);           // +0.6 ms over the NEW 10 s only
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 60000);                         // 0.6 ms / 10 s, not 1.6 ms / 60 s
}

/// invalidate_snapshot() is the reload-step hook: the old-rate baseline is
/// dropped, the next note re-seeds, and no estimate exists in between.
TEST_CASE("PhaseAnchor invalidate_snapshot drops the baseline",
          "[sync][anchor]") {
    PhaseAnchor pa;
    int32_t skew = 0;

    pa.note_measurement(100000u, 0, 0);
    pa.note_measurement(120000u, 2, 0);
    REQUIRE(pa.has_snapshot());

    pa.invalidate_snapshot();
    REQUIRE_FALSE(pa.has_snapshot());
    REQUIRE_FALSE(pa.adjust_skew_ppb(skew));

    pa.note_measurement(130000u, 2, 0);             // re-seeds here
    pa.note_measurement(140000u, 2, 800);           // +0.8 ms over 10 s
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 80000);                         // measured from the re-seed only
}

/// A baseline that reaches kMaxBaselineMs with sub-threshold motion is locked
/// (< 0.14 ppm): report zero and drop the snapshot to bound the int32 span.
TEST_CASE("PhaseAnchor drops a locked baseline at the max-baseline bound",
          "[sync][anchor]") {
    PhaseAnchor pa;
    int32_t skew = 123;

    pa.note_measurement(100000u, 0, 0);
    pa.note_measurement(100000u + static_cast<uint32_t>(PhaseAnchor::kMaxBaselineMs),
                        0, 400);                    // 0.4 ms in an hour: locked
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 0);
    REQUIRE_FALSE(pa.has_snapshot());               // dropped, re-seeds next note
}

/// The anchor delta uses the same wrap-safe int32 subtraction as the smoother, so
/// a baseline straddling the uint32 tick rollover still yields the true span.
/// 0xFFFFF000 + 95000 wraps to 90904; +2 ms over 95 s -> round(2e9/95000) = 21053.
TEST_CASE("PhaseAnchor tolerates uint32 anchor wraparound",
          "[sync][anchor][wrap]") {
    PhaseAnchor pa;
    int32_t skew = 0;

    pa.note_measurement(0xFFFFF000u, 0, 0);
    pa.note_measurement(0xFFFFF000u + 95000u, 2, 0);   // wraps past zero
    REQUIRE(pa.adjust_skew_ppb(skew));
    REQUIRE(skew == 21053);
}
