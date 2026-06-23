// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/communication/sync.h"

#include <cmath>

#include "rt/common/helper_functions.h"


namespace {

/// @brief Wrap-safe signed difference of two millisecond counters.
/// @details Unsigned subtraction modulo 2^32 followed by a reinterpreting cast
///          recovers the true small delta even across a counter rollover.
int32_t wrap_delta(uint32_t a, uint32_t b)
{
    return static_cast<int32_t>(a - b);
}


/// @brief Split a non-negative value into whole + thousandths-fractional.
/// @details Rounds the fractional part to the nearest thousandth and carries a
///          rounded-up 1000 into the whole part.
void split_unsigned(double value, uint32_t& whole_out, uint32_t& frac_out)
{
    if (value < 0.0) {
        value = 0.0;
    }
    double whole       = std::floor(value);
    long   frac        = std::lround((value - whole) * 1000.0);
    if (frac >= 1000) {           // Rounding carried into the next whole unit.
        frac -= 1000;
        whole += 1.0;
    }
    whole_out = static_cast<uint32_t>(whole);
    frac_out  = static_cast<uint32_t>(frac);
}


} // namespace


void offset_split_signed(double value, int32_t& whole_out, int32_t& frac_out)
{
    double int_part  = 0.0;
    double frac_part = std::modf(value, &int_part);   // Truncates toward zero.
    int32_t whole    = static_cast<int32_t>(int_part);
    long    frac     = std::lround(frac_part * 1000.0);
    if (frac >= 1000) {           // Positive rounding carry.
        frac -= 1000;
        whole += 1;
    } else if (frac <= -1000) {   // Negative rounding carry.
        frac += 1000;
        whole -= 1;
    }
    whole_out = whole;
    frac_out  = static_cast<int32_t>(frac);
}


SyncFixed compute_sync_fixed(uint32_t t1, uint32_t t2, uint32_t t3, uint32_t t4)
{
    // NTP's four-timestamp form. The two server-side instants are kept distinct
    // (T2 = receive, T3 = response send) so the RT's own handling time is
    // subtracted out instead of assumed zero, and the wrap-safe deltas keep a
    // millisecond counter rollover between captures correct.
    const double d_t4_t1 = static_cast<double>(wrap_delta(t4, t1));   // T4 - T1
    const double d_t3_t2 = static_cast<double>(wrap_delta(t3, t2));   // T3 - T2
    const double d_t1_t2 = static_cast<double>(wrap_delta(t1, t2));   // T1 - T2
    const double d_t4_t3 = static_cast<double>(wrap_delta(t4, t3));   // T4 - T3

    // Mean (one-way) path delay: half the round trip with the server hold time
    // (T3 - T2) removed -- delay = ((T4 - T1) - (T3 - T2)) / 2. The reported
    // field is the one-way delay (RTT/2), as before. Clamped at zero so a tiny
    // negative from jitter never encodes as a huge unsigned whole part
    // (split_unsigned also clamps, but be explicit).
    double path_delay = (d_t4_t1 - d_t3_t2) / 2.0;
    if (path_delay < 0.0) {
        path_delay = 0.0;
    }

    // Offset is BC - RT (the correction added to a local RT tick to recover BC
    // time): the mean of the forward (T1 - T2) and reverse (T4 - T3) BC-minus-RT
    // differences. This is the negation of NTP's server-minus-client theta, to
    // match the sign the clock store expects. Positive when the BC clock leads.
    const double offset = (d_t1_t2 + d_t4_t3) / 2.0;

    SyncFixed out;
    split_unsigned(path_delay, out.path_delay_whole, out.path_delay_frac);
    offset_split_signed(offset, out.offset_whole, out.offset_frac);
    return out;
}


double sync_decode_unsigned(uint32_t whole, uint32_t frac)
{
    return static_cast<double>(whole) + static_cast<double>(frac) / 1000.0;
}


double sync_decode_signed(int32_t whole, int32_t frac)
{
    return static_cast<double>(whole) + static_cast<double>(frac) / 1000.0;
}


uint32_t apply_bc_offset(uint32_t local_ms, int32_t offset_whole, int32_t offset_frac)
{
    // Whole-millisecond shift. Reinterpreting a (possibly negative) int32 as
    // uint32 and adding modulo 2^32 subtracts for negative offsets and tolerates
    // counter rollover, with no signed-overflow hazard.
    uint32_t shifted = local_ms + static_cast<uint32_t>(offset_whole);

    // Round to the nearest whole millisecond from the thousandths-fractional
    // part; the +/-1 uses the same modulo-2^32 arithmetic.
    if (offset_frac >= 500) {
        shifted += 1u;
    } else if (offset_frac <= -500) {
        shifted -= 1u;
    }
    return shifted;
}


uint32_t apply_bc_discipline(uint32_t local_ms, uint32_t anchor_ms,
                             int32_t offset_whole, int32_t offset_frac,
                             int32_t skew_ppb)
{
    // Phase term: the same nearest-millisecond offset shift as apply_bc_offset().
    uint32_t shifted = apply_bc_offset(local_ms, offset_whole, offset_frac);

    // Frequency term: how far the offset has drifted since the anchor, at the
    // estimated skew. `elapsed` is wrap-safe signed (a negative value means the
    // clock has not yet reached the anchor); the product is taken in int64 so it
    // cannot overflow for any int32 inputs.
    const int32_t elapsed = static_cast<int32_t>(local_ms - anchor_ms);
    const int64_t num     = static_cast<int64_t>(skew_ppb) * static_cast<int64_t>(elapsed);
    // Round to the nearest whole millisecond (half a billion ppb-ms == 0.5 ms).
    const int64_t skew_ms = (num >= 0) ? (num + 500000000) / 1000000000
                                       : (num - 500000000) / 1000000000;

    // Fold in with the same modulo-2^32 arithmetic; converting a negative int64 to
    // uint32 keeps the correct two's-complement low word, so the add subtracts.
    shifted += static_cast<uint32_t>(skew_ms);
    return shifted;
}


int32_t offset_pack_micro(int32_t whole, int32_t frac)
{
    return whole * 1000 + frac;
}


void offset_unpack_micro(int32_t micro, int32_t& whole_out, int32_t& frac_out)
{
    // C++ integer division and remainder both truncate toward zero, so the whole
    // and fractional parts share the sign of `micro` (e.g. -1234 -> {-1, -234}),
    // matching the SyncFixed sign invariant.
    whole_out = micro / 1000;
    frac_out  = micro % 1000;
}


uint32_t next_tick_reload(uint32_t current_reload, uint32_t nominal_reload, int32_t skew_ppb)
{
    // Deadband at the timer's resolution floor: a skew that one reload tick cannot
    // represent is left alone, so the register stops dithering once it is as close as
    // the hardware allows. This also makes cold start (skew_ppb == 0) a no-op.
    if (skew_ppb > -kReloadDeadbandPpb && skew_ppb < kReloadDeadbandPpb) {
        return current_reload;
    }

    // Ideal reload that fully cancels the averaged skew. skew_ppb > 0 means the BC gains
    // on the RT (RT slow) -> shorten the tick -> DECREASE the reload (and vice versa):
    //   ideal = round(current * (1 - skew_ppb / 1e9)) = current - round(current * skew_ppb / 1e9)
    // Done in int64 (cannot overflow for any uint32 reload * int32 ppb), rounded to the
    // nearest tick (half away from zero), matching apply_bc_discipline()'s style.
    const int64_t cur   = static_cast<int64_t>(current_reload);
    const int64_t num   = cur * static_cast<int64_t>(skew_ppb);
    const int64_t delta = (num >= 0) ? (num + 500000000) / 1000000000
                                     : (num - 500000000) / 1000000000;
    const int64_t ideal = cur - delta;

    // Smoothed partial step: move only kReloadGainNum/kReloadGainDen of the way to the
    // ideal, so the reload approaches the target over several telemetry/adjust cycles
    // rather than jumping. The next telemetry phase measures the remaining error and the
    // following adjust shrinks it again -- geometric convergence. Rounded half away from
    // zero so a one-tick gap (just past the deadband) still moves one tick.
    const int64_t gap  = ideal - cur;
    const int64_t gnum = static_cast<int64_t>(kReloadGainNum);
    const int64_t gden = static_cast<int64_t>(kReloadGainDen);
    int64_t step = (gap >= 0) ? (gap * gnum + gden / 2) / gden
                              : (gap * gnum - gden / 2) / gden;
    // Past the deadband but rounded to zero (only reachable with a gain below 1/2): still
    // take a single tick in the correct direction so the loop cannot stall short of lock.
    if (step == 0) { step = (gap >= 0) ? 1 : -1; }
    int64_t next = cur + step;

    // Hard clamp to +/- kReloadBandPercent of nominal: the inviolable safety bound.
    const int64_t nom  = static_cast<int64_t>(nominal_reload);
    const int64_t band = nom * static_cast<int64_t>(kReloadBandPercent) / 100;
    if (next < nom - band) { next = nom - band; }
    if (next > nom + band) { next = nom + band; }

    return static_cast<uint32_t>(next);
}


SyncState::SyncState()
    : m_units(static_cast<uint8_t>(TimeUnits::Millisecond))
    , m_t1(0)
    , m_t2(0)
    , m_t3(0)
{
}


uint32_t SyncState::on_probe(uint8_t units, uint32_t t1, uint32_t rx_now,
                             uint32_t tx_now, uint8_t* resp)
{
    m_units = units;
    m_t1    = t1;
    m_t2    = rx_now;
    m_t3    = tx_now;
    emplace_u32_le(resp,     m_t2);
    emplace_u32_le(resp + 4, m_t3);
    return kSyncProbeRespBytes;
}


uint32_t SyncState::on_final(uint32_t t4, uint8_t* resp, SyncFixed& result_out)
{
    const SyncFixed r = compute_sync_fixed(m_t1, m_t2, m_t3, t4);
    result_out = r;
    emplace_u32_le(resp,      r.path_delay_whole);
    emplace_u32_le(resp + 4,  r.path_delay_frac);
    emplace_u32_le(resp + 8,  static_cast<uint32_t>(r.offset_whole));
    emplace_u32_le(resp + 12, static_cast<uint32_t>(r.offset_frac));
    return kSyncFinalCoreBytes;
}
