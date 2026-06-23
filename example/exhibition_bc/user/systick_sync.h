// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#pragma once

#include <stdint.h>
#include "rt_client.h"

// BC-side least-squares tracker over recent (bc_time_ms, offset_ms) samples, purely
// for the demo print below. The raw offset BC-RT keeps ramping because the host and
// the STM32 are independent oscillators with slightly different rates -- that drift
// is physical and expected. What *settles* is the SLOPE of that ramp: the clock
// skew, in ppm. The fit residual (how far the latest offset sits from the fitted
// line) settles toward zero once the estimate locks on. The RT applies the same idea
// internally (see OffsetSmoother / set_bc_discipline) to keep its now_bc_ms() aligned
// to the BC clock between syncs; this tracker just makes the effect visible here.

class SyncHelper
{
    private:
    SyncMeasurement m_last_measurement;
    static constexpr int kN = 16; // Length of sample history
    double m_t[kN];
    double m_y[kN];
    int    m_count;
    int    m_head;

    uint32_t m_sync_count; // Total number of sync operations
    uint32_t m_phase; // Telemetry phase to read timing vs to adjust systick
    uint32_t m_telemetry_count; // How many syncs have been done since last systick adjustment
    uint32_t m_telemetry_target;   // How many cyncs to do before adjusting systick
    bool     m_metrics_header_written; // Whether the metrics CSV header row has been emitted


    /// @brief push new data to array to average over
    void push(double tm, double off);


    /// @brief Fit y = slope*t + intercept over the window. Returns the slope (ms per ms);
    /// writes the latest sample's deviation from the fit into @p residual_out.
    double fit(double t_last, double y_last, double& residual_out) const;


    public:

    /// @brief  Sync helper for systick timer
    SyncHelper();


    /// @brief Blocking send X sync commands without modifying anything
    /// @note should be called exactly once at startup
    Status initial_sync(RtClient &client);


    /// @brief Write latest values to csv file to reviewing time sync effectiveness
    /// @param logger Logger whose metrics side-car file receives the CSV row
    /// @param current_timestamp Current BC millisecond timestamp
    /// @param offset Offset between RT and BC in milliseconds
    /// @param skew Rate of deviation in time ppm
    /// @param residue Inaccuracy of time adjustment in milliseconds at this moment
    /// @param path_delay Path delay of one way message in milliseconds `RTT/2`
    /// @param reload_ticks Current valys systick reload counter is set to
    void log_metrics(Logger& logger, uint32_t current_timestamp, double offset, double skew, double residue, double path_delay, uint32_t reload_ticks);


    /// @brief Blocking sync and update systick periodically
    /// @note Should be called once or twice a second
    /// @details
    /// Phased time sync: a TELEMETRY phase of several sync_once() exchanges (which only
    /// MEASURE the skew -- no reload change), then ONE sync_adjust() that applies a single
    /// smoothed, bounded SysTick-reload step from the skew the phase averaged. Repeat; the
    /// clocks lock over several cycles because each step is partial.
    Status single_sync(RtClient &client, const IClock &clock, Logger &logger);


};
