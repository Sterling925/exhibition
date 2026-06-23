// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef I_CLOCK_H
#define I_CLOCK_H

#include <stdint.h>


/// @brief Abstract monotonic time source injected into the scheduler.
/// @details
/// Wrap of the 32-bit millisecond counter is tolerated because all timeout
/// comparisons use unsigned subtraction.
class IClock {
    public:

    virtual ~IClock() = default;

    /// @brief Current value of a local monotonic millisecond counter.
    virtual uint32_t now_ms() const = 0;

    /// @brief Current BC millisecond counter
    /// @warning This timer may not be monotonic as
    /// it is modified to synchronize with Bus Controller.
    /// @warning If `has_sync() == false` this will return the non
    /// synchronized local timestamp. Check before use!
    /// @details
    /// This returns a timestamp synchronized to the BC definition: the local tick
    /// shifted by the disciplined offset and corrected for the estimated clock skew
    /// (see set_bc_discipline / apply_bc_discipline), so it stays aligned to the BC
    /// clock between syncs rather than drifting at the skew rate.
    /// Intended for use when common timestamps are needed.
    /// @note On BC, `now_ms()  == now_bc_ms()` since there is no offset for itself.
    virtual uint32_t now_bc_ms() const = 0;

    /// @brief Set the calculated millisecond offset from BC clock
    /// @details Phase-only discipline (no skew). Equivalent to set_bc_discipline()
    ///          anchored at now_ms() with a zero skew; kept for callers that only
    ///          have an instantaneous offset.
    virtual void set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms) = 0;

    /// @brief Discipline the BC clock with both an offset and a rate (skew).
    /// @param anchor_ms Local tick at which @p whole_ms / @p fractional_ms hold.
    /// @param whole_ms Whole-millisecond offset (BC - local) at @p anchor_ms (signed).
    /// @param fractional_ms Fractional thousandths of that offset (signed).
    /// @param skew_ppb Clock skew in parts-per-billion: over E ms of local time the
    ///        offset grows by skew_ppb * E / 1e9 ms (positive when the BC clock runs
    ///        faster). Lets now_bc_ms() stay accurate between syncs instead of
    ///        drifting at the skew rate. See apply_bc_discipline() in sync.h.
    virtual void set_bc_discipline(const uint32_t anchor_ms, const int32_t whole_ms,
                                   const int32_t fractional_ms, const int32_t skew_ppb) = 0;

    /// @brief Get current BC time offset
    /// @param[out] whole_ms Whole ms offset
    /// @param[out] fractional_ms Fractional part of offset in ms/1000
    virtual void get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const = 0;

    /// @brief Current estimated clock skew relative to the BC, parts-per-billion.
    /// @return Signed skew (0 before a skew has been estimated, and always 0 on the
    ///         BC, which is its own reference). Exposed for observability and tests.
    virtual int32_t bc_skew_ppb() const = 0;

    /// @brief Current SysTick reload as a full tick count (the HAL_SYSTICK_Config
    ///        argument, e.g. 168000 -- not the LOAD-1 register value).
    /// @details Default 0: only an on-target SysTick clock has a reload to discipline;
    ///          host/BC clocks and unrelated test doubles inherit this no-op.
    virtual uint32_t tick_reload() const { return 0u; }

    /// @brief Nominal SysTick reload (SystemCoreClock / 1000 on target); defines the
    ///        +/- band the discipline may move within. Default 0 (no discipline).
    virtual uint32_t nominal_tick_reload() const { return 0u; }

    /// @brief Apply a new SysTick reload (full tick count) to the hardware.
    /// @details On target this reprograms the timer atomically (see SysTickClock);
    ///          the default is a no-op for clocks with no SysTick to discipline.
    virtual void set_tick_reload(uint32_t /*ticks*/) {}

    /// @brief Is clock synchronized with BC?
    /// @return true = yes, false = no
    /// @note Required for `now_bc_ms()` to work as intended
    virtual bool has_sync() const = 0;

    /// @brief Set true once synchronization with BC is complete
    /// @param synchronized true if synchronized, else false
    virtual void set_has_sync(const bool synchronized) = 0;

};


#endif // I_CLOCK_H
