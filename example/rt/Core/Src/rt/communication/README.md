<!--
SPDX-License-Identifier: MIT OR Apache-2.0
SPDX-FileCopyrightText: 2026 https://github.com/Sterling925
-->

# Communication

**Protocol T2** — a small message protocol and FIFO scheduler intended to run
in firmware on an embedded ARM core. The protocol semantics are a single
**Bus Controller** (BC) drives all transactions, **Remote Terminals** (RTs)
respond.

## Files

| File | What it is |
|---|---|
| [`message_header.h`](message_header.h) / [`.cpp`](message_header.cpp) | The 6-byte packed header type. CRC-protected. Reused by every frame on the wire. |
| [`scheduler.h`](scheduler.h) / [`.cpp`](scheduler.cpp) | `BusController` and `RemoteTerminal` classes. The FIFO scheduler. |
| [`sync.h`](sync.h) / [`.cpp`](sync.cpp) | PTP-style time-synchronization formulas, fixed-point encode/decode, and the RT-side `SyncState`. HAL-free and unit-tested. |
| [`i_frame_transport.h`](i_frame_transport.h) | Pure-virtual transport interface (DI seam). |
| [`i_clock.h`](i_clock.h) | Pure-virtual clock interface (DI seam). |
| [`i_sub_address_handler.h`](i_sub_address_handler.h) | Pure-virtual per-sub-address handler interface, plus the shared `Status` enum. |

Working examples for every public API are in
[`../tests/test_scheduler.cpp`](../tests/test_scheduler.cpp).

---

## Protocol T2

T2 is [Protocol T1](protocol_t1.md) plus one optimization: a 2-byte payload rides
inline in the header. Full spec: [`protocol_t2.md`](protocol_t2.md).

### The header

Every frame on the wire begins with a 6-byte `MessageHeader`:

| Field | Bits | Notes |
|---|---|---|
| Priority | 2 | `Critical` (0) → `Low` (3). Lower value = higher priority. |
| Command flag | 1 | `1` = command (BC→RT), `0` = response (RT→BC). |
| Large frame | 1 | Selects between the small (0..14 B) and large (16..2048 B) payload size tables. |
| Transmitted size | 3 | Indexes into the size table; together with the large-frame bit gives one of 16 fixed payload sizes. |
| Fault flag | 1 | **Inverted polarity: 0 = fault, 1 = no fault.** Default-constructed `MessageHeader`s are fault-flagged until `set_fault_flag(false)` is called. |
| Sub-address | 8 | Routes the message to a sub-component (e.g. one motor on a multi-motor board). **Value 0 is reserved.** |
| Payload CRC / inline payload | 16 | Size ≥ 4: CRC-16/MODBUS over the payload bytes. Size 2: the two payload bytes themselves (no payload chunk follows). Size 0: zero. |
| Header CRC | 16 | CRC-16/MODBUS over header bytes 0..3 (so it also covers an inline payload). |

Payload sizes are sparse: `{0, 2, 4, 6, 8, 10, 12, 14, 16, 32, 64, 128, 256,
512, 1024, 2048}`. Smaller payloads must be zero-padded up to the next valid
size by whoever produces them — the scheduler does this for handler responses
automatically; for command payloads the caller is responsible.

### Wire flow for a single transaction

The protocol is half-duplex; BC always initiates. Each transmission is one
*frame* = either a 6-byte serialized header or a payload block of the size
declared in the preceding header.

**Write (cmd size = X, X ≥ 4):**

```
BC -> RT : Header (cmd, size = X)
RT -> BC : Header (response, size = 0)        [ack of cmd header]
BC -> RT : Payload (X bytes)
RT -> BC : Header (response, size = 0)        [final ack]
```

**Read (cmd size = X, X ≥ 4, response size = Y):**

```
BC -> RT : Header (cmd, size = X)
RT -> BC : Header (response, size = 0)        [ack of cmd header]
BC -> RT : Payload (X bytes)
RT -> BC : Header (response, size = Y)        [response header]
RT -> BC : Payload (Y bytes)
```

**Size-0 and inline (size-2) cmds collapse the flow:** the header carries the
whole command, so the intermediate ack is omitted.

- Write: just `BC→RT cmd header` then `RT→BC ack`. Done.
- Read: just `BC→RT cmd header` then `RT→BC response header (size=Y)` then
  `RT→BC payload (Y bytes)`.

**A size-2 response** (or fault) is inline in the response header too: no
payload frame follows it. Every chunk's size is still known before it arrives,
so a receive driver can always hand an exact-length read to the peripheral.

**Faults** flip the fault bit on whichever response header reports them and
carry a fault-detail payload of the indicated size (inline when it is 2 bytes). The BC reports a
`RemoteFault` status to its caller after consuming the fault payload.

Fault payloads are self-describing (v2 format): byte 0 is a `FaultDomain` tag
that tells the receiver how to interpret byte 1 onward:

| `FaultDomain` | Value | Byte 1.. is |
|---|---|---|
| `Protocol` | `0` | A `Status` value emitted by the RT protocol layer. |
| `Hal` | `1` | A HAL/driver return code. *Reserved — not emitted yet.* |
| `App` | `2` | Application-defined handler fault detail. |

RT generates fault frames when:
- No handler is registered for the cmd's sub-address, or the sub-address is
  reserved / out of range (2-byte `{Protocol, InvalidSubAddress}` fault frame).
- A handler returns a non-`Ok` `Status` (fault frame: RT prepends
  `FaultDomain::App` ahead of the handler's detail bytes, so the BC can
  distinguish app detail from a `Status` code; may round up one size slot).
- The cmd payload's CRC doesn't match the header, or its length is wrong
  (2-byte `{Protocol, CrcFailPayload}` fault frame).
- The cmd's declared payload exceeds the RT's buffer (2-byte
  `{Protocol, PayloadTooLarge}` fault frame).

Protocol faults are always exactly 2 bytes (fits the `size_2` slot, so they
are always a single inline fault header). The BC
reports `RemoteFault` to its caller and exposes the fault payload; the generated
blocking client inspects byte 0 (`FaultDomain`) and byte 1 (reason) to produce a
log line, e.g. `RemoteFault (remote: PayloadTooLarge)`. A header that fails its
**own** CRC is still silently dropped (the sub-address/priority can't be
trusted), so the BC surfaces that as a `Timeout` rather than a remote fault.

A fault frame traveling the **other** direction (BC → RT) is a reset
signal: when RT sees an inbound header with the fault bit cleared (fault
signaled, per the inverted-polarity convention) it returns to `Idle` and
drops any in-flight payload state, without emitting an ack. This gives BC
a way to resync RT after a missed transaction without needing a separate
control channel.

### Repeats

The protocol does not suppress repeats. Every well-formed command header
is forwarded to the registered handler — identical bytes back-to-back will
cause two handler invocations and two responses. Applications that need
idempotency or sequence tracking should encode that in the payload at the
application layer.

---

## Using the BusController

A BC instance owns a priority queue of pending requests and runs at most one
transaction at a time. It is single-threaded and driven by polling `tick()`.

### 1. Provide a transport and a clock

```cpp
class MyUartTransport : public IFrameTransport {
public:
    int32_t send(uint8_t const* bytes, uint32_t length) override {
        // Non-blocking: push `length` bytes to the UART TX FIFO.
        // Return `length` on success, 0 if the FIFO is full, -1 on hard error.
    }
    int32_t receive(uint8_t* out, uint32_t max_length) override {
        // Non-blocking: return one complete frame chunk if available,
        // 0 if nothing is waiting, -1 on hard error. The chunk is either
        // a 6-byte header or a payload of the size declared in the prior header.
    }
    void reset_rx() override {
        // Abort any in-progress receive and resync to hunting for a 6-byte
        // header. The scheduler calls this when it abandons a frame (timeout /
        // error / CRC fail) so framing cannot desync from protocol state.
    }
};

class MySysTickClock : public IClock {
public:
    uint32_t now_ms() const override { return HAL_GetTick(); }
};
```

The transport contract is **frame-level**: every call to `send()` writes one
logical chunk, and every call to `receive()` returns *at most* one chunk. If
your physical link is byte-streamed (UART, etc.) you need a small framing
adapter underneath that re-assembles complete chunks based on the size in the
header. That adapter must also implement `reset_rx()`: the scheduler calls it
when it abandons a frame (timeout / error / CRC failure) so a partially-received
payload cannot leave the framer permanently out of step with the protocol. For
tests, the fake transport in
[`test_scheduler.cpp`](../tests/test_scheduler.cpp) shows the simplest form.

### 2. Construct the BC

```cpp
MyUartTransport transport;
MySysTickClock  clock;
BusController   bc(transport, clock);
```

### 3. Submit a request

```cpp
// Write 2 bytes to sub-address 5.
uint8_t cmd[2] = { 0x12, 0x34 };
Request req{};
req.sub_address  = 5;
req.priority     = MessagePriorities::Medium;
req.is_read      = false;
req.cmd_size     = PayloadSizes::size_2;
req.cmd_len      = 2;
req.cmd_ext      = nullptr;
std::memcpy(req.cmd_inline, cmd, 2);
req.resp_ext     = nullptr;
req.resp_buf_max = 0;

Status reject_reason = Status::Ok;
bool ok = bc.submit(req, my_completion, &my_user_ctx, reject_reason);
// `ok == false` means rejected; `reject_reason` says why (InvalidSubAddress,
// PayloadTooLarge, QueueFull).
```

For a **read**, set `req.is_read = true` and supply a response buffer:

```cpp
req.is_read      = true;
req.resp_buf_max = 16;        // Inline storage is used for ≤ 16 bytes.
// For larger responses, point at a caller-owned buffer:
//   req.resp_ext      = my_resp_buf;
//   req.resp_buf_max  = sizeof(my_resp_buf);
```

### 4. Wire up a completion callback

```cpp
void my_completion(Status status,
                   const MessageHeader& resp_header,
                   uint8_t const* resp_payload,
                   uint32_t resp_len,
                   void* user) {
    if (status == Status::Ok) {
        // resp_payload + resp_len are the response bytes (or nullptr/0 for writes).
    } else {
        // status is one of: Timeout, RemoteFault, CrcFailHeader,
        // CrcFailPayload, TransportError, PayloadTooLarge.
        // For RemoteFault, resp_payload holds the fault-detail bytes RT supplied.
    }
}
```

`resp_payload` is **valid only for the duration of the call** — copy out
anything you need to keep. A callback may safely call `bc.submit()` again
(the slot is reset to Idle before the callback fires); it must not call
`bc.tick()`.

### 5. Drive `tick()`

```cpp
while (running) {
    bc.tick();
    // ... other periodic work ...
}
```

`tick()` is non-blocking and performs at most one `transport.send()` plus one
`transport.receive()` per call. Call it as often as you can afford; each call
advances the active transaction by one state-machine step.

### Buffer ownership

Payloads up to `kInlinePayloadBytes` (16) bytes are copied into the `Request`
struct at submit time — no lifetime concerns. Larger payloads use caller-owned
buffers (`cmd_ext`, `resp_ext`) which **must remain valid until the completion
callback fires**. This is not compile-time enforced, so be careful.

---

## Using the RemoteTerminal

An RT instance listens for commands and dispatches them to a handler per
sub-address.

### 1. Implement a handler

```cpp
class MotorHandler : public ISubAddressHandler {
public:
    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override {
        // Read cmd_header.get_size_bytes() bytes from cmd_payload.
        // Do the work. For a read, write response bytes into resp_buf and
        // set resp_len_out. For a pure write, leave resp_len_out = 0.
        // Return Status::Ok on success; any other value causes RT to emit a
        // fault frame whose payload is `resp_buf[0..resp_len_out]`
        // interpreted as fault detail.
        return Status::Ok;
    }
};
```

Handlers run **synchronously inside `RemoteTerminal::tick()`**. A slow handler
stalls the bus. They may register or unregister handlers for other sub-
addresses but must not call `tick()`.

### 2. Construct the RT and register handlers

```cpp
FaultCounter   rt_faults;
RemoteTerminal rt(transport, clock, rt_faults);
MotorHandler   axis_x_handler;
MotorHandler   axis_y_handler;
rt.register_handler(1, &axis_x_handler);
rt.register_handler(2, &axis_y_handler);
// Pass nullptr to unregister: rt.register_handler(1, nullptr);
```

`register_handler` returns false for sub-address 0 or out-of-range values.
Unregistered sub-addresses cause RT to emit a fault response to the BC carrying
the `InvalidSubAddress` code in byte 0 of its payload.

### 3. Drive `tick()`

```cpp
while (running) {
    rt.tick();
    // ... other periodic work ...
}
```

Same contract as BC: non-blocking, advances one state per call, not reentrant.

---

## Configuration

These constants live at the top of [`scheduler.h`](scheduler.h):

| Constant | Default | Purpose |
|---|---|---|
| `kInlinePayloadBytes` | 16 | Payload size at or below which buffers are stored inline in `Request` / BC response slot. Matches the small/large frame boundary in `PayloadSizes`. |
| `kMaxSubAddresses` | 32 | Slot table size for both BC and RT. Usable sub-address range is 1..31. The protocol's 8-bit field allows up to 255; raise this constant if a deployment needs more. Memory cost scales linearly: roughly ~130 B/slot on BC (`BcSlot` holds a full `Request` plus inline payload/response buffers and both headers) and one handler pointer per slot on RT. |
| `kBcQueueDepth` | 16 | BC priority-queue capacity. `submit()` returns `QueueFull` when exceeded. |
| `kFrameTimeoutMs` | 50 | Per-frame deadline. If a `Wait…` state hasn't progressed within this window, BC fires the completion callback with `Status::Timeout`; RT silently returns to `Idle`. |
| `kMaxPayloadBytes` | 64 | Max payload size the RT will accept and buffer. RT rejects any inbound command whose declared payload exceeds this limit (`PayloadTooLarge` fault). Must be one of the discrete payload-size values; raise it (128, 256, …) if a command needs a larger payload. |

---

## Status codes

| Code | Source | Meaning |
|---|---|---|
| `Ok` | Both | Transaction completed normally. |
| `QueueFull` | BC submit | `kBcQueueDepth` requests already pending. |
| `InvalidSubAddress` | BC submit / RT register / RT fault payload | Sub-address is 0 or ≥ `kMaxSubAddresses`, or no handler is registered for it. RT puts this code in the fault payload. |
| `PayloadTooLarge` | BC submit / BC recv / RT fault payload | Submitted `cmd_len > kMaxPayloadBytes`, response payload doesn't fit in the supplied buffer, or the cmd's declared payload exceeds the RT buffer (RT fault payload). |
| `Timeout` | BC tick | RT did not respond within `kFrameTimeoutMs` (includes a cmd header dropped for a bad header CRC). |
| `RemoteFault` | BC tick | RT set the fault bit on the response header. `resp_payload` byte 0 is the `FaultDomain` tag; byte 1 is the reason code (a `Status` for `Protocol`-domain faults). The blocking client decodes both into the log line. |
| `CrcFailHeader` | BC tick | A header the BC received failed its own CRC. (RT silently drops cmd headers with a bad CRC rather than faulting.) |
| `CrcFailPayload` | BC tick / RT fault payload | Received payload bytes didn't match the CRC declared by the header, or had the wrong length. RT puts this code in the fault payload. |
| `TransportError` | Both | `IFrameTransport::send`/`receive` returned `-1`. |
| `HandlerError` | RT-side | A handler may return this (or any non-`Ok` code) to trigger a fault response. |

---

## Time synchronization

The system handler (sub-address 1) carries a time-synchronization sub-routine
(NTP-style four-timestamp offset/delay with a sliding min-delay clock filter) that
demonstrates how this protocol can recover the offset
between an RT's local millisecond systick and the BC's clock, plus the one-way
path delay. The math lives in [`sync.h`](sync.h) / [`sync.cpp`](sync.cpp); the BC
façade is the generated `RtClient::sync_once()`.

### The timestamps

| | Captured by | When |
|---|---|---|
| **T1** | BC | sending `SYNC_PROBE` |
| **T2** | RT | the `SYNC_PROBE` command header arrives |
| **T3** | RT | just before sending the `SYNC_PROBE` response |
| **T4** | BC | the `SYNC_PROBE` response arrives |

The BC runs three command types against sub-address 1:

1. **`SYNC_PROBE` (0x06)** — 6-byte payload `[opcode, units, uint32 value]`, `value` = T1.
   The RT caches T1, takes T2 from the command header's arrival (before the ack and
   payload legs, so the forward path matches the one-leg return) and T3 just before replying, and returns both
   (two `uint32`, `size_8`). The BC records T4 when the reply lands.
2. **`SYNC_FINAL` (0x07)** — 6-byte payload `[opcode, units, uint32 value]`, `value` = T4.
   The RT now has all four timestamps, measures this exchange, pushes the
   `(offset, delay)` sample into the 8-deep min-delay clock filter, and disciplines
   `now_bc_ms()` from the lowest-delay sample. Replies with the fixed-point result (16 bytes, `size_16`):
   `path_delay_whole (uint32)`, `path_delay_frac (uint32)`, `offset_whole (int32)`,
   `offset_frac (int32)`. Does **not** alter the SysTick reload.
3. **`SYNC_ADJUST` (0x08)** — 1-byte payload `[opcode]`. Issued once after a telemetry
   phase of repeated `SYNC_PROBE`/`SYNC_FINAL` exchanges. The RT reads the skew the
   phase averaged and applies a smoothed, bounded step to the SysTick reload. Replies with
   8 bytes: `avg_skew_ppb (int32)`, `reload (uint32)`.

### The formula

```
path_delay = ((T4 - T1) - (T3 - T2)) / 2      # one-way (RTT/2)
offset     = ((T1 - T2) + (T4 - T3)) / 2      # BC clock minus RT clock
```

This is the NTP four-timestamp form: capturing both RT-side instants (T2 receive,
T3 send) lets the RT's own handling time `T3 - T2` be removed from the round trip
instead of assumed zero, as the older Cristian three-timestamp scheme did (it set
`T3 == T2`). The offset is the **BC clock minus the RT clock**, the correction the
RT *adds* to its local tick to recover BC time, positive when the BC clock leads.
Timestamp differences use wrap-safe signed subtraction (the millisecond counter
rolls over about every 49.7 days), then the math runs in `double` and is split into
fixed-point pairs for the wire. The `SYNC_FINAL` response is four little-endian words:
`path_delay_whole (uint32)`, `path_delay_frac (uint32)`, `offset_whole (int32)`,
`offset_frac (int32)`. The fractional part is thousandths of a unit; the offset is
signed (whole and fractional share a sign), so a value of -0.5 encodes unambiguously
as `{0, -500}`. The path-delay words are the **selected (min-delay)** sample from the
clock filter, and the offset words are the **disciplined offset** the RT just applied
to its own clock (see *Clock discipline* below).

### Units

The `units` byte is the magnitude of the negative power of ten one unit
represents in seconds (milliseconds = 3, microseconds = 6, …). Exhibition's
clocks are millisecond systicks, so `units` is 3 in practice; the field keeps the
exchange unambiguous if a higher-resolution clock is substituted.

### Clock discipline

The RT is the single source of truth for the clock offset. Each `SYNC_FINAL` first
pushes the exchange's `(offset, path delay)` into the 8-deep min-delay `ClockFilter`
(see [`clock_filter.h`](clock_filter.h)) and selects the lowest-delay sample of the
recent window, the one least contaminated by queueing jitter (the way NTP picks its
clock-filter output). That selected sample is then fed into `OffsetSmoother` (see
[`offset_smoother.h`](offset_smoother.h)), which fits a least-squares line over a
32-sample window to recover both the **offset** (intercept at the latest sample)
and the **skew** (slope, in ppb). It uses this to:

- **Software path (every `SYNC_FINAL`):** call `set_bc_discipline()` so that
  `now_bc_ms()` tracks the BC clock between syncs using the fitted offset and skew
  term. The raw `HAL_GetTick()` systick is not touched.
- **Hardware path (every `SYNC_ADJUST`):** call `next_tick_reload()` →
  `set_tick_reload()` to physically retune the SysTick reload register by a
  smoothed partial step, gradually steering the RT's tick rate toward the BC's.

`RtClient::sync_once()` stores the `SYNC_FINAL` offset directly; the BC does no
additional filtering. Call it repeatedly (a telemetry phase of ~10-20 probes) before
issuing `SYNC_ADJUST` to let the estimator converge. `path_delay_units()` is the
filtered min-delay value reported by the latest exchange.

> **Resolution note.** With both ends on millisecond clocks the raw differences
> are coarse; the fractional fields are populated only by the `/2` in the path
> delay. The scheme — not sub-millisecond accuracy — is what this demonstrates.
> Substituting a microsecond clock behind `IClock` and `units = 6` would sharpen
> it without touching the protocol.

---

## Constraints and gotchas

- **Only one transaction is in flight across the BC at any time.** The
  per-sub-address slot tables exist for bookkeeping; concurrency across
  sub-addresses was traded away for half-duplex simplicity. Lower-priority
  requests wait until the current one completes.
- **Handler `resp_len_out` is rounded up to a valid `PayloadSizes`.** If a
  handler writes 5 bytes, RT pads to 6 (`size_6`) before sending. On the BC
  side the callback sees the rounded-up `resp_len`. Callers should interpret
  trailing bytes accordingly.
- **Caller buffer lifetime is not compile-time enforced.** `cmd_ext` and
  `resp_ext` must stay valid from `submit()` until the completion callback
  fires.
- **Default-constructed `MessageHeader`s are fault-flagged** because of the
  inverted fault-bit polarity, and remain so until `set_fault_flag(false)`
  is called. They are not valid on the wire, though, until
  `update_header_crc()` has run — an all-zero buffer fails the header CRC
  check on receive and is silently dropped rather than treated as a fault.
- **`tick()` is not reentrant.** Completion callbacks (BC) and handlers (RT)
  may submit new work or register handlers but must not call `tick()` again.
- **Single automatic retry on transient failures.** When the BC detects a
  timeout or a CRC / malformed-frame failure (a header/payload it cannot
  validate, or a `RemoteFault` whose reason byte is `CrcFailPayload`) it
  re-sends the whole transaction once, starting again from the command header.
  If the retry also fails, the completion callback fires with the failure
  status as before. A `RemoteFault` from a handler error (or an
  `InvalidSubAddress` / `PayloadTooLarge` reason) is **not** retried — the
  command was parsed correctly, so a retry would fail identically.
- **No protocol-level deduplication.** The wire format has no transaction ID,
  and neither BC nor RT suppresses repeats: the single retry re-runs the
  handler on the RT, so handlers must tolerate a re-executed command (or carry
  application-layer sequence numbers / idempotency tokens).
- **Message-fault counters.** Both ends keep a `uint32_t` count of messages
  that failed due to a timeout or a CRC / malformed frame — every failed
  attempt is counted, including ones later recovered by the retry. The BC
  exposes it via `BusController::fault_count()`; the RT increments a shared
  `FaultCounter` that the host can read with the system command
  `GET_MESSAGE_FAULT_COUNTER` (sub-address 1, opcode `0x09`).

---

## Running the tests

From [`../tests/`](../tests/):

```
cmake --preset Debug
cmake --build --preset Debug
ctest --preset Debug
```

Builds the Catch2 binaries (to `build/Debug/`) and runs them in sequence:

- `test_crcs` — CRC implementations.
- `test_message_header` — header packing, CRCs, round-tripping.
- `test_scheduler` — BC + RT, end-to-end via paired fake transports, fault and
  timeout paths.
- `test_transports` — frame transport implementations.
- `test_helper_functions` — little-endian (de)serialization helpers.
- `test_sync` — time-sync formulas, fixed-point encode/decode, and `SyncState`.
