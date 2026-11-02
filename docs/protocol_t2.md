<!--
SPDX-License-Identifier: MIT OR Apache-2.0
SPDX-FileCopyrightText: 2026 https://github.com/Sterling925
-->

# Protocol T2 Specification

## 1. Overview

**Protocol T2** is a compact, half-duplex, frame-based communication protocol
designed for use over byte-stream physical links (UART, USB CDC, RS-422, etc.).
It defines the structure of every frame on the wire: a common six-byte header,
an acknowledgment mechanism, and a fixed set of payload sizes. It does **not**
define the encoding of application payloads — the content carried inside a
payload frame is determined entirely by the application layer.

T2 is derived from [Protocol T1](protocol_t1.md) and is identical to it except
for one optimization: **a 2-byte payload is carried inline in the header**, in
the two bytes T1 uses for the payload CRC. No separate payload chunk is sent
for a 2-byte payload. For a 2-byte command this also removes the intermediate
ack, saving a full round trip; for a 2-byte response it saves the payload chunk
(Section 8.6).

The model is a single **Bus Controller** (BC) paired with one or more
**Remote Terminals** (RTs):

- The BC is the only initiator. It sends command frames and waits for responses.
- Each RT listens, processes commands, and replies. It never transmits
  unsolicited frames.
- Only one transaction is in flight at any time (half-duplex).

### 1.1 Differences from T1

| Area | T1 | T2 |
|---|---|---|
| Header bytes 2–3, size-2 frame | Payload CRC | The 2 payload bytes (inline) |
| Payload chunk after a size-2 header | Sent | Not sent |
| Integrity of a 2-byte payload | Payload CRC | Header CRC |
| 2-byte write | 4 frames | 2 frames |
| 2-byte command with a read response | 5 frames | 3 frames (2 if the response is also 2 bytes) |
| Protocol fault (always 2 bytes) | Header + payload chunk | Header only |
| Framer | Expects a payload chunk after any non-zero size | Expects a payload chunk only for sizes ≥ 4 |

Everything else — header layout, size table, CRC algorithm, sub-addressing,
fault semantics, repeat behavior, configuration — is unchanged. T2 is **not**
wire-compatible with T1 (Section 13).

---

## 2. Scope

Protocol T2 specifies:

- The 6-byte frame header structure and bit layout.
- The supported payload size set and how sizes are encoded in the header.
- Inline carriage of 2-byte payloads in the header.
- The CRC algorithm used for both header and payload integrity.
- The transaction wire flows (command, acknowledgment, response).
- Fault signaling via the header fault bit.
- Sub-address routing within a single physical link.
- Transport-level framing requirements (`send`, `receive`, `reset_rx` semantics).

Protocol T2 does **not** specify:

- How application payloads are encoded or interpreted. That is the
  responsibility of the application (or a higher-level protocol layer).
- Which physical or link layer carries the frames.
- Addressing or discovery beyond the 8-bit sub-address field.
- Encryption or authentication.

---

## 3. Frame Header

Every Protocol T2 frame begins with a 6-byte header. The header is always
present. For frames whose declared payload size is 0 or 2, the header is the
**only** frame on the wire.

### 3.1 Header Byte Layout

```
Byte 0 (flags):
  Bits 7-6  Priority         (2 bits)
  Bit  5    Command flag     (1 bit)
  Bit  4    Large frame      (1 bit)
  Bits 3-1  Transmitted size (3 bits)
  Bit  0    Fault flag       (1 bit)  [INVERTED POLARITY — see below]

Byte 1:
  Bits 7-0  Sub-address      (8 bits)

Bytes 2-3:
  Size 0:   0x0000
  Size 2:   Inline payload   (byte 2 = payload[0], byte 3 = payload[1])
  Size ≥ 4: Payload CRC-16/MODBUS, little-endian

Bytes 4-5:
  Header CRC-16/MODBUS, little-endian
```

### 3.2 Field Descriptions

| Field | Bits | Description |
|---|---|---|
| **Priority** | 2 | Transaction priority. Lower numerical value = higher priority. `0` = Critical, `1` = High, `2` = Medium, `3` = Low. |
| **Command flag** | 1 | `1` = this is a command frame (BC→RT). `0` = this is a response frame (RT→BC). |
| **Large frame** | 1 | Selects the payload size table. `0` = small table (0–14 B). `1` = large table (16–2048 B). |
| **Transmitted size** | 3 | Index into the selected size table (see Section 4). Together with the large-frame bit this selects one of 16 discrete payload sizes. |
| **Fault flag** | 1 | **Inverted polarity: `0` means a fault is present; `1` means no fault.** A default-constructed header is fault-flagged (`0`) until explicitly cleared. This ensures an all-zero or uninitialized buffer cannot be misread as a valid, fault-free frame. |
| **Sub-address** | 8 | Routes the frame to a specific endpoint on the RT. Value `0` is reserved and must not be used. See Section 7. |
| **Payload CRC / inline payload** | 16 | Meaning depends on the declared size. **Size 0:** `0x0000`. **Size 2:** the two payload bytes, in wire order (Section 3.3). **Size ≥ 4:** CRC-16/MODBUS over the payload bytes (Section 6). |
| **Header CRC** | 16 | CRC-16/MODBUS over header bytes 0–3 only (Section 6). Must be recomputed any time bytes 0–3 change. For a size-2 frame this also protects the inline payload. |

### 3.3 Inline Payload

When the declared payload size is 2 (large-frame bit `0`, index `1`), the
payload is carried in header bytes 2–3 and **no payload chunk follows**. This
is mandatory: a sender must not send a separate payload chunk for a size-2
frame, and a receiver must not wait for one.

- Byte 2 holds `payload[0]` and byte 3 holds `payload[1]`, exactly as they would
  have appeared in a payload chunk. The bytes are not reinterpreted.
  Implementations that store bytes 2–3 as a little-endian `uint16_t` field
  (as T1 does for the payload CRC) see the value
  `payload[0] | (payload[1] << 8)`.
- A logical 1-byte payload is padded to 2 bytes (Section 5), so it is also
  carried inline.
- No payload CRC is computed for an inline payload. The header CRC covers
  bytes 0–3 and therefore protects the payload as well.

### 3.4 Header Integrity

A receiver must validate the header CRC before trusting any other field in the
header, including an inline payload. A header whose CRC does not match must be
silently discarded — the sub-address and priority cannot be trusted, so no fault
response is sent.

---

## 4. Payload Sizes

Protocol T2 supports a sparse, fixed set of payload sizes. The 4-bit encoding
(large-frame bit + 3-bit index) maps to byte counts as follows:

| Large-frame bit | Index (bits 3-1) | Payload bytes | Carried in |
|---|---|---|---|
| 0 | 0 | 0 | — |
| 0 | 1 | 2 | **Header bytes 2–3 (inline)** |
| 0 | 2 | 4 | Payload chunk |
| 0 | 3 | 6 | Payload chunk |
| 0 | 4 | 8 | Payload chunk |
| 0 | 5 | 10 | Payload chunk |
| 0 | 6 | 12 | Payload chunk |
| 0 | 7 | 14 | Payload chunk |
| 1 | 0 | 16 | Payload chunk |
| 1 | 1 | 32 | Payload chunk |
| 1 | 2 | 64 | Payload chunk |
| 1 | 3 | 128 | Payload chunk |
| 1 | 4 | 256 | Payload chunk |
| 1 | 5 | 512 | Payload chunk |
| 1 | 6 | 1024 | Payload chunk |
| 1 | 7 | 2048 | Payload chunk |

The sender always chooses the **smallest size that fits** the logical payload.
For sizes ≥ 4 the receiver reads exactly the declared number of bytes as the
payload chunk. There is no 1-byte size; the minimum non-empty payload is 2
bytes, which is always inline.

---

## 5. Payload Padding

When a logical payload does not fill the declared frame size, the remaining
bytes must be padded. The receiver will read the full declared size regardless,
so both ends must agree on a padding convention to avoid misinterpreting
trailing bytes. For an inline payload the padding byte sits in header byte 3
and is covered by the header CRC.

Common conventions:

- **Zero-fill** — append `0x00` bytes to the declared frame size. Simple to
  implement; straightforward to verify in a debugger or logic analyzer.
- **Alignment-padded structs** — define the payload as a packed C/C++ struct
  where the compiler inserts natural alignment padding. Efficient to parse
  on platforms that support unaligned access, but the layout is
  compiler/toolchain dependent and must be documented carefully for
  cross-platform implementations.
- **Length-prefixed** — include an explicit byte count at a known offset
  within the payload so the receiver can determine the valid portion
  regardless of trailing content. More robust to extension but costs at
  least one byte per frame.

Whatever convention is chosen must be applied consistently by both the BC and
all RT implementations that participate in a given protocol deployment.

If an application protocol is designed on top of T2, it should be formed
to take advantage of the given sizes. It can be more efficient to send a response
with some other useful data, instead of just sending some padding with it. In
T2 this matters most at the 2-byte boundary: a command or response that fits
in 2 bytes skips a whole payload chunk (and, for commands, an ack), while a
3-byte one does not.

---

## 6. CRC Algorithm

Protocol T2 uses **CRC-16/MODBUS** for both CRC fields.

| Parameter | Value |
|---|---|
| Width | 16 bits |
| Polynomial | 0x8005 (reflected: 0xA001) |
| Initial value | 0xFFFF |
| Input reflected | Yes |
| Output reflected | Yes |
| Final XOR | 0x0000 |
| Check value | 0xB4C8 (CRC of the ASCII string "123456789") |

### 6.1 Header CRC

Covers header bytes 0–3 (priority/flags byte, sub-address byte, and bytes 2–3,
which hold either the payload CRC or the inline payload). Stored in header
bytes 4–5, little-endian. The header CRC must be recomputed any time bytes 0–3
change.

### 6.2 Payload CRC

Covers the payload bytes that follow the header. Stored in header bytes 2–3
(little-endian) as part of the header. A payload CRC exists only for declared
sizes ≥ 4:

- **Size 0:** no payload is transmitted; bytes 2–3 are `0x0000`.
- **Size 2:** no payload CRC is computed; bytes 2–3 hold the inline payload
  (Section 3.3).

The payload CRC is computed over the full declared frame size — including any
padding bytes — so padding convention directly affects the CRC value. Both
ends must pad identically before computing or verifying the CRC.

---

## 7. Sub-addressing

The sub-address field (byte 1) routes a frame to a specific logical endpoint
(sub-component) on the RT. This allows a single physical link to serve multiple
independent handlers on one RT.

- **Value `0` is reserved** by the protocol and must never appear in a valid
  command or response frame.
- Values `1` through `255` are usable. A specific deployment may cap the
  upper bound lower (an implementation with 8-bit slot tables that only
  allocates 32 entries, for example, treats values ≥ 32 as invalid).
- On a command frame the sub-address identifies the destination handler.
- On a response frame the sub-address echoes which handler produced the response.
- Sub-address assignment (which physical device or function maps to which
  number) is an application-layer concern not defined by Protocol T2.

### 7.1 Reserved universal command — `GET_SERVICE_HASH` (opcode `0x00`)

The first byte of a command payload is the opcode. **Opcode `0x00` is reserved
protocol-wide** for a universal command, `GET_SERVICE_HASH`, that every
registered sub-address answers identically: it returns a 4-byte little-endian
*per-service hash* — an FNV-1a hash over just that sub-address's command/response
vocabulary. No device opcode may reuse `0x00`.

The RT answers `0x00` framework-side, before dispatching to the sub-address's
handler, so the response is uniform and a handler cannot override it. An
unregistered sub-address still faults `InvalidSubAddress` (the routing check runs
first); the universal command never fabricates a hash for an absent service.

On the wire it is a 0-arg read whose response is the 4-byte hash. The opcode
byte travels in the size-2 slot, so in T2 the command is **inline** and the
exchange follows Section 8.4 (inline command, response Y = 4):

```
BC → RT : command header    (cmd=1, sub-addr=A, size=2, bytes 2–3 = {0x00, pad})
RT → BC : response header   (cmd=0, sub-addr=A, size=4, fault=OK)
RT → BC : response payload  (4-byte hash)
```

Two complementary version checks result: the single global *fingerprint* (read
from the system handler) confirms the whole API matches in one comparison; the
per-service hashes localize *which* sub-address diverged when it does not. Every
sub-address of the same peripheral kind reports the same per-service hash,
because the hash covers only the command vocabulary.

---

## 8. Transaction Wire Flow

All transactions are initiated by the BC. Each frame on the wire is either a
6-byte header or a payload block of the size declared in the most recent header.
A header declaring size 0 or size 2 is never followed by a payload block.

In the flows below, **X** is the command payload size and **Y** is the response
payload size.

### 8.1 Write — command payload in a chunk (X ≥ 4)

Unchanged from T1.

```
BC → RT : command header    (cmd=1, sub-addr=A, size=X)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
BC → RT : command payload   (X bytes)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
```

### 8.2 Read — command payload in a chunk (X ≥ 4)

For Y ≥ 4 this is unchanged from T1:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=X)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
BC → RT : command payload   (X bytes)
RT → BC : response header   (cmd=0, sub-addr=A, size=Y, fault=OK)
RT → BC : response payload  (Y bytes)
```

For Y = 2 the response is inline and the final payload chunk is dropped:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=X)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
BC → RT : command payload   (X bytes)
RT → BC : response header   (cmd=0, sub-addr=A, size=2, fault=OK, bytes 2–3 = response)
```

In every sequence both sides know the exact size of the next chunk before it
arrives: after a header declaring size 0 or 2 the next chunk is a header;
after a header declaring size ≥ 4 it is a payload of that size. This lets the
receive drivers read a known length in one shot (lower CPU load), though
falling back to smaller reads is still required to recover from a desync. It
also lets payload sizes be negotiated at run time — a read size can be agreed
per transaction, or rejected if too large for one side to buffer — at the cost
of the extra round trip's latency.

#### Design rationale: exact-size chunks

T2 deliberately keeps T1's intermediate ack for command payloads of 4 bytes or
more. The ack costs a round trip, but it guarantees that **every receiver knows
the exact length of the next chunk before any of it arrives**: a header is always
6 bytes, and the only thing that ever follows a header is either another header
(after size 0 or 2) or a payload of exactly the declared size (after size ≥ 4).
A receive driver can therefore hand a single fixed-length N-byte read to the
peripheral (DMA, FIFO threshold, USB transfer) instead of parsing a byte stream,
and the RT can refuse an oversized payload before it is sent.

Sending the command payload immediately after its header ("eager payload")
would save that round trip for every command, but the RT would then have to
accept a payload it has not sized or approved yet, so it was rejected. The
inline 2-byte payload is the optimization that keeps the rule: it removes the
round trip only where no payload chunk is needed at all.

### 8.3 Inline write (X = 2)

The command header carries the whole command, so the RT has everything it
needs on receipt. The intermediate ack and the command payload chunk are
omitted; the RT replies with a single ack once the command is processed.

```
BC → RT : command header    (cmd=1, sub-addr=A, size=2, bytes 2–3 = command)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
```

### 8.4 Inline read (X = 2)

As with the inline write, the RT replies directly with the response.

For Y ≥ 4:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=2, bytes 2–3 = command)
RT → BC : response header   (cmd=0, sub-addr=A, size=Y, fault=OK)
RT → BC : response payload  (Y bytes)
```

For Y = 2, the whole transaction is two headers:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=2, bytes 2–3 = command)
RT → BC : response header   (cmd=0, sub-addr=A, size=2, fault=OK, bytes 2–3 = response)
```

### 8.5 Size-0 commands (X = 0)

Unchanged from T1. A size-0 write is:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=0)
RT → BC : ack header        (cmd=0, sub-addr=A, size=0, fault=OK)
```

A size-0 read with Y ≥ 4 is:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=0)
RT → BC : response header   (cmd=0, sub-addr=A, size=Y, fault=OK)
RT → BC : response payload  (Y bytes)
```

A size-0 read with Y = 2 is:

```
BC → RT : command header    (cmd=1, sub-addr=A, size=0)
RT → BC : response header   (cmd=0, sub-addr=A, size=2, fault=OK, bytes 2–3 = response)
```

### 8.6 Frame Count Comparison

Frames on the wire per successful transaction:

| Command size X | Response | T1 frames | T2 frames |
|---|---|---|---|
| 0 | ack (write) | 2 | 2 |
| 0 | Y = 2 | 3 | **2** |
| 0 | Y ≥ 4 | 3 | 3 |
| 2 | ack (write) | 4 | **2** |
| 2 | Y = 2 | 5 | **2** |
| 2 | Y ≥ 4 | 5 | **3** |
| ≥ 4 | ack (write) | 4 | 4 |
| ≥ 4 | Y = 2 | 5 | **4** |
| ≥ 4 | Y ≥ 4 | 5 | 5 |

The largest saving is for 2-byte commands, which in T2 never need the
intermediate ack round trip.

---

## 9. Fault Handling

### 9.1 Fault Bit Polarity

The fault bit in byte 0 has **inverted polarity**:

- **`0`** = a fault is being reported.
- **`1`** = no fault (normal frame).

This means a zeroed or uninitialized header is fault-flagged by default. A
receiver should never treat a header with the fault bit cleared as a normal
transaction frame.

### 9.2 RT-Generated Fault Responses

When the RT detects a protocol fault it replaces the normal ack or response with
a fault frame: a header with the fault bit cleared (`0`) carrying a payload that
gives the cause. A fault payload follows the same carriage rules as any other
payload: a 2-byte fault is inline in header bytes 2–3, and a fault of 4 bytes
or more is sent as a payload chunk after the header.

**Self-describing fault payload (v2).** The payload is *self-describing*: its
first byte names the domain that the following bytes are to be interpreted in,
so a receiver dispatches on the domain rather than assuming every fault byte is a
`Status` value.

Fault payload format:

- Byte 0: **fault domain** tag (see table below).
- Byte 1: **reason code** within that domain. For the `Protocol` domain this is a
  `Status` value; for other domains it is domain-specific.
- Bytes 2..N: optional domain-specific detail (handler faults only).

| `FaultDomain` | Value | Byte 1.. is |
|---|---|---|
| `Protocol` | `0` | A `Status` value emitted by the RT protocol layer. |
| `Hal` | `1` | A HAL/driver return code. *Reserved — not emitted yet.* |
| `App` | `2` | Application-defined fault detail supplied by a handler. |

A **protocol-level** fault is always exactly 2 bytes — `{ Protocol, <Status> }`
in the size-2 slot — and is therefore **always inline** in T2: the fault is a
single header with `Protocol` in byte 2 and the `Status` in byte 3.

```
RT → BC : fault header      (cmd=0, sub-addr=A, size=2, fault=0, bytes 2–3 = {Protocol, Status})
```

A **handler-level** fault is `{ App, <handler bytes...> }`: the RT auto-prepends
the `App` tag ahead of whatever the handler wrote, so the host can never mistake
handler/HAL detail for a `Status` code. A handler fault with 0 or 1 detail bytes
fits in 2 bytes and is inline; a longer one is sent as a payload chunk.

> **Size-slot rounding.** Because the prepended domain byte adds one octet and
> the protocol only carries the discrete payload sizes (0, 2, 4, …), a handler
> fault may round up one size slot — e.g. a 2-byte detail becomes 3 bytes and is
> sent in the size-4 slot (last byte zero-padded). In T2 this also moves it from
> inline to a payload chunk. Protocol faults are unaffected (always size-2,
> always inline).

Conditions that cause RT to emit a fault response:

| Condition | Domain | Byte 1.. |
|---|---|---|
| Sub-address is 0, out of range, or has no registered handler | `Protocol` | `InvalidSubAddress` |
| Command payload CRC does not match the header's payload CRC field (X ≥ 4 only) | `Protocol` | `CrcFailPayload` |
| Command's declared payload size exceeds the RT's receive buffer | `Protocol` | `PayloadTooLarge` |
| Application handler returns a non-Ok status | `App` | Handler-supplied bytes |

`CrcFailPayload` cannot occur for an inline (X = 2) command, because there is
no payload CRC. A corrupted inline payload fails the header CRC instead and is
silently dropped (Section 9.4); the BC observes a timeout.

> **Compatibility.** This v2 payload format is a breaking change from the prior
> "byte 0 is a bare reason code" layout. BC and RT are generated together from a
> single engine version, so regenerating both ends keeps them in lockstep; there
> is no on-wire version field, and interop with a pre-v2 peer requires
> regenerating that peer.

### 9.3 BC-to-RT Fault Signal (Reset)

A fault frame traveling **BC → RT** (a command-direction header with the fault
bit cleared) is a **reset signal**. When the RT receives such a frame it
immediately returns to its idle state and drops any in-flight payload, without
emitting an acknowledgment. This gives the BC a way to re-synchronize the RT
after a missed or corrupt transaction without needing a separate control channel.

The BC should send a reset with declared size 0. If a reset header declares
size 2, the RT ignores bytes 2–3. A reset header must not declare a size ≥ 4,
because the RT would then wait for a payload chunk the BC never sends.

### 9.4 Bad Header CRC — Silent Drop

A received header whose header CRC does not match is silently discarded by
the receiver. No fault response is sent because the sub-address and priority
fields cannot be trusted. The sender will observe a timeout. In T2 this also
covers corruption of an inline payload.

---

## 10. Repeats and Idempotency

Protocol T2 has no transaction ID and does not suppress repeated commands.
Every well-formed command header received by the RT is dispatched to the
registered handler — identical back-to-back frames produce two handler
invocations and two responses.

Applications that need idempotency or sequence tracking must encode that
information in the payload at the application layer. This is intentional: most
messages do not need this protection, so only the handlers that do pay the cost
of carrying transaction IDs, and keeping that state in the handler simplifies its
own sequence tracking.

---

## 11. Transport Interface Contract

Protocol T2 operates at the **frame level**, not the byte level. The physical
link beneath it must present a framing interface with the following semantics:

### `send(bytes, length) → int32_t`

Transmit one complete logical frame chunk (either a 6-byte header or a payload
block). Returns `length` on success, `0` if the link is busy or would block,
`-1` on a hard error. The call must be non-blocking.

### `receive(out, max_length) → int32_t`

Return **at most one** complete frame chunk. Returns the number of bytes copied
on success (positive), `0` if no complete chunk is available yet, `-1` on hard
error. The call must be non-blocking. A byte-streamed physical link (UART, USB
CDC, etc.) requires a framing adapter underneath that re-assembles complete
chunks based on the size declared in the preceding header.

The framing adapter must be **T2-aware**: after a header declaring size 0 or
size 2 it expects the next chunk to be a header, and only after a header
declaring size ≥ 4 does it expect a payload chunk of that size. A T1 framer
would treat a size-2 header as announcing a 2-byte payload chunk and absorb the
next header as that payload, desynchronizing the link.

### `reset_rx()`

Abort any in-progress receive and return the framer to the state of hunting for
a fresh 6-byte header. The BC/RT calls this when abandoning a frame (timeout,
CRC failure, transport error, or inbound fault reset signal) so that a
partially-received payload cannot permanently desync framing from protocol
state. `reset_rx()` does not flush the underlying byte source — any residual
bytes in the physical-layer buffer will be realigned when the framer next
searches for a valid header CRC.

A momentary short read (data has not all arrived yet, and the per-frame deadline
has not expired) must **not** trigger `reset_rx()` — the framer's partial
progress must be preserved so the frame completes once the remaining bytes land.

---

## 12. Configuration Parameters

The following parameters tune the implementation. All are compile-time
constants in the reference implementation; a conforming implementation may
expose them differently. They are the same as in T1.

| Parameter | Default | Description |
|---|---|---|
| `kMaxPayloadBytes` | 64 | Maximum payload size the RT will accept and buffer. Commands whose declared payload exceeds this limit are rejected with a `PayloadTooLarge` fault. Must be one of the discrete values in the payload size table (Section 4). |
| `kFrameTimeoutMs` | 50 ms | Per-frame deadline. If a receiver has not received the expected chunk within this window the transaction is abandoned, `reset_rx()` is called, and the BC reports a timeout. |
| `kMaxSubAddresses` | 32 | Size of the per-sub-address slot tables. Usable sub-address range is `1..(kMaxSubAddresses-1)`. The 8-bit field allows up to 255 sub-addresses; raise this constant if a deployment needs more. Memory cost scales linearly. |
| `kBcQueueDepth` | 16 | Capacity of the BC's priority queue. `submit()` returns `QueueFull` when this depth is reached. |
| `kInlinePayloadBytes` | 16 | Payload size at or below which the BC stores bytes in the request struct itself rather than referencing a caller-supplied buffer. Matches the small/large frame boundary. (This is a BC storage detail and is unrelated to the on-wire inline payload of Section 3.3.) |

---

## 13. Compatibility with T1

T2 is **not** wire-compatible with T1, and neither protocol carries an on-wire
version field, so a peer cannot detect which protocol the other end speaks.
The BC, every RT on the link, and the framing adapters on both ends must all be
built for the same protocol.

Mixing them fails on any size-2 frame:

- A T1 receiver treats the inline payload in bytes 2–3 as a payload CRC and
  waits for a 2-byte payload chunk that never arrives, ending in a timeout or a
  framing desync.
- A T2 receiver accepts a T1 size-2 header, whose header CRC is valid, and
  acts on the T1 payload CRC in bytes 2–3 as if it were the payload. The 2-byte
  payload chunk that follows is then taken as the start of the next header,
  and the framer is misaligned until it resynchronizes on a valid header CRC.

Frames of size 0 and size ≥ 4 are byte-identical between T1 and T2, but since
protocol faults are always size 2, a mismatched link breaks on the first fault
even if no application command uses a 2-byte payload.
