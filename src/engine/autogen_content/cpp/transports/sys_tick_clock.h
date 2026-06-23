// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SYS_TICK_CLOCK_H
#define SYS_TICK_CLOCK_H

#include <stdint.h>

#include "main.h"
#include "../communication/i_clock.h"
#include "../communication/sync.h"


/// @brief IClock backed by the CubeMX-managed `HAL_GetTick()` counter.
class SysTickClock : public IClock {
    public:

    SysTickClock()
    {
        m_bc_offset_whole = 0;
        m_bc_offset_fractional = 0;
        m_bc_anchor_ms = 0;
        m_bc_skew_ppb = 0;
        m_has_sync = false;
        // CubeMX programs the SysTick for a 1 kHz tick: reload == SystemCoreClock/1000.
        m_tick_reload = SystemCoreClock / 1000u;
    }

    /// @brief Re-latch the cached reload from the current SystemCoreClock.
    /// @details Must be called once after the PLL / SystemClock_Config() has run.
    ///          File-scope statics are constructed before main(), so the constructor
    ///          captures the pre-PLL reset default (16 MHz); this corrects that.
    void sync_from_core_clock()
    {
        m_tick_reload = SystemCoreClock / 1000u;
    }

    uint32_t now_ms() const override
    {
        return HAL_GetTick();
    }

    /// @brief Local systick disciplined onto the BC clock (offset + skew).
    /// @details Delegates to apply_bc_discipline() (host-unit-tested in sync.cpp),
    ///          which folds the whole- and fractional-millisecond offset into the
    ///          local tick and adds the frequency correction
    ///          skew_ppb * (now - anchor) / 1e9, all with wrap-safe arithmetic, so
    ///          the result tracks the BC clock between syncs instead of drifting.
    uint32_t now_bc_ms() const override
    {
        return apply_bc_discipline(now_ms(), m_bc_anchor_ms,
                                   m_bc_offset_whole, m_bc_offset_fractional,
                                   m_bc_skew_ppb);
    }

    void set_bc_offset(const int32_t whole_ms, const int32_t fractional_ms) override
    {
        // Phase-only update: anchor at the current tick with no skew.
        m_bc_offset_whole = whole_ms;
        m_bc_offset_fractional = fractional_ms;
        m_bc_anchor_ms = now_ms();
        m_bc_skew_ppb = 0;
    }

    void set_bc_discipline(const uint32_t anchor_ms, const int32_t whole_ms,
                           const int32_t fractional_ms, const int32_t skew_ppb) override
    {
        m_bc_anchor_ms = anchor_ms;
        m_bc_offset_whole = whole_ms;
        m_bc_offset_fractional = fractional_ms;
        m_bc_skew_ppb = skew_ppb;
    }

    void get_bc_offset(int32_t &whole_ms, int32_t &fractional_ms) const override
    {
        whole_ms = m_bc_offset_whole;
        fractional_ms = m_bc_offset_fractional;
    }

    int32_t bc_skew_ppb() const override
    {
        return m_bc_skew_ppb;
    }

    uint32_t tick_reload() const override
    {
        return m_tick_reload;
    }

    uint32_t nominal_tick_reload() const override
    {
        return SystemCoreClock / 1000u;
    }

    /// @brief Reprogram the SysTick reload to discipline the local tick rate onto the BC,
    ///        WITHOUT discarding the partially-elapsed millisecond.
    /// @details The earlier implementation called HAL_SYSTICK_Config() -> SysTick_Config(),
    ///          which zeroes SysTick->VAL (and re-writes CTRL and the NVIC priority) on every
    ///          step. Zeroing VAL throws away the current countdown -- a one-directional
    ///          ~1 ms time bleed off HAL_GetTick() on each reload change. Because the bleed
    ///          always pushes now_ms() further behind the BC, it adds error in the very
    ///          direction the governor is chasing, so the loop never settles (see
    ///          time_sync_design.md sec. 7.2). Instead we write only SysTick->LOAD: on
    ///          Cortex-M the reload register is consumed at the NEXT wrap, so the in-progress
    ///          millisecond completes on the old period and the new period takes effect from
    ///          the following tick -- no count is lost and HAL_IncTick() still fires exactly
    ///          once per wrap (the rate retune is a length change, not an extra/skipped
    ///          interrupt). The one edge case is shortening the period below the current
    ///          count (the common convergence direction, RT slow -> faster): if VAL > new
    ///          LOAD the hardware would otherwise run one extra, old-length period before
    ///          reloading, so we clamp VAL down to LOAD to apply the shorter period
    ///          immediately. That clamp advances the phase by at most the difference, once;
    ///          it never bleeds repeatedly. (Writing VAL also clears CTRL.COUNTFLAG, which is
    ///          harmless: HAL_GetTick() reads uwTick, not COUNTFLAG.) Done in a PRIMASK-saved
    ///          critical section so the LOAD/VAL writes are atomic w.r.t. the SysTick ISR and
    ///          HAL_GetTick() readers, and safe from an already-critical caller. CTRL and the
    ///          NVIC priority are deliberately left untouched -- they were set once at boot
    ///          and must not be re-initialized mid-run. The governor (next_tick_reload) keeps
    ///          @p ticks within +/-10% of nominal, so `ticks - 1` stays well inside the
    ///          24-bit LOAD range.
    void set_tick_reload(uint32_t ticks) override
    {
        const uint32_t primask = __get_PRIMASK();
        __disable_irq();
        const uint32_t load = ticks - 1u;   // LOAD == period - 1 on Cortex-M.
        SysTick->LOAD = load;               // Consumed at the next wrap; VAL keeps counting.
        if (SysTick->VAL > load) {          // Shortening below the current count:
            SysTick->VAL = load;            //   apply the shorter period now (one-time nudge).
        }
        m_tick_reload = ticks;
        if (primask == 0u) {
            __enable_irq();
        }
    }

    bool has_sync() const override
    {
        return m_has_sync;
    }

    void set_has_sync(const bool synchronized) override
    {
        m_has_sync = synchronized;
    }

    private:

    /// @brief Whole part of time offset in milliseconds
    int32_t m_bc_offset_whole;

    /// @brief Fractional part of time offset in milliseconds/1000
    int32_t m_bc_offset_fractional;

    /// @brief Local tick the stored offset is referenced to (the discipline anchor).
    uint32_t m_bc_anchor_ms;

    /// @brief Estimated clock skew relative to the BC, parts-per-billion.
    int32_t m_bc_skew_ppb;

    /// @brief Current SysTick reload (full tick count) the governor has programmed.
    ///        Initialized to nominal (SystemCoreClock/1000) and disciplined on SYNC_FINAL.
    uint32_t m_tick_reload;

    /// @brief True once a SYNC_FINAL exchange has stored a BC offset.
    bool m_has_sync;

};


#endif // SYS_TICK_CLOCK_H
