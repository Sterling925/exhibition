// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/system/system_handler.h"

#include <cassert>
#include <stdint.h>

#include "main.h"
#include "rt/api_fingerprint.h"
#include "rt/command_ids.h"
#include "rt/common/helper_functions.h"








SystemHandler::SystemHandler(uint16_t device_count, IClock& clock,
                             const FaultCounter& fault_counter, OffsetSmoother& smoother)
    : m_device_count(device_count)
    , m_clock(clock)
    , m_fault_counter(fault_counter)
    , m_smoother(smoother)
    , m_filter()
    , m_last_filter_epoch(0)
    , m_have_filter_epoch(false)
    , m_phase_anchor()
    , m_last_disc()
    , m_reset_pending(false)
{
}


Status SystemHandler::handle(const MessageHeader& cmd_header,
                             uint8_t const* cmd_payload,
                             uint8_t* resp_buf,
                             uint32_t resp_buf_max,
                             uint32_t& resp_len_out)
{
    resp_len_out = 0;

    if (cmd_header.get_size_bytes() < 1 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE0;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint8_t op = cmd_payload[0];
    switch (static_cast<SystemCmd>(op)) {
        case SystemCmd::GetFingerprint: {
            if (resp_buf_max < 4) {
                return Status::HandlerError;
            }
            emplace_u32_le(resp_buf, kApiFingerprint);
            resp_len_out = 4;
            return Status::Ok;
        }
        case SystemCmd::GetDeviceCount: {
            if (resp_buf_max < 2) {
                return Status::HandlerError;
            }
            resp_buf[0] = static_cast<uint8_t>( m_device_count       & 0xFFu);
            resp_buf[1] = static_cast<uint8_t>((m_device_count >> 8) & 0xFFu);
            resp_len_out = 2;
            return Status::Ok;
        }
        case SystemCmd::GetSysTick: {
            if (resp_buf_max < 4) {
                return Status::HandlerError;
            }
            emplace_u32_le(resp_buf, m_clock.now_ms());
            resp_len_out = 4;
            return Status::Ok;
        }
        case SystemCmd::GetCoreDeviceId: {
            if (resp_buf_max < 8) {
                return Status::HandlerError;
            }
            emplace_u32_le(resp_buf,     HAL_GetDEVID());
            emplace_u32_le(resp_buf + 4, HAL_GetREVID());
            resp_len_out = 8;
            return Status::Ok;
        }
        case SystemCmd::GetCpuId: {
            if (resp_buf_max < 12) {
                return Status::HandlerError;
            }
            emplace_u32_le(resp_buf,     HAL_GetUIDw0());
            emplace_u32_le(resp_buf + 4, HAL_GetUIDw1());
            emplace_u32_le(resp_buf + 8, HAL_GetUIDw2());
            resp_len_out = 12;
            return Status::Ok;
        }
        case SystemCmd::SyncProbe: {
            // Capture the receive instant (T2) before any other work.
            const uint32_t rx_now = m_clock.now_ms();
            if (cmd_header.get_size_bytes() < kSyncCmdBytes ||
                resp_buf_max < kSyncProbeRespBytes) {
                return Status::HandlerError;
            }
            const uint8_t  units = cmd_payload[1];
            const uint32_t t1    = extract_u32_le(cmd_payload + 2);
            // Capture the send instant (T3) as the latest handler-side moment before
            // the reply is serialized; the difference T3 - T2 is the RT's own handling
            // time, which the four-timestamp formula removes from the round trip.
            const uint32_t tx_now = m_clock.now_ms();
            resp_len_out = m_sync.on_probe(units, t1, rx_now, tx_now, resp_buf);
            return Status::Ok;
        }
        case SystemCmd::SyncFinal: {
            if (cmd_header.get_size_bytes() < kSyncCmdBytes ||
                resp_buf_max < kSyncFinalRespBytes) {
                return Status::HandlerError;
            }
            const uint32_t t4 = extract_u32_le(cmd_payload + 2);
            // Telemetry step. First compute this exchange's raw four-timestamp
            // (offset, delay) from the cached T1, T2, T3 and this T4.
            SyncFixed result;
            m_sync.on_final(t4, resp_buf, result);   // writes the kSyncFinalCoreBytes core words

            // NTP-style clock filter: push this sample and select the lowest-delay
            // one of the recent window (ties -> newest). That selected sample -- which
            // may be an earlier, cleaner exchange -- is the one we discipline from, so
            // a single jittery round trip never steers the clock.
            m_filter.push(m_sync.t2(), result.offset_whole, result.offset_frac,
                          result.path_delay_whole, result.path_delay_frac);
            ClockFilter::Selected sel;
            m_filter.select(sel);   // always true: we just pushed a sample

            // Stale-output gate: only re-discipline when the filter picked a sample we
            // have not already fed. Otherwise a stale min-delay sample would enter the
            // OLS window as a duplicate point and bias the fit. When skipped, the prior
            // discipline (already on the clock) simply holds.
            if (!m_have_filter_epoch || sel.abscissa_ms != m_last_filter_epoch) {
                // Fold the selected offset into the sliding-window estimator. The
                // estimator fits offset-vs-local-tick to recover both the offset and the
                // clock skew, and the clock applies both so now_bc_ms() tracks the BC
                // clock between syncs instead of drifting at the skew rate (Path A,
                // software). The SysTick reload (Path B, hardware) is NOT touched here --
                // it is disciplined once per telemetry phase by SYNC_ADJUST.
                OffsetSmoother::Discipline disc;
                m_smoother.update(sel.abscissa_ms, sel.offset_whole, sel.offset_frac, disc);
                m_clock.set_bc_discipline(disc.anchor_ms, disc.offset_whole,
                                          disc.offset_frac, disc.skew_ppb);
                m_clock.set_has_sync(true);
                // Feed the long-baseline estimator one disciplined point per accepted
                // exchange (Path B input), and retain the discipline so SYNC_ADJUST can
                // re-store it with a rate-corrected skew after a reload step.
                m_phase_anchor.note_measurement(disc.anchor_ms, disc.offset_whole,
                                                disc.offset_frac);
                m_last_disc         = disc;
                m_last_filter_epoch = sel.abscissa_ms;
                m_have_filter_epoch = true;
            }

            // Report the filtered view: the selected sample's (min-delay) path delay in
            // bytes 0..7, and the recommended (disciplined) BC-minus-RT offset the RT is
            // currently applying in bytes 8..15, so the BC's view matches the RT's.
            emplace_u32_le(resp_buf,      sel.delay_whole);
            emplace_u32_le(resp_buf + 4,  sel.delay_frac);
            emplace_u32_le(resp_buf + 8,  static_cast<uint32_t>(m_last_disc.offset_whole));
            emplace_u32_le(resp_buf + 12, static_cast<uint32_t>(m_last_disc.offset_frac));
            resp_len_out = kSyncFinalRespBytes;
            return Status::Ok;
        }
        case SystemCmd::SyncAdjust: {
            // Adjustment step, issued once per telemetry phase. The skew comes from the
            // long-baseline phase anchor, which keeps integrating offset motion across
            // phases until it clears its noise floor -- so even a residual far below the
            // smoother's windowed SNR gate eventually reaches the governor instead of
            // stalling the reload a few ticks short of lock. Before the first baseline
            // exists, fall back to the windowed (gated) skew the last SYNC_FINAL stored
            // on the clock. One smoothed, bounded step is then applied to the SysTick
            // reload toward the rate that cancels the skew (see next_tick_reload); the
            // partial-step gain means several phases are needed to fully lock, by design.
            if (resp_buf_max < kSyncAdjustRespBytes) {
                return Status::HandlerError;
            }
            int32_t avg_skew_ppb = 0;
            if (!m_phase_anchor.adjust_skew_ppb(avg_skew_ppb)) {
                avg_skew_ppb = m_clock.bc_skew_ppb();
            }
            const uint32_t old_reload = m_clock.tick_reload();
            const uint32_t reload     = next_tick_reload(old_reload,
                                                         m_clock.nominal_tick_reload(),
                                                         avg_skew_ppb);
            // Only reprogram when the value actually changes; set_tick_reload() preserves
            // the in-flight SysTick period (VAL is not reset), so a step costs no time.
            if (reload != old_reload) {
                m_clock.set_tick_reload(reload);
                // The running rate just changed, so every estimate built under the old
                // rate must be corrected, or Path A keeps applying the pre-step skew on
                // top of the retuned hardware (the rate double-count):
                //   - the anchor baseline is invalid: drop it (re-seeds next SYNC_FINAL);
                //   - the smoother's window samples are pivoted onto the new-rate line;
                //   - the clock's discipline skew becomes the post-step residual,
                //     avg_skew_ppb + rate_delta. A larger reload slows the local ms
                //     counter, so the BC-minus-RT slope rises by rate_delta =
                //     (new - old) * 1e9 / old (rounded half away from zero).
                // The discipline's anchor/offset are re-stored unchanged: the phase error
                // from not re-pivoting them is rate_delta * (now - anchor), well under a
                // microsecond per ppm at the sync cadence -- far below ms quantization.
                m_phase_anchor.invalidate_snapshot();
                assert(old_reload > 0u);   // a reload step from 0 is impossible
                const int64_t diff = static_cast<int64_t>(reload)
                                   - static_cast<int64_t>(old_reload);
                const int64_t num  = diff * 1000000000LL;
                const int64_t den  = static_cast<int64_t>(old_reload);
                const int32_t rate_delta_ppb = static_cast<int32_t>(
                    (num >= 0) ? (num + den / 2) / den : (num - den / 2) / den);
                m_smoother.rebase_rate(m_clock.now_ms(), rate_delta_ppb);
                m_last_disc.skew_ppb = avg_skew_ppb + rate_delta_ppb;
                m_clock.set_bc_discipline(m_last_disc.anchor_ms, m_last_disc.offset_whole,
                                          m_last_disc.offset_frac, m_last_disc.skew_ppb);
            }
            // Reply with the averaged skew used and the reload now programmed, so the BC can
            // log convergence (skew settling into the deadband, reload within +/-10%).
            emplace_u32_le(resp_buf,     static_cast<uint32_t>(avg_skew_ppb));
            emplace_u32_le(resp_buf + 4, reload);
            resp_len_out = kSyncAdjustRespBytes;
            return Status::Ok;
        }
        case SystemCmd::SoftReset: {
            m_reset_pending = true;
            return Status::Ok;
        }
        case SystemCmd::ReadMemory: {
            // Command payload: [opcode, uint32 address (LE), uint16 length (LE)].
            if (cmd_header.get_size_bytes() < 7) {
                if (resp_buf_max >= 1) {
                    resp_buf[0] = 0xE0;   // malformed / short payload
                    resp_len_out = 1;
                }
                return Status::HandlerError;
            }
            const uint32_t address = extract_u32_le(cmd_payload + 1);
            const uint16_t length  = extract_u16_le(cmd_payload + 5);
            // The requested size must fit the current max response frame, and the
            // [address, address+length) span must not wrap past the 32-bit range.
            if (length > resp_buf_max ||
                (length > 0u && address > (UINT32_MAX - (length - 1u)))) {
                if (resp_buf_max >= 1) {
                    resp_buf[0] = 0xE1;   // requested size out of range
                    resp_len_out = 1;
                }
                return Status::HandlerError;
            }
            const uint8_t* src = reinterpret_cast<const uint8_t*>(address);
            for (uint16_t i = 0; i < length; ++i) {
                resp_buf[i] = src[i];
            }
            resp_len_out = length;
            assert(resp_len_out <= resp_buf_max);   // invariant after the size check
            return Status::Ok;
        }
        case SystemCmd::GetMessageFaultCounter: {
            if (resp_buf_max < 4) {
                return Status::HandlerError;
            }
            emplace_u32_le(resp_buf, m_fault_counter.value());
            resp_len_out = 4;
            return Status::Ok;
        }
        default:
            if (resp_buf_max >= 1) {
                resp_buf[0] = op;
                resp_len_out = 1;
            }
            return Status::HandlerError;
    }
}
