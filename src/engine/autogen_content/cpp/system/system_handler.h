// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SYSTEM_HANDLER_H
#define SYSTEM_HANDLER_H

#include <stdint.h>

#include "../communication/i_sub_address_handler.h"
#include "../communication/i_clock.h"
#include "../communication/sync.h"
#include "../communication/offset_smoother.h"
#include "../communication/phase_anchor.h"
#include "../communication/clock_filter.h"
#include "../communication/fault_counter.h"


/// @brief Sub-address handler bound to sub-address 1 for project metadata and
///        the time-synchronization sub-routine.
/// @details
/// Recognized opcodes:
///   - 0x01 GET_FINGERPRINT  : response is the 32-bit API fingerprint
///                             (little-endian) declared in
///                             `api_fingerprint.h`. RT pads to size_4.
///   - 0x02 GET_DEVICE_COUNT : response is the number of registered device
///                             sub-addresses as uint16 (little-endian).
///                             RT pads to size_2.
///   - 0x03 GET_SYS_TICK     : response is the current millisecond uptime
///                             counter (the injected IClock, which wraps
///                             `HAL_GetTick()` on target) as uint32
///                             (little-endian). RT pads to size_4.
///   - 0x04 GET_CORE_DEVICE_ID : response is the ARM core device ID
///                             (`HAL_GetDEVID()`) followed by the revision ID
///                             (`HAL_GetREVID()`), two uint32s
///                             (little-endian). RT pads to size_8.
///   - 0x05 GET_CPU_ID       : response is the 96-bit CPU unique ID, words 0..2
///                             (`HAL_GetUIDw0/1/2()`), three uint32s
///                             (little-endian). RT pads to size_12.
///   - 0x06 SYNC_PROBE       : NTP-style sync step 1. Command payload is
///                             [opcode, units, uint32 T1]; response is the RT
///                             receive instant T2 followed by the RT response-send
///                             instant T3, two uint32s (little-endian,
///                             kSyncProbeRespBytes). The distinct T3 lets the BC's
///                             round trip have the RT handling time removed.
///   - 0x07 SYNC_FINAL       : NTP-style sync step 2 (telemetry). Command payload
///                             is [opcode, units, uint32 T4]; response is four
///                             little-endian words (one-way path delay whole/frac as
///                             uint32, clock offset whole/frac as int32). The RT first
///                             computes this exchange's four-timestamp (offset, delay),
///                             then runs them through the sliding min-delay clock filter
///                             (see `clock_filter.h`): the lowest-delay sample of the
///                             recent window -- not necessarily this exchange -- is what
///                             gets disciplined into its clock (Path A). The path-delay
///                             words report that selected sample's delay; the offset
///                             words report the recommended (smoothed) BC-minus-RT value
///                             the RT just applied, so the BC can store it without
///                             smoothing again. The SysTick reload is untouched here --
///                             it is changed only by SYNC_ADJUST (0x08). The four words
///                             total kSyncFinalRespBytes (16). See `sync.h` for the
///                             fixed-point encoding and `offset_smoother.h` for the
///                             smoothing.
///   - 0x08 SYNC_ADJUST      : Apply one smoothed, bounded SysTick-reload adjustment.
///                             Command payload is [opcode] only; response is two
///                             little-endian words (kSyncAdjustRespBytes, 8): the averaged
///                             skew in parts-per-billion as int32, and the SysTick reload
///                             now programmed as uint32. The skew is the long-baseline
///                             phase-anchor average (see `phase_anchor.h`), which keeps
///                             integrating offset motion across phases until it is
///                             measurable -- so residual skews far below the smoother's
///                             windowed SNR gate still reach the governor; the windowed
///                             (gated) skew is used only as a cold-start fallback before
///                             the first baseline exists. The reload moves a fraction of
///                             the way toward the rate that cancels the skew and stays
///                             within +/-10% of nominal (see `next_tick_reload` in
///                             `sync.h`), so the clocks lock over several telemetry/adjust
///                             cycles. When the reload actually changes, the rate change is
///                             also propagated into Path A (the smoother window is rebased
///                             and the clock's discipline skew is re-anchored) so the
///                             software rate term never double-counts the hardware retune.
///   - 0x09 GET_MESSAGE_FAULT_COUNTER : response is the running count of inbound
///                             messages that failed due to a timeout or a CRC /
///                             malformed frame, as uint32 (little-endian).
///                             RT pads to size_4.
///   - 0x0A SOFT_RESET         : triggers a system reset. The RT sends a
///                             size-0 write-ack so the BC sees Status::Ok,
///                             then sets an internal flag. The caller must
///                             check reset_pending() after tick() and invoke
///                             HAL_Delay(10); HAL_NVIC_SystemReset();
///                             resp_payload: 0.
///   - 0x0B READ_MEMORY        : reads an arbitrary region of memory. Command
///                             payload is [opcode, uint32 address (LE), uint16
///                             length (LE)]; response is `length` raw bytes read
///                             from `address`. The command is rejected with a
///                             one-byte fault (0xE1) if `length` exceeds the
///                             current max response frame (resp_buf_max) or if
///                             `address + length` would wrap past the 32-bit
///                             range, and with 0xE0 on a short payload.
/// All sync timestamps and results are expressed in the unit identified by the
/// command's `units` byte (a negative power of ten; milliseconds == 3).
/// Any other opcode is reported as a one-byte fault response.
class SystemHandler : public ISubAddressHandler {
    public:

    /// @brief Construct with the count of device sub-addresses registered.
    /// @param device_count Number of peripherals on the RT (sub-addresses
    ///        2..(2+device_count-1) inclusive).
    /// @param clock Monotonic millisecond clock used for GET_SYS_TICK and to
    ///        timestamp the sync receive instants T2/T3. Must outlive *this.
    /// @param fault_counter Shared message-fault counter the RemoteTerminal
    ///        increments; exposed by GET_MESSAGE_FAULT_COUNTER. Must outlive *this.
    /// @param smoother Sliding-window estimator that fits the offset-vs-local-tick
    ///        line of successive syncs to recover both the BC offset and the clock
    ///        skew the RT applies via set_bc_discipline(). Must outlive *this.
    SystemHandler(uint16_t device_count, IClock& clock,
                  const FaultCounter& fault_counter, OffsetSmoother& smoother);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    /// @brief Returns true once a SOFT_RESET command has been processed.
    /// @details The caller must poll this after every tick() and, when true,
    ///          call HAL_Delay(10); HAL_NVIC_SystemReset(); to allow the
    ///          write-ack to clear the transport before the reset fires.
    bool reset_pending() const { return m_reset_pending; }

    private:

    uint16_t            m_device_count;
    IClock&             m_clock;
    const FaultCounter& m_fault_counter;
    OffsetSmoother&     m_smoother;
    SyncState           m_sync;

    /// @brief Sliding 8-deep min-delay clock filter (NTP-style). Every SYNC_FINAL
    ///        pushes its raw (offset, delay) sample; the lowest-delay sample of the
    ///        recent window is the one disciplined into the clock. See clock_filter.h.
    ClockFilter         m_filter;

    /// @brief Abscissa (T2) of the filter sample last fed to the smoother. Used to
    ///        skip a SYNC_FINAL whose filter output is unchanged from the previous
    ///        one, so a stale min-delay sample is not pushed into the OLS window as a
    ///        duplicate point. Sentinel m_have_filter_epoch guards the first use.
    uint32_t            m_last_filter_epoch;
    bool                m_have_filter_epoch;

    /// @brief Long-baseline skew estimator for SYNC_ADJUST (Path B only); fed one
    ///        disciplined point per SYNC_FINAL. See phase_anchor.h.
    PhaseAnchor         m_phase_anchor;

    /// @brief Discipline most recently stored on the clock by SYNC_FINAL; kept so
    ///        SYNC_ADJUST can re-store it with a rate-corrected skew after a
    ///        reload step, so the discipline is not applied on top of the
    ///        already-retuned hardware rate (the Path A / Path B rate double-count).
    OffsetSmoother::Discipline m_last_disc;

    bool                m_reset_pending;
};


#endif // SYSTEM_HANDLER_H
