// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_sync_e2e.cpp
/// @brief End-to-end time-synchronization test over the real scheduler.
/// @details
/// Drives a real BusController <-> RemoteTerminal pair (through paired
/// FakeTransports) with a real SystemHandler on the system sub-address, running
/// the two-command Cristian exchange the BC client performs in sync_once(). The
/// BC and RT run on SEPARATE clocks so a known offset Delta = (RT - BC) can be
/// injected, and an optional per-tick advance models propagation. The test then
/// checks that the RT's now_bc_ms() lands on the BC clock.
///
/// The invariant under test: the offset is (BC - RT) and is *added* to the RT
/// tick, so injecting an RT that leads the BC by 12 ms must leave now_bc_ms() on
/// the BC clock -- not 24 ms (2*Delta) away from it, the error a wrong-signed
/// offset would produce.

#include "thirdparty/catch_amalgamated.hpp"

#include "../communication/scheduler.h"
#include "../communication/sync.h"
#include "../communication/offset_smoother.h"
#include "../communication/i_clock.h"
#include "../communication/i_frame_transport.h"
#include "../communication/fault_counter.h"
#include "../system/system_handler.h"
#include "../generated/command_ids.h"
#include "../common/helper_functions.h"

#include <cstring>
#include <deque>
#include <vector>


namespace {

/// @brief Settable IClock whose now_bc_ms() folds in the stored discipline exactly
///        like the on-target SysTickClock (shared apply_bc_discipline()).
class FakeClock : public IClock {
    public:
    uint32_t m_now_ms    = 0;
    int32_t  m_off_whole = 0;
    int32_t  m_off_frac  = 0;
    uint32_t m_anchor_ms = 0;
    int32_t  m_skew_ppb  = 0;
    bool     m_has_sync  = false;
    uint32_t m_tick_reload = 168000u;   ///< Nominal STM32F407 reload; disciplined on SYNC_FINAL.

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
    uint32_t tick_reload() const override { return m_tick_reload; }
    uint32_t nominal_tick_reload() const override { return 168000u; }
    void set_tick_reload(uint32_t ticks) override { m_tick_reload = ticks; }
    bool has_sync() const override { return m_has_sync; }
    void set_has_sync(const bool synchronized) override { m_has_sync = synchronized; }
};


/// @brief Frame transport backed by std::deque; send() also enqueues onto the
///        peer's rx so a BC and RT instance can be wired as a bidirectional bus.
class FakeTransport : public IFrameTransport {
    public:
    std::deque<std::vector<uint8_t>> rx_queue;
    FakeTransport* peer = nullptr;

    int32_t send(uint8_t const* bytes, uint32_t length) override
    {
        std::vector<uint8_t> v(bytes, bytes + length);
        if (peer != nullptr) {
            peer->rx_queue.push_back(v);
        }
        return static_cast<int32_t>(length);
    }
    int32_t receive(uint8_t* out, uint32_t max_length) override
    {
        if (rx_queue.empty()) return 0;
        auto& front = rx_queue.front();
        uint32_t n = static_cast<uint32_t>(front.size());
        if (n > max_length) n = max_length;
        std::memcpy(out, front.data(), n);
        rx_queue.pop_front();
        return static_cast<int32_t>(n);
    }
    void reset_rx() override {}
};


/// @brief Captured completion-callback arguments.
struct Completion {
    bool                 fired = false;
    Status               status = Status::Ok;
    std::vector<uint8_t> payload;
};

void capture_completion(Status status, const MessageHeader&,
                        uint8_t const* resp_payload, uint32_t resp_len, void* user)
{
    auto* c = static_cast<Completion*>(user);
    c->fired  = true;
    c->status = status;
    if (resp_payload != nullptr && resp_len > 0) {
        c->payload.assign(resp_payload, resp_payload + resp_len);
    } else {
        c->payload.clear();
    }
}

/// @brief Drive bc.tick()/rt.tick() until the completion fires, advancing both
///        clocks by @p prop_ms after each pair to model symmetric propagation.
/// @details Stops before the advance on the tick that completes, so the BC clock
///          read right after is the instant the response arrived.
void drive(BusController& bc, RemoteTerminal& rt, FakeClock& bcc, FakeClock& rtc,
           Completion& comp, uint32_t prop_ms, int max_iters = 100)
{
    for (int i = 0; i < max_iters && !comp.fired; ++i) {
        bc.tick();
        rt.tick();
        if (comp.fired) return;
        bcc.m_now_ms += prop_ms;
        rtc.m_now_ms += prop_ms;
    }
}

/// @brief Build a sync command Request on the system sub-address.
/// @details The SYNC_PROBE response is kSyncProbeRespBytes (8: T2 and T3) and the
///          SYNC_FINAL response is kSyncFinalRespBytes (16). Both fit the inline
///          slot, but the caller still passes an external response buffer (the
///          Request contract uses resp_ext only when resp_buf_max exceeds
///          kInlinePayloadBytes).
Request make_sync_req(uint8_t opcode, uint8_t units, uint32_t value,
                      uint8_t* resp_ext, uint32_t resp_cap)
{
    Request r{};
    r.sub_address   = 1;   // system sub-address
    r.priority      = MessagePriorities::Medium;
    r.is_read       = true;
    r.cmd_inline[0] = opcode;
    r.cmd_inline[1] = units;
    emplace_u32_le(r.cmd_inline + 2, value);
    r.cmd_len       = kSyncCmdBytes;
    r.cmd_size      = bytes_to_payload_size(kSyncCmdBytes);
    r.cmd_ext       = nullptr;
    r.resp_buf_max  = resp_cap;
    r.resp_ext      = (resp_cap > kInlinePayloadBytes) ? resp_ext : nullptr;
    return r;
}

/// @brief Outcome of one end-to-end exchange.
struct SyncOutcome {
    Status   status     = Status::Ok;
    double   path_delay = 0.0;
    double   offset     = 0.0;   ///< Decoded BC - RT offset.
    uint32_t t1 = 0, t2 = 0, t3 = 0, t4 = 0;
};

/// @brief Run one Cristian exchange from the BC side, just like the generated
///        RtClient::sync_once(). MIRRORS bc_codegen::render_sync_impl(); keep the
///        two in lockstep.
SyncOutcome sync_once_e2e(BusController& bc, RemoteTerminal& rt,
                          FakeClock& bcc, FakeClock& rtc, uint32_t prop_ms)
{
    SyncOutcome out;
    const uint8_t units = static_cast<uint8_t>(TimeUnits::Millisecond);
    Status s = Status::Ok;
    uint8_t resp_buf[kSyncFinalRespBytes] = {0};

    // Step 1 (SYNC_PROBE): T1 = BC clock at send; RT returns T2 and T3 and caches
    // T1; T4 = BC clock when the response arrives.
    out.t1 = bcc.now_ms();
    Request probe_req = make_sync_req(static_cast<uint8_t>(SystemCmd::SyncProbe), units, out.t1,
                                      resp_buf, sizeof(resp_buf));
    Completion probe;
    if (!bc.submit(probe_req, capture_completion, &probe, s)) { out.status = s; return out; }
    drive(bc, rt, bcc, rtc, probe, prop_ms);
    out.t4 = bcc.now_ms();
    if (!probe.fired || probe.status != Status::Ok || probe.payload.size() < kSyncProbeRespBytes) {
        out.status = probe.fired ? probe.status : Status::Timeout;
        return out;
    }
    out.t2 = extract_u32_le(probe.payload.data());
    out.t3 = extract_u32_le(probe.payload.data() + 4);

    // Step 2 (SYNC_FINAL): send T4; the RT computes path delay and the BC-minus-RT
    // offset from its cached T1, T2, T3 and this T4, runs the min-delay clock
    // filter, applies the selected offset, and returns the fixed-point result for
    // the BC's view.
    Request final_req = make_sync_req(static_cast<uint8_t>(SystemCmd::SyncFinal), units, out.t4,
                                      resp_buf, sizeof(resp_buf));
    Completion fin;
    if (!bc.submit(final_req, capture_completion, &fin, s)) { out.status = s; return out; }
    drive(bc, rt, bcc, rtc, fin, prop_ms);
    if (!fin.fired || fin.status != Status::Ok || fin.payload.size() < kSyncFinalRespBytes) {
        out.status = fin.fired ? fin.status : Status::Timeout;
        return out;
    }
    out.path_delay = sync_decode_unsigned(extract_u32_le(fin.payload.data()),
                                          extract_u32_le(fin.payload.data() + 4));
    out.offset = sync_decode_signed(
        static_cast<int32_t>(extract_u32_le(fin.payload.data() + 8)),
        static_cast<int32_t>(extract_u32_le(fin.payload.data() + 12)));
    out.status = Status::Ok;
    return out;
}


/// @brief Outcome of one SYNC_ADJUST command.
struct AdjustOutcome {
    Status   status       = Status::Ok;
    int32_t  avg_skew_ppb = 0;   ///< Averaged skew the RT used (ppb).
    uint32_t reload       = 0;   ///< SysTick reload the RT programmed after the adjust.
};

/// @brief Run one SYNC_ADJUST from the BC side, like RtClient::sync_adjust().
///        MIRRORS bc_codegen::render_sync_impl(); keep the two in lockstep.
/// @details A one-byte read command (mirrors get_message_fault_counter's shape): the RT
///          applies one smoothed, bounded reload step from the skew the telemetry phase
///          averaged and replies with [skew (i32), reload (u32)] = kSyncAdjustRespBytes.
AdjustOutcome sync_adjust_e2e(BusController& bc, RemoteTerminal& rt,
                              FakeClock& bcc, FakeClock& rtc)
{
    AdjustOutcome out;
    Request r{};
    r.sub_address   = 1;   // system sub-address
    r.priority      = MessagePriorities::Medium;
    r.is_read       = true;
    r.cmd_inline[0] = static_cast<uint8_t>(SystemCmd::SyncAdjust);
    // Pad the opcode-only command up to its wire bucket (the smallest is size_2, 2 bytes),
    // exactly as the generated client's stage_inline_cmd() does. cmd_inline is zero-filled.
    r.cmd_size      = bytes_to_payload_size(kSyncAdjustCmdBytes);
    r.cmd_len       = payload_size_to_bytes(r.cmd_size);
    r.cmd_ext       = nullptr;
    r.resp_buf_max  = 16u;     // 8 B reply fits the inline slot
    r.resp_ext      = nullptr;

    Status s = Status::Ok;
    Completion comp;
    if (!bc.submit(r, capture_completion, &comp, s)) { out.status = s; return out; }
    drive(bc, rt, bcc, rtc, comp, 0u);
    if (!comp.fired || comp.status != Status::Ok || comp.payload.size() < kSyncAdjustRespBytes) {
        out.status = comp.fired ? comp.status : Status::Timeout;
        return out;
    }
    out.avg_skew_ppb = static_cast<int32_t>(extract_u32_le(comp.payload.data()));
    out.reload       = extract_u32_le(comp.payload.data() + 4);
    out.status       = Status::Ok;
    return out;
}

/// @brief Wire a BC and RT (with a system handler) together over paired
///        transports. Bundled so each test case gets an independent rig.
struct SyncRig {
    FakeTransport tb, tr;
    FakeClock     bc_clock, rt_clock;
    BusController bc;
    FaultCounter  rt_fc;
    RemoteTerminal rt;
    OffsetSmoother smoother;
    SystemHandler  sys;

    SyncRig()
        : bc(tb, bc_clock)
        , rt(tr, rt_clock, rt_fc)
        , sys(0, rt_clock, rt_fc, smoother)
    {
        tb.peer = &tr;
        tr.peer = &tb;
        rt.register_handler(1, &sys);
        // As the generated RT main does: T2 is the command header's arrival.
        sys.set_rx_time_source(&rt);
    }
};

} // namespace


///////////////////////////////////////////////////////////////////////////////
// Scenario A: RT leads BC by 12 ms, zero propagation.
///////////////////////////////////////////////////////////////////////////////

/// With instantaneous frames, T1 == T4 so path_delay is 0 and the offset is the
/// pure clock difference: (T1+T4)/2 - T2 = 1000 - 1012 = -12 (BC - RT). Applying
/// it must put now_bc_ms() back on the BC clock (1000). A wrong-signed offset
/// would instead store +12 and report 1012 + 12 = 1024 -- a doubled error.
TEST_CASE("e2e: RT leading by 12 ms recovers the BC clock (no propagation)", "[sync][e2e]") {
    SyncRig rig;
    rig.bc_clock.m_now_ms = 1000u;
    rig.rt_clock.m_now_ms = 1012u;   // Delta = RT - BC = +12.

    const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
    REQUIRE(r.status == Status::Ok);
    REQUIRE(r.t1 == 1000u);
    REQUIRE(r.t2 == 1012u);
    REQUIRE(r.t4 == 1000u);
    REQUIRE(r.path_delay == Catch::Approx(0.0));
    REQUIRE(r.offset     == Catch::Approx(-12.0));

    REQUIRE(rig.rt_clock.has_sync());
    int32_t whole = 0, frac = 0;
    rig.rt_clock.get_bc_offset(whole, frac);
    REQUIRE(whole == -12);
    REQUIRE(frac  == 0);

    // The corrected RT clock now reads the BC clock, not 2*Delta away from it.
    REQUIRE(rig.rt_clock.now_bc_ms() == rig.bc_clock.now_ms());   // 1012 - 12 == 1000
    REQUIRE(rig.rt_clock.now_bc_ms() == 1000u);
    REQUIRE(rig.rt_clock.now_bc_ms() != 1024u);                   // 2*Delta, the doubled-error value this guards against
}


///////////////////////////////////////////////////////////////////////////////
// Scenario B: RT leads BC by 12 ms with 1 ms/tick propagation.
///////////////////////////////////////////////////////////////////////////////

/// With real per-frame propagation the offset is still recovered to within the
/// (small) framing residual -- bounded by the path delay, NOT ~2*Delta. This is
/// the case that proves the doubling is gone even when delay is non-zero.
TEST_CASE("e2e: offset recovered within the framing residual under propagation", "[sync][e2e]") {
    SyncRig rig;
    rig.bc_clock.m_now_ms = 1000u;
    rig.rt_clock.m_now_ms = 1012u;   // Delta = RT - BC = +12, held constant (lockstep advance).

    const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 1u);
    REQUIRE(r.status == Status::Ok);

    // Serialization check: the returned values are exactly the four-timestamp
    // formulas applied to the timestamps actually captured. T3 - T2 is non-zero
    // here because T2 is the header's arrival and T3 follows the ack and payload.
    const double t1 = r.t1, t2 = r.t2, t3 = r.t3, t4 = r.t4;
    REQUIRE(r.path_delay == Catch::Approx(((t4 - t1) - (t3 - t2)) / 2.0));
    const double expected_offset = ((t1 - t2) + (t4 - t3)) / 2.0;
    REQUIRE(r.offset == Catch::Approx(expected_offset).margin(0.51));

    // T2 is stamped when the SYNC_PROBE header reaches the RT (the first RT tick,
    // RT clock 1012), not when its payload lands after the ack round trip.
    REQUIRE(r.t2 == 1012u);

    // The recovered offset is near the true -12, off only by the framing residual.
    REQUIRE(r.offset == Catch::Approx(-12.0).margin(r.path_delay + 1.0));

    // The corrected clock tracks the BC clock to within that same small residual,
    // and is nowhere near the doubled (~24 ms) error.
    const uint32_t now_bc = rig.rt_clock.now_bc_ms();
    const uint32_t bc_now = rig.bc_clock.now_ms();
    const int64_t  err    = static_cast<int64_t>(now_bc) - static_cast<int64_t>(bc_now);
    REQUIRE(err <= static_cast<int64_t>(r.path_delay) + 1);
    REQUIRE(err >= -(static_cast<int64_t>(r.path_delay) + 1));
}


///////////////////////////////////////////////////////////////////////////////
// Scenario C: RT lags BC by 7 ms, zero propagation (opposite sign).
///////////////////////////////////////////////////////////////////////////////

/// (T1+T4)/2 - T2 = 2000 - 1993 = +7 (BC leads): now_bc_ms() advances the RT tick
/// to meet the BC clock. Exercises the positive branch of apply_bc_offset().
TEST_CASE("e2e: RT lagging by 7 ms recovers the BC clock (no propagation)", "[sync][e2e]") {
    SyncRig rig;
    rig.bc_clock.m_now_ms = 2000u;
    rig.rt_clock.m_now_ms = 1993u;   // Delta = RT - BC = -7.

    const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
    REQUIRE(r.status == Status::Ok);
    REQUIRE(r.path_delay == Catch::Approx(0.0));
    REQUIRE(r.offset     == Catch::Approx(7.0));

    int32_t whole = 0, frac = 0;
    rig.rt_clock.get_bc_offset(whole, frac);
    REQUIRE(whole == 7);
    REQUIRE(frac  == 0);
    REQUIRE(rig.rt_clock.now_bc_ms() == 2000u);   // 1993 + 7
}


///////////////////////////////////////////////////////////////////////////////
// Scenario D: the RT smoother seeds exactly, then holds a steady target.
///////////////////////////////////////////////////////////////////////////////

/// Running the same exchange twice: the first sync seeds the offset exactly
/// (-12), and the second -- measuring the same offset -- leaves it unchanged.
TEST_CASE("e2e: repeated syncs seed then hold the offset", "[sync][e2e]") {
    SyncRig rig;
    rig.bc_clock.m_now_ms = 1000u;
    rig.rt_clock.m_now_ms = 1012u;

    const SyncOutcome r1 = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
    REQUIRE(r1.status == Status::Ok);
    int32_t whole = 0, frac = 0;
    rig.rt_clock.get_bc_offset(whole, frac);
    REQUIRE(whole == -12);   // seeded exactly on the first sync
    REQUIRE(frac  == 0);

    const SyncOutcome r2 = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
    REQUIRE(r2.status == Status::Ok);
    REQUIRE(r2.offset == Catch::Approx(-12.0));
    rig.rt_clock.get_bc_offset(whole, frac);
    REQUIRE(whole == -12);   // still in cold start (zero skew): the estimator holds it
    REQUIRE(frac  == 0);
    REQUIRE(rig.rt_clock.bc_skew_ppb() == 0);
    REQUIRE(rig.rt_clock.now_bc_ms() == 1000u);
}


///////////////////////////////////////////////////////////////////////////////
// Scenario E: a steady skew is recovered, and now_bc_ms() stays on the BC clock
// even between syncs. The regression that proves the drift fix.
///////////////////////////////////////////////////////////////////////////////

/// Inject a clean ramp: each sync advances the RT clock 1000 ms but the BC clock
/// only 999 ms, so the RT gains 1 ms per sync. The measured (BC - RT) offset then
/// falls exactly -1 ms per 1000 ms of RT time -- a -1,000,000 ppb skew. After a full
/// window the estimator must recover that skew, and the rate-corrected now_bc_ms()
/// must keep tracking the BC clock both at the last sync and 5000 ms beyond it
/// (where an offset-only clock would have drifted ~5 ms).
TEST_CASE("e2e: a steady skew is recovered and now_bc_ms keeps tracking the BC clock", "[sync][e2e][skew]") {
    SyncRig rig;
    uint32_t rt = 100000u;
    uint32_t bc = 100000u;

    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        rig.rt_clock.m_now_ms = rt;
        rig.bc_clock.m_now_ms = bc;
        const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
        REQUIRE(r.status == Status::Ok);
        rt += 1000u;   // RT clock runs fast...
        bc += 999u;    // ...by 1 ms per sync relative to the BC clock.
    }

    // Recovered skew is within a small tolerance of the injected -1e6 ppb.
    REQUIRE(rig.rt_clock.bc_skew_ppb() == Catch::Approx(-1000000).margin(2000));

    // At the last sync instant (clock still == anchor) the disciplined clock equals
    // the BC clock; the skew term is zero there.
    {
        const int64_t err = static_cast<int64_t>(rig.rt_clock.now_bc_ms())
                          - static_cast<int64_t>(rig.bc_clock.now_ms());
        REQUIRE(err <= 1);
        REQUIRE(err >= -1);
    }

    // Now advance the RT clock 5000 ms PAST the last sync without re-syncing. The BC
    // clock would have advanced ~4995 ms in real time. The skew correction must keep
    // now_bc_ms() on the BC clock instead of drifting ~5 ms ahead.
    rig.rt_clock.m_now_ms += 5000u;
    const uint32_t bc_extrapolated = rig.bc_clock.m_now_ms + 4995u;  // 999/1000 of 5000
    const int64_t err2 = static_cast<int64_t>(rig.rt_clock.now_bc_ms())
                       - static_cast<int64_t>(bc_extrapolated);
    REQUIRE(err2 <= 1);
    REQUIRE(err2 >= -1);

    // SYNC_FINAL is now telemetry-only: it disciplines now_bc_ms() (Path A, checked above)
    // but must NOT touch the SysTick reload (Path B). That is deferred to SYNC_ADJUST, so
    // after a whole window of telemetry the reload is still exactly nominal.
    REQUIRE(rig.rt_clock.tick_reload() == rig.rt_clock.nominal_tick_reload());
}


///////////////////////////////////////////////////////////////////////////////
// Scenario F: a telemetry phase leaves the reload at nominal; SYNC_ADJUST then
// disciplines the SysTick toward the BC rate in one smoothed, bounded step and
// reports the skew it used and the reload it programmed.
///////////////////////////////////////////////////////////////////////////////

/// Mirror of Scenario E for the hardware path, opposite direction: the RT runs SLOW
/// (each sync advances RT 999 ms while BC advances 1000 ms -> measured skew ~ +1e6 ppb).
/// SYNC_FINAL only collects telemetry; the reload stays at nominal through the whole
/// phase. SYNC_ADJUST then takes the averaged skew and SHORTENS the reload below nominal
/// to speed the RT up -- a *partial* step (it does not jump all the way to the
/// skew-cancelling value), bounded by -10%. The response words echo exactly the skew
/// used and the reload programmed.
TEST_CASE("e2e: SYNC_ADJUST disciplines and reports the SysTick reload", "[sync][e2e][reload]") {
    SyncRig rig;
    uint32_t rt = 100000u;
    uint32_t bc = 100000u;
    SyncOutcome last;
    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        rig.rt_clock.m_now_ms = rt;
        rig.bc_clock.m_now_ms = bc;
        last = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
        REQUIRE(last.status == Status::Ok);
        rt += 999u;    // RT clock runs slow...
        bc += 1000u;   // ...by 1 ms per sync relative to the BC clock.
    }

    const uint32_t nominal = rig.rt_clock.nominal_tick_reload();
    const int32_t  skew    = rig.rt_clock.bc_skew_ppb();
    REQUIRE(skew > 0);                                  // RT slow -> positive averaged skew
    // Telemetry made no hardware change: the reload is still nominal on the RT.
    // (SYNC_FINAL no longer carries the reload; SYNC_ADJUST is the authoritative source.)
    REQUIRE(rig.rt_clock.tick_reload() == nominal);

    // One adjustment: shorten the reload toward the skew-cancelling rate.
    const AdjustOutcome adj = sync_adjust_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock);
    REQUIRE(adj.status == Status::Ok);
    // The reply carries the skew the governor consumed: the phase-anchor average over
    // the whole telemetry phase, which must land on the injected ~+1/999 truth
    // (1001001 ppb). On this noise-free ramp it agrees with the smoother's slope.
    REQUIRE(adj.avg_skew_ppb == Catch::Approx(1001001).margin(2000));
    REQUIRE(adj.reload == rig.rt_clock.tick_reload()); // reply echoes the reload it programmed

    REQUIRE(rig.rt_clock.tick_reload() < nominal);                  // RT slow -> reload shortened
    REQUIRE(rig.rt_clock.tick_reload() >= nominal - nominal / 10u); // bounded by -10%
    // Smoothed: one step does NOT reach the full skew-cancelling reload. The ideal that
    // cancels the skew used is nominal - round(nominal*skew/1e9); a partial-gain step
    // lands above it.
    const int64_t ideal = static_cast<int64_t>(nominal)
                        - (static_cast<int64_t>(nominal) * adj.avg_skew_ppb + 500000000)
                          / 1000000000;
    REQUIRE(static_cast<int64_t>(rig.rt_clock.tick_reload()) > ideal);
}


///////////////////////////////////////////////////////////////////////////////
// Scenario G: a residual skew BELOW the smoother's SNR-gate floor still steps
// the reload, via the phase anchor.
///////////////////////////////////////////////////////////////////////////////

/// The scenario: a residual skew (~18 ppm) that sits below the smoother's gate
/// floor (kSkewSignalMs over the ~44 s window span) is invisible to the smoother,
/// so on its own every SYNC_ADJUST would receive zero and the reload would freeze
/// above the 6 ppm deadband. Reproduce the geometry: at a 2000 ms cadence the
/// window spans 62 s, making the gate floor ~24 ppm. Inject ~16 ppm by stepping
/// the BC clock +1 ms every 31 syncs -- each window holds at most one 1 ms step,
/// so the predicted ramp never clears 1.5 ms and the smoother reports ZERO at
/// every single final. The phase anchor instead grows its baseline across the
/// whole run (186 s), integrates ~2.1 ms of disciplined offset motion, measures
/// ~11 ppm -- above the deadband -- and the reload moves.
TEST_CASE("e2e: a sub-gate residual skew still steps the reload via the phase anchor",
          "[sync][e2e][reload][anchor]") {
    SyncRig rig;
    uint32_t rt = 100000u;
    uint32_t bc = 100000u;

    for (uint32_t k = 0u; k < 94u; ++k) {
        rig.rt_clock.m_now_ms = rt;
        rig.bc_clock.m_now_ms = bc;
        const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
        REQUIRE(r.status == Status::Ok);
        // The smoother must stay gated at EVERY final: the windowed slope from a
        // single in-window step predicts well under kSkewSignalMs of ramp. The
        // smoother alone would therefore stall this run forever -- only the phase
        // anchor advances the reload.
        REQUIRE(rig.rt_clock.bc_skew_ppb() == 0);
        rt += 2000u;
        bc += 2000u;
        if ((k + 1u) % 31u == 0u) {
            bc += 1u;   // BC gains 1 ms every 62 s: ~16 ppm, between deadband and gate
        }
    }

    const uint32_t nominal = rig.rt_clock.nominal_tick_reload();
    REQUIRE(rig.rt_clock.tick_reload() == nominal);     // telemetry never moves it

    const AdjustOutcome adj = sync_adjust_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock);
    REQUIRE(adj.status == Status::Ok);
    // The anchor's long-baseline average: ~2.1 ms of disciplined offset motion over
    // 186 s (the staircase intercept lags the instantaneous truth slightly, which
    // only makes the step conservative). What matters: clearly above the deadband.
    REQUIRE(adj.avg_skew_ppb > kReloadDeadbandPpb);
    REQUIRE(adj.avg_skew_ppb < 17000);                  // sane: at most the ~16 ppm truth
    // The headline the field run never produced: the reload MOVED off nominal
    // (BC gains -> RT slow -> shorten), instead of freezing at skew == 0.
    REQUIRE(adj.reload == rig.rt_clock.tick_reload());
    REQUIRE(rig.rt_clock.tick_reload() < nominal);
    REQUIRE(rig.rt_clock.tick_reload() >= nominal - nominal / 10u);
}


///////////////////////////////////////////////////////////////////////////////
// Scenario H: a reload step is propagated into Path A -- the stored discipline
// and the smoother window -- so the software rate term does not double-count
// the hardware retune (design doc section 7.3).
///////////////////////////////////////////////////////////////////////////////

/// Run Scenario F's slow-RT phase (true skew +1/999 ~ 1001001 ppb) and adjust.
/// The reload step retunes the hardware rate by rate_delta = (new-old)*1e9/old;
/// the residual the software must keep correcting is avg_skew + rate_delta. The
/// old code left the FULL pre-step skew on the clock and pre-step samples in the
/// window, so now_bc_ms() over-corrected by |rate_delta| (~0.75 ms per second)
/// until the window flushed. Now: the stored skew is the residual immediately,
/// now_bc_ms() tracks a BC clock advancing at the residual rate, and the very
/// next fit over the rebased window reports the residual -- not a stale blend.
TEST_CASE("e2e: a reload step rebases the discipline so Path A does not double-count",
          "[sync][e2e][reload][rebase]") {
    SyncRig rig;
    uint32_t rt = 100000u;
    uint32_t bc = 100000u;
    for (uint32_t k = 0u; k < OffsetSmoother::kWindow; ++k) {
        rig.rt_clock.m_now_ms = rt;
        rig.bc_clock.m_now_ms = bc;
        const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
        REQUIRE(r.status == Status::Ok);
        rt += 999u;    // RT slow by 1 ms per BC second, as in Scenario F
        bc += 1000u;
    }

    const uint32_t nominal = rig.rt_clock.nominal_tick_reload();
    const AdjustOutcome adj = sync_adjust_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock);
    REQUIRE(adj.status == Status::Ok);
    REQUIRE(adj.reload < nominal);                      // a real step happened

    // The handler's exact rate-delta arithmetic (round half away from zero).
    const int64_t diff = static_cast<int64_t>(adj.reload) - static_cast<int64_t>(nominal);
    const int64_t num  = diff * 1000000000LL;
    const int32_t rate_delta_ppb = static_cast<int32_t>(
        (num >= 0) ? (num + static_cast<int64_t>(nominal) / 2) / nominal
                   : (num - static_cast<int64_t>(nominal) / 2) / nominal);
    const int32_t residual_ppb = adj.avg_skew_ppb + rate_delta_ppb;

    // 1) The stored discipline skew became the post-step residual immediately --
    //    not the stale pre-step value (which would be ~4x larger here).
    REQUIRE(rig.rt_clock.bc_skew_ppb() == residual_ppb);

    // 2) now_bc_ms() tracks a BC clock that now advances at the residual rate.
    //    Model the retuned RT: over 4000 RT ms the BC gains residual*4000/1e9
    //    ~ +1 ms. With the stale skew the prediction would be ~+4 ms (3 ms off).
    rig.rt_clock.m_now_ms += 4000u;
    rig.bc_clock.m_now_ms += 4001u;
    {
        const int64_t err = static_cast<int64_t>(rig.rt_clock.now_bc_ms())
                          - static_cast<int64_t>(rig.bc_clock.now_ms());
        REQUIRE(err <= 1);
        REQUIRE(err >= -1);
    }

    // 3) The window was rebased onto the new rate, so the very next fit reports
    //    the residual slope -- not the pre-step blend (~31 old samples at
    //    ~1001001 ppb would otherwise dominate and read ~970000 ppb).
    const SyncOutcome r = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
    REQUIRE(r.status == Status::Ok);
    REQUIRE(rig.rt_clock.bc_skew_ppb()
            == Catch::Approx(residual_ppb).margin(5000));
}


///////////////////////////////////////////////////////////////////////////////
// Scenario I: the NTP min-delay clock filter rejects a single slow exchange.
// The headline of the whole refactor.
///////////////////////////////////////////////////////////////////////////////

/// A run of clean, zero-delay exchanges disciplines the clock to the true -10 ms
/// offset (RT leads the BC by 10). Then one exchange hits heavy propagation and
/// so measures a large path delay. The clock filter holds the recent
/// minimum-delay sample instead of disciplining from this slow one: the reported
/// path delay stays near zero (the filtered minimum, NOT this exchange's high
/// reading), the stale-output gate keeps the prior discipline rather than folding
/// the noisier sample into the OLS window, and now_bc_ms() never lurches off the
/// BC clock. The old single-sample path fed every measurement straight into the
/// discipline; this proves the filter now stands between them.
TEST_CASE("e2e: the min-delay clock filter rejects a single slow exchange", "[sync][e2e][filter]") {
    SyncRig rig;
    uint32_t bc = 1000u;
    uint32_t rt = 1010u;   // Delta = RT - BC = +10, so offset BC - RT = -10.

    SyncOutcome clean;
    for (uint32_t k = 0u; k < 5u; ++k) {
        rig.bc_clock.m_now_ms = bc;
        rig.rt_clock.m_now_ms = rt;
        clean = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 0u);
        REQUIRE(clean.status == Status::Ok);
        REQUIRE(clean.path_delay == Catch::Approx(0.0));   // zero-delay seeds
        REQUIRE(clean.offset     == Catch::Approx(-10.0));
        bc += 1000u;   // advance the cadence; the true offset holds at -10
        rt += 1000u;
    }
    REQUIRE(rig.rt_clock.now_bc_ms() == rig.bc_clock.now_ms());   // locked to BC

    // One heavily delayed exchange.
    rig.bc_clock.m_now_ms = bc;
    rig.rt_clock.m_now_ms = rt;
    const SyncOutcome jit = sync_once_e2e(rig.bc, rig.rt, rig.bc_clock, rig.rt_clock, 20u);
    REQUIRE(jit.status == Status::Ok);

    // This exchange's own round trip really was slow.
    const double raw_delay = static_cast<double>(jit.t4 - jit.t1) / 2.0;
    REQUIRE(raw_delay >= 5.0);

    // But the filter reported the held minimum-delay sample, not this one: the
    // reported path delay is the filtered minimum (~0), well below this exchange's
    // own delay.
    REQUIRE(jit.path_delay == Catch::Approx(0.0));
    REQUIRE(jit.path_delay < raw_delay);

    // The discipline was held (the slow sample never entered the window), so the
    // reported offset stays the disciplined -10 and now_bc_ms() stays locked.
    REQUIRE(jit.offset == Catch::Approx(-10.0));
    const int64_t err = static_cast<int64_t>(rig.rt_clock.now_bc_ms())
                      - static_cast<int64_t>(rig.bc_clock.now_ms());
    REQUIRE(err <= 1);
    REQUIRE(err >= -1);
}
