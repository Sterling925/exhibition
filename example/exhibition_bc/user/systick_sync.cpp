// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "systick_sync.h"

#include <cstdio>
#include "host/delay.h"
#include "host/logger.h"


void SyncHelper::push(double tm, double off)
{
    m_t[m_head] = tm;
    m_y[m_head] = off;
    m_head = (m_head + 1) % kN;
    if (m_count < kN) {
        ++m_count;
    }
}


double SyncHelper::fit(double t_last, double y_last, double& residual_out) const
{
    if (m_count < 2) {
        residual_out = 0.0;
        return 0.0;
    }
    double sx = 0.0;
    double sy = 0.0;
    double sxx = 0.0;
    double sxy = 0.0;
    const double n = static_cast<double>(m_count);
    for (int i = 0; i < m_count; ++i) {
        sx  += m_t[i];
        sy  += m_y[i];
        sxx += m_t[i] * m_t[i];
        sxy += m_t[i] * m_y[i];
    }
    const double denom = n * sxx - sx * sx;
    const double slope = (denom == 0.0) ? 0.0 : (n * sxy - sx * sy) / denom;
    const double intercept = (sy - slope * sx) / n;
    residual_out = y_last - (slope * t_last + intercept);
    return slope;
}


SyncHelper::SyncHelper()
{
    m_count = 0;
    m_head = 0;
    m_last_measurement.offset_units = 0;
    m_last_measurement.path_delay_units = 0;
    m_last_measurement.reload_ticks = 0;
    m_last_measurement.units = 0;
    m_sync_count = 0;
    m_phase            = 0;
    m_telemetry_count  = 0;
    m_telemetry_target = 3;   // first telemetry phase length;
    m_metrics_header_written = false;
    for(int32_t i = 0; i < kN; ++ i)
    {
        m_t[i] = 0.0;
        m_y[i] = 0.0;
    }
}


Status SyncHelper::initial_sync(RtClient &client)
{
    // PTP-style time sync: run the exchange a few times to let the RT's
    // offset filter settle. The offset is the RT systick relative to this
    // host's clock; add it to an RT timestamp to express it here.
    // Note 32 is the RT smoother window size.
    for (int i = 0; i < 32; ++i) {

        Status s = client.sync_once(&m_last_measurement);
        ++m_sync_count;
        if (Status::Ok != s) {
            return s;
        }
        delay(50);
    }
    // Read the RT's live SysTick reload back into the BC's view. SYNC_FINAL carries
    // only path+offset, so without this the BC has no reload value until the first
    // single_sync() ADJUST -- the leading CSV rows would log a placeholder 0. This
    // adjust is a deliberate cold-start NO-OP on the RT: the phase anchor has < 5 s
    // baseline (falls back to the windowed skew, which is SNR-gated to 0 this early),
    // so next_tick_reload() stays inside its deadband and the reload is not
    // reprogrammed. Its only effect is to populate RtClient::reload_ticks().
    SyncAdjustResult adj;
    const Status as = client.sync_adjust(&adj);
    if (Status::Ok != as) {
        return as;
    }
    if (client.has_sync()) {
        std::printf("Time sync: offset = %.3f, path delay = %.3f (units 1e-%u s), "
                    "systick reload = %u\n",
                    client.clock_offset_units(), client.path_delay_units(),
                    static_cast<unsigned>(client.sync_units()),
                    static_cast<unsigned>(client.reload_ticks()));
    }
    return Status::Ok;
}


void SyncHelper::log_metrics(Logger& logger, uint32_t current_timestamp, double offset, double skew, double residue, double path_delay, uint32_t reload_ticks)
{
    // The CSV schema lives here (not in the generic Logger): emit the header
    // once, lazily, on the first row so the side-car file is self-describing.
    if (!m_metrics_header_written) {
        logger.metric("bc_time_ms,offset_ms,skew_ppm,residual_ms,path_delay_ms,reload_ticks");
        m_metrics_header_written = true;
    }

    char buf[160] = {};
    std::snprintf(buf, sizeof(buf), "%u,%.6f,%.3f,%.6f,%.6f,%u",
                  static_cast<unsigned>(current_timestamp), offset, skew, residue,
                  path_delay, static_cast<unsigned>(reload_ticks));
    logger.metric(buf);
}


Status SyncHelper::single_sync(RtClient &client, const IClock &clock, Logger &logger)
{
       printf("On iteration %u (phase %u, telemetry %u/%u)\n",
           m_sync_count, m_phase, m_telemetry_count, m_telemetry_target);
    const Status ss = client.sync_once(&m_last_measurement);
    ++m_sync_count;
    if (ss != Status::Ok) {
        char buf[96] = {};
        std::snprintf(buf, sizeof(buf),
                      "sync failed after %u syncs: %s",
                      static_cast<unsigned>(m_sync_count), status_to_string(ss));
        logger.line("ERROR", buf);
        printf("Sync failed (%s) at iteration %u\n", status_to_string(ss), m_sync_count);
        return ss;
    }
    if (client.has_sync()) {
        // The raw offset keeps ramping (the clocks differ in rate); the skew and
        // the residual are what settle. Skew is the slope of offset-vs-BC-time,
        // shown in ppm; residual is the latest offset's deviation from that line.
        const uint32_t bc_now_ms_u = clock.now_ms();
        const double bc_now_ms = static_cast<double>(bc_now_ms_u);
        const double offset_ms = client.clock_offset_units();
        this->push(bc_now_ms, offset_ms);
        double residual_ms = 0.0;
        const double skew_ppm =
            this->fit(bc_now_ms, offset_ms, residual_ms) * 1.0e6;
        std::printf("Time sync: offset = %.3f ms, skew = %+.2f ppm, "
                    "residual = %+.3f ms, path delay = %.3f ms\n",
                    offset_ms, skew_ppm, residual_ms, client.path_delay_units());
        log_metrics(logger, bc_now_ms_u, offset_ms, skew_ppm, residual_ms,
                    client.path_delay_units(), client.reload_ticks());
    }

    // End of a telemetry phase: apply one smoothed reload adjustment, then start the
    // next (longer) phase. The reload only ever changes here, never during telemetry.
    if (++m_telemetry_count >= m_telemetry_target) {
        SyncAdjustResult adj;
        const Status as = client.sync_adjust(&adj);
        if (as == Status::Ok) {
            std::printf("Time sync ADJUST (phase %u, after %u syncs): "
                        "avg skew = %+.3f ppm, systick reload = %u\n",
                        m_phase, m_telemetry_target,
                        static_cast<double>(adj.avg_skew_ppb) / 1000.0,
                        static_cast<unsigned>(adj.reload_ticks));
        } else {
            std::printf("Time sync ADJUST failed: %s\n", status_to_string(as));
            return as;
        }
        m_telemetry_count  = 0;
        m_telemetry_target = 64; // Larger than offset smoother `kWindow`
        ++m_phase;
    }
    return Status::Ok;
}
