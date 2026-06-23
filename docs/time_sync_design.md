# Time Synchronization (BC ↔ RT)

Sync is a repeating pair: a *telemetry phase* of ~10–20 `SYNC_PROBE`/`SYNC_FINAL`
exchanges that only **measure** (and keep `now_bc_ms()` disciplined), followed by
a single `SYNC_ADJUST` (0x08) that disciplines the **SysTick reload** in one
smoothed, bounded step from the skew the phase averaged. The reload changes at
most once per phase rather than on every sync. The exchange is NTP's
four-timestamp form — the RT captures a response-send instant `T3` so its own
handler time is removed per sample — feeding a sliding 8-stage min-delay clock
filter, so the RT disciplines from the lowest-delay sample of the last eight
exchanges.

---

## 1. Purpose

The Bus Controller (BC) and Remote Terminal (RT) keep their clocks aligned with
an NTP-style four-timestamp exchange. This document explains (a) the flow, (b)
the message formats, and (c) exactly where and in which direction every clock
adjustment is applied ([§4](#4-sign-conventions)).

A growing *raw* offset is, by itself, **expected** — the two boards have
independent oscillators, so `BC − RT` ramps continuously. What is supposed to
settle is the **skew** (the ramp's *slope*) and the **residual**. See
[§6](#6-what-should-converge-vs-what-stays-fixed).

---

## 2. System overview

| | BC (Bus Controller) | RT (Remote Terminal) |
|---|---|---|
| Role | Host; **initiates** every transaction | STM32F407 firmware; **responds** |
| Code | [`example/bc/`](../example/bc/) | [`example/rt/`](../example/rt/) |
| Clock | `std::chrono::steady_clock` | `HAL_GetTick()` (1 kHz SysTick) |
| Transport | Protocol T1 over UART / USB-CDC, half-duplex | same |

Time-sync uses **sub-address 1**, the `SystemHandler`
([`system_handler.cpp`](../example/rt/Core/Src/rt/system/system_handler.cpp)). The BC
driver is the generated `RtClient::sync_once()`
([`rt_client.cpp`](../example/bc/rt_client.cpp)).

The BC is purely passive in the control loop: it sends timestamps and **stores
whatever offset the RT reports** — it does no filtering of its own
([`rt_client.cpp` `sync_once`](../example/bc/rt_client.cpp)). **All correction
logic lives on the RT.**

---

## 3. The exchange & message format

![Message flow](time_sync_message_flow.svg)

*(source: [`time_sync_message_flow.puml`](time_sync_message_flow.puml))*

NTP four-timestamp offset/delay over a single round trip. Four timestamps:

| | Captured by | When |
|---|---|---|
| **T1** | BC | sending `SYNC_PROBE` |
| **T2** | RT | receiving `SYNC_PROBE` (a raw `HAL_GetTick()` value) |
| **T3** | RT | just before sending the `SYNC_PROBE` response (a raw `HAL_GetTick()` value) |
| **T4** | BC | the `SYNC_PROBE` response arrives back at the BC |

Capturing both RT-side instants (T2 receive, T3 send) lets the RT's own handler
processing time, `T3 - T2`, be subtracted out per sample instead of assumed zero as in
the old Cristian three-timestamp scheme.

The same exchange viewed on a time axis — where each of T1–T4 is captured, the two
propagation legs, the RT handling interval (`T3 − T2`), and the round trip (`T4 − T1`):

![Timing diagram](time_sync_timing.svg)

*(source: [`time_sync_timing.puml`](time_sync_timing.puml))*

Two **telemetry** commands, each a `[opcode(1) | units(1) | value(4 LE)]` 6-byte payload,
plus a one-byte **adjustment** command:

1. **`SYNC_PROBE` (0x06)** — `value = T1`. RT caches T1, stamps **T2** on receipt and
   **T3** just before replying, and returns both (**8 bytes**). BC records **T4** when the
   reply lands.
   ([`system_handler.cpp:99`](../example/rt/Core/Src/rt/system/system_handler.cpp#L99),
   [`SyncState::on_probe`](../example/rt/Core/Src/rt/communication/sync.cpp#L200))
2. **`SYNC_FINAL` (0x07)** — `value = T4`. RT now has all four timestamps, **measures**
   this exchange (offset + path delay), pushes the `(offset, delay)` sample into the
   **8-deep min-delay clock filter**, and disciplines `now_bc_ms()` from the selected
   (lowest-delay) sample (Path A). It replies with a **16-byte** result and **no longer
   changes the SysTick reload** — that is deferred to `SYNC_ADJUST`.
   ([`system_handler.cpp:115`](../example/rt/Core/Src/rt/system/system_handler.cpp#L115))
3. **`SYNC_ADJUST` (0x08)** — one-byte `[opcode]`. Issued once, after a telemetry phase.
   The RT takes the long-baseline **phase-anchor** skew (the windowed
   `clock.bc_skew_ppb()` is the cold-start fallback) and applies a single
   smoothed, bounded step to the SysTick reload (Path B), replying with an **8-byte**
   result `[ avg_skew_ppb (i32) | reload (u32) ]`.

The `SYNC_FINAL` response is four little-endian words
([`sync.h`](../example/rt/Core/Inc/rt/communication/sync.h)):

| Bytes | Field | Meaning |
|---|---|---|
| 0–3 | `path_delay_whole` | the **selected (min-delay)** path delay from the 8-stage clock filter (diagnostic) |
| 4–7 | `path_delay_frac` | thousandths of the above |
| 8–11 | `offset_whole` | the **disciplined** offset the RT just applied |
| 12–15 | `offset_frac` | thousandths of the above |

---

## 4. Sign conventions

Everything hinges on one definition and four downstream signs:

```
path_delay = ((T4 - T1) - (T3 - T2)) / 2          (one-way, RTT/2)
offset     = ((T1 - T2) + (T4 - T3)) / 2     ==   BC - RT
```

`T3 - T2` is the RT's own handling time; removing it from the round trip is what
distinguishes the four-timestamp NTP form from the old three-timestamp Cristian one
(which assumed `T3 == T2`). On a `FakeClock` that does not advance inside the handler
(`T3 == T2`) the formulas reduce exactly to the Cristian numbers, which is why the
existing host-test expectations are unchanged.

| # | Quantity | Sign convention | Where |
|---|---|---|---|
| 1 | `offset = ((T1−T2)+(T4−T3))/2` | **= BC − RT.** Positive ⟺ BC clock is *ahead* of RT. | [`compute_sync_fixed` `sync.cpp`](../example/rt/Core/Src/rt/communication/sync.cpp) |
| 2 | `now_bc_ms = now_ms + offset` | To turn an RT tick into BC time you **add** `BC − RT`. Negative offset subtracts (mod 2³²). | [`apply_bc_offset` `sync.cpp:97-112`](../example/rt/Core/Src/rt/communication/sync.cpp#L97) |
| 3 | `skew_ppb` = slope of `offset` vs local tick × 1e9 | Positive ⟺ offset grows as the RT tick advances ⟺ **BC gains on RT ⟺ RT runs slow.** | [`offset_smoother.h:114`](../example/rt/Core/Inc/rt/communication/offset_smoother.h#L114) |
| 4 | frequency term `+ skew·(now − anchor)/1e9` | Positive skew ⟹ **add** a growing term so `now_bc_ms` keeps pace with the faster BC. | [`apply_bc_discipline` `sync.cpp:134`](../example/rt/Core/Src/rt/communication/sync.cpp#L134) |
| 5 | governor `reload ← cur − round(cur·skew/1e9)` | `skew>0` (RT slow) ⟹ **decrease** reload ⟹ SysTick fires sooner ⟹ `HAL_GetTick()` runs **faster** ⟹ RT catches up. | [`next_tick_reload` `sync.cpp:155-189`](../example/rt/Core/Src/rt/communication/sync.cpp#L155) |

---

## 5. How adjustments are made — phased: telemetry (Path A) then adjust (Path B)

![Discipline flow](time_sync_discipline_flow.svg)

*(source: [`time_sync_discipline_flow.puml`](time_sync_discipline_flow.puml))*

The work is split across the two phases.

**Telemetry phase.** Each `SYNC_FINAL` runs
([`system_handler.cpp`](../example/rt/Core/Src/rt/system/system_handler.cpp)):

1. `compute_sync_fixed(T1, T2, T3, T4)` → raw `offset` and `path_delay` (the
   four-timestamp form; `T3 − T2` removes the RT handler time from the round trip).
2. **Min-delay clock filter** ([`clock_filter.h`](../example/rt/Core/Inc/rt/communication/clock_filter.h)):
   `ClockFilter::push(T2, offset, path_delay)` folds the sample into a sliding
   **8-stage** register (NTP's clock filter), and `select()` returns the entry with the
   **lowest** `path_delay` (ties → most-recent). The selected sample — which may be an
   *earlier, cleaner* exchange — is what disciplines the clock, so a single jittery round
   trip never steers it. A **stale-output gate** (`sel.abscissa_ms != m_last_filter_epoch`)
   skips the discipline when `select()` re-returns an already-fed sample, so a held
   min-delay sample is not pushed into the OLS window as a duplicate point.
3. `OffsetSmoother::update(sel.abscissa, sel.offset)` → fits a straight line
   `offset ≈ b + m·t` by least squares over a **32-sample window**, anchored at the
   selected sample's T2
   ([`offset_smoother.h`](../example/rt/Core/Inc/rt/communication/offset_smoother.h)):
   - **intercept `b`** → the disciplined offset at the anchor
   - **slope `m`** → `skew_ppb`
   - Until ≥ 4 samples spanning ≥ 5 s are present, it reports the raw offset with
     `skew = 0` (cold start).
   - **SNR gate:** even past cold start, the slope is released only once the
     offset change it predicts across the window (`|m|·span`) clears the ±0.5 ms timestamp
     quantization by a few LSBs (`kSkewSignalMs = 1.5 ms`). Below that the slope — and
     crucially its *sign* — is noise, so the estimator reports the disciplined intercept
     with `skew = 0` rather than feed the governor a wrong-direction step. This adapts to
     skew magnitude: a small skew needs a longer baseline, a large one releases sooner.
4. **Path A only:** `set_bc_discipline(anchor, offset, skew_ppb)` so `now_bc_ms()` tracks
   the BC clock between syncs. The SysTick reload is **not** touched here. The 16-byte
   reply carries the **selected (min-delay)** path delay in bytes 0–7 and the disciplined
   offset in bytes 8–15.

**Adjustment.** One `SYNC_ADJUST` per phase reads the **phase-anchor** average skew
(`PhaseAnchor::adjust_skew_ppb()`, [`phase_anchor.h`](../example/rt/Core/Inc/rt/communication/phase_anchor.h)):
the average slope of the *disciplined* offset since the last accepted measurement, over a
baseline that keeps **growing** until the integrated offset motion clears a 0.5 ms signal
floor — so even a residual skew far below the smoother's windowed SNR gate eventually
becomes measurable. Before the first baseline exists it falls back to the windowed, gated
`clock.bc_skew_ppb()`. It then runs **Path B**:
`next_tick_reload(...)` → `set_tick_reload(...)`. The step is *smoothed* — the reload moves
only a fraction (`kReloadGainNum/kReloadGainDen`, **3/4**) of the way to the reload
that would fully cancel the skew, then is clamped to **±10% of nominal**; a skew inside the
±6000 ppb deadband is a no-op. Because each step is partial, the clocks lock over
**several** telemetry/adjust cycles: each phase measures the residual skew at the new rate
and the next adjust steps less. (The gain is kept below 1/1 so a single adjust never
overshoots the ideal; the SNR gate guarantees the governor only ever sees a
trustworthy, correct-sign skew.)

When a step actually reprograms the reload, the rate change `rate_delta_ppb =
(new - old) * 1e9 / old` is also **propagated into Path A** so the software rate term does
not double-count the hardware retune: the phase-anchor baseline is dropped
(it was measured under the old rate), every sample in the smoother window is pivoted onto
the new-rate line (`OffsetSmoother::rebase_rate()`), and the clock's discipline is
re-stored with the post-step residual skew (`avg_skew_ppb + rate_delta_ppb`).

| | **Path A — software (telemetry)** | **Path B — hardware (adjust)** |
|---|---|---|
| When | every `SYNC_FINAL` | once per phase, on `SYNC_ADJUST` |
| Call | `set_bc_discipline(anchor, offset, skew)` | `next_tick_reload(...)` → `set_tick_reload(...)` |
| Effect | `now_bc_ms()` returns `now_ms + offset + skew·(now−anchor)/1e9` | a direct `SysTick->LOAD` write physically **retunes the rate of `now_ms()`** |
| Touches `now_ms()`? | **No** — read-only projection | **Yes** — mutates the raw systick |
| Step size | exact (the fitted values) | smoothed partial step (gain 3/4), clamped ±10% |

`set_tick_reload()` writes `SysTick->LOAD = ticks - 1` directly and leaves `VAL`
running (clamping `VAL` down to the new `LOAD` only when the period is *shortened*
below the current count), so the in-progress millisecond is never discarded and
`HAL_IncTick()` still fires exactly once per wrap.

The component model below shows who computes, stores, and applies each piece:

![Clock model](time_sync_clock_model.svg)

*(source: [`time_sync_clock_model.puml`](time_sync_clock_model.puml))*

---

## 6. What should converge vs. what stays fixed

This distinction is essential to interpreting the loop's behavior (and is spelled out in the
BC demo, [`main.cpp:38-45`](../example/bc/main.cpp#L38)):

- **The raw offset `BC − RT` ramps forever — that is physical and expected.** Two
  independent oscillators differ in rate, so the gap grows linearly. Seeing the offset
  number increase is *not* itself a fault.
- **What must settle is the *slope* (skew, in ppm) and the *residual*** (how far the
  latest sample sits off the fitted line). A healthy loop drives the RT's rate toward
  the BC's, so the ramp's slope flattens and the residual → 0.

So the real test is **not** "is the offset growing?" but **"does the skew settle and the
residual fall toward zero?"** The BC prints exactly these each iteration
([`main.cpp:208-216`](../example/bc/main.cpp#L208)):

```
Time sync: offset = … ms, skew = +… ppm, residual = +… ms, path delay = … ms
```

If `skew`/`residual` never settle — or the offset ramp *steepens* instead of
straightening — the rate loop (Path B) is misbehaving.

---

## 7. Diagram index

| File | Kind | Shows |
|---|---|---|
| [`time_sync_message_flow.puml`](time_sync_message_flow.puml) | sequence | BC↔RT message exchange, four-timestamp capture (8-byte probe reply), 16-byte SYNC_FINAL result |
| [`time_sync_discipline_flow.puml`](time_sync_discipline_flow.puml) | activity | the `SYNC_FINAL` pipeline (four-timestamp compute → min-delay clock filter → discipline) and the two correction paths, with sign decisions |
| [`time_sync_clock_model.puml`](time_sync_clock_model.puml) | class/component | who computes/stores/applies the correction (incl. `ClockFilter`); where `now_ms()` is mutated |
| [`time_sync_timing.puml`](time_sync_timing.puml) | timing | T1..T4 on a time axis: the two propagation legs, RT handling (T3-T2), round trip (T4-T1), and the offset/delay formulas |
| [`time_sync_sequence.puml`](time_sync_sequence.puml) | sequence | a simplified message-level view of the four-timestamp exchange and the least-squares smoother |

Render with [`render.sh`](render.sh) (PlantUML 1.2020.02): `cd docs && ./render.sh`.
