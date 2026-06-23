# Adding a New Command to a Device

This guide walks through every change needed to add a new command opcode to an
existing device type, and explains how that command becomes visible on both the
Remote Terminal (RT) firmware side and the Bus Controller (BC) host side.

---

## 1. Architecture Overview

Commands flow through three layers:

```
YAML catalog  →  generator (exhibition)  →  C++ source
     │                                         │
     │  autogen_content/cpp/generated/         │  RT firmware
     │    command_ids.h  (enums)               │    <device>_handler.cpp
     │    api_fingerprint.h                    │
     │                                         │  BC host
     │  exhibition_bc/                         │    rt_client.h / rt_client.cpp
     │    rt_client.h (methods)                │    (generated blocking façade)
     │    rt_client.cpp
```

**Single source of truth — the YAML catalog.**
Every opcode for every device type lives in
`src/engine/commands/catalog/<device_type>.yaml`.
Running the generator from a `.ioc` file causes exhibition to read those
catalogs and produce:

- `autogen_content/cpp/generated/command_ids.h` — one `enum class <Device>Cmd`
  per device type with a named constant for each opcode.
- `autogen_content/cpp/generated/api_fingerprint.h` — a 32-bit FNV-1a hash
  over all manifest files.  Changing any opcode changes the fingerprint,
  so an RT firmware and BC host built from different catalogs will disagree at
  runtime and `RtClient::verify_fingerprint()` will return an error.
- `autogen_content/cpp/generated/service_hashes.h` — one
  `constexpr uint32_t kServiceHash<Kind>` per device type: an FNV-1a hash over
  just that kind's command vocabulary, returned at runtime by the universal
  `GET_SERVICE_HASH` command (opcode `0x00`) on every sub-address.
  `RtClient::verify_service_hashes()` reads each sub-address's hash to localize
  *which* handler diverged when the global fingerprint mismatches.
- `api/<sub_addr>_<name>.yaml` — human-readable per-sub-address command
  manifest files documenting each opcode's payload shape.
- `exhibition_bc/rt_client.h` / `rt_client.cpp` — typed blocking methods for
  every catalog command on the BC host side.

**RT side — the handler.**
Each device type has a concrete handler class that inherits `ISubAddressHandler`
(`autogen_content/cpp/communication/i_sub_address_handler.h`).  The handler's
`handle()` method dispatches on the first payload byte (the opcode) using a
`switch` statement.  Adding a command means adding one `case` block.

**BC side — the client.**
The generator creates a typed blocking wrapper around the raw `BusController`
(`autogen_content/cpp/communication/scheduler.h`).  Each catalog command becomes
one method on the device's nested client class inside `RtClient`.  No manual BC
code is needed; regenerating and rebuilding is sufficient.

---

## 2. Step-by-Step

### Step 1 — Add the opcode to the YAML catalog

Open `src/engine/commands/catalog/<device_type>.yaml` and append an entry under
`commands:`.

Required fields:

| Field | Description |
|-------|-------------|
| `name` | SCREAMING_SNAKE_CASE name. Becomes `<Device>Cmd::<PascalCase>` in C++. |
| `opcode` | Hex byte string (`"0x03"`). Must not duplicate any existing entry. Opcode `0x00` is reserved for the universal `GET_SERVICE_HASH` command (auto-added to every catalog and answered by `RemoteTerminal`, not the handler), so device opcodes start at `0x01`. |
| `cmd_payload` | Short English description: byte count and field names. `"0"` for no payload. |
| `resp_payload` | Same format. `"0"` for write-only commands. |

Example — adding `GET_STATUS` (opcode `0x03`) to the UART catalog:

```yaml
# src/engine/commands/catalog/uart.yaml
kind: uart
cpp_enum: UartCmd
commands:
  - name: TX
    opcode: "0x01"
    cmd_payload: "3+ (len_le16, data...)"
    resp_payload: "0"
  - name: RX
    opcode: "0x02"
    cmd_payload: "2 (uint16 length)"
    resp_payload: "N"
  - name: GET_STATUS          # ← new entry
    opcode: "0x03"
    cmd_payload: "1 (opcode only)"
    resp_payload: "1 (status_flags uint8)"
```

### Step 2 — Re-run the generator

Run exhibition with the same `.ioc` file used to generate the project. Exhibition is
interactive — when prompted, supply the same paths used during the original generation:

1. **STM32CubeMX project root** — the directory that contains the `.ioc` file (Exhibition
   auto-detects it; you will only be asked for the `.ioc` path directly if more than one
   is found).
2. **Transport selection** — choose the same UART or USB CDC interface as before.
3. **Generate BC project?** — answer `Y` and supply the same BC output root to regenerate
   the host-side project as well.

Verify the following files changed:

- `autogen_content/cpp/generated/command_ids.h` — new enum member present.
- `autogen_content/cpp/generated/api_fingerprint.h` — `kApiFingerprint` value
  changed.
- `api/<sub_addr>_uart.yaml` (or the relevant device) — new command listed.
- `exhibition_bc/rt_client.h` / `rt_client.cpp` — new blocking method present.

### Step 3 — Implement the RT handler

Open `autogen_content/cpp/devices/<type>/<type>_handler.cpp`.

Inside `<Device>Handler::handle()`, add a new `case` block **before** the
`default:` label of the existing `switch`:

```cpp
case UartCmd::GetStatus: {
    // cmd: [opcode]  resp: [status_flags uint8]
    if (resp_buf_max < 1) {
        return Status::HandlerError;
    }
    resp_buf[0] = read_status_flags(m_huart);   // your HAL call here
    resp_len_out = 1;
    return Status::Ok;
}
```

Rules for every `case` block:

1. **Validate lengths first.**  Check `cmd_len` (from `cmd_header.get_size_bytes()`)
   is at least the minimum your command requires, and that `resp_buf_max` is
   large enough for the response you will write.  Return `Status::HandlerError`
   on failure — the RT will send a fault frame to the BC.

2. **Write response bytes little-endian.**  Use the helpers from
   `autogen_content/cpp/common/helper_functions.h`:
   - `emplace_u16_le(ptr, value)` — 2 bytes
   - `emplace_u32_le(ptr, value)` — 4 bytes
   - Raw assignment for single bytes.

3. **Set `resp_len_out`** to the exact number of bytes written, then return
   `Status::Ok`.

4. **Write-only commands** (no response payload) leave `resp_len_out = 0`
   and return `Status::Ok`.

5. **Do not modify `default:`.**  It already echoes the unknown opcode byte
   and returns `Status::HandlerError`, which is the correct behavior for
   unrecognized opcodes.

#### Complete `handle()` skeleton after the addition (UART example)

```cpp
Status UartHandler::handle(const MessageHeader& cmd_header,
                           uint8_t const* cmd_payload,
                           uint8_t* resp_buf,
                           uint32_t resp_buf_max,
                           uint32_t& resp_len_out)
{
    resp_len_out = 0;

    const uint32_t cmd_len = cmd_header.get_size_bytes();
    if (cmd_len < 1 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) { resp_buf[0] = 0xE0; resp_len_out = 1; }
        return Status::HandlerError;
    }

    const uint8_t op = cmd_payload[0];
    switch (static_cast<UartCmd>(op)) {
        case UartCmd::Tx: { /* ... existing ... */ }
        case UartCmd::Rx: { /* ... existing ... */ }

        case UartCmd::GetStatus: {
            if (resp_buf_max < 1) { return Status::HandlerError; }
            resp_buf[0] = read_status_flags(m_huart);
            resp_len_out = 1;
            return Status::Ok;
        }

        default:
            if (resp_buf_max >= 1) { resp_buf[0] = op; resp_len_out = 1; }
            return Status::HandlerError;
    }
}
```

### Step 4 — Use the command from the BC side

#### Option A — Generated `RtClient` (recommended)

After regenerating and rebuilding the BC project, the device's nested client
class inside `RtClient` gains a typed method whose signature matches the
catalog entry.  Call it directly:

```cpp
// rt_client is your RtClient instance.
uint8_t status_flags = 0;
Status s = rt_client.uart_usart2().get_status(status_flags);
if (s != Status::Ok) { /* handle error */ }
```

No `Request` construction or `tick()` loop is needed — the generated method
handles all of that internally.

#### Option B — Raw `BusController` (advanced)

Use this when you have a `BusController` instance directly and want to avoid
the generated façade.

```cpp
// Sub-address for the target UART device (from sub_addresses.h).
constexpr uint8_t kUartSubAddr = static_cast<uint8_t>(SubAddress::UartUsart2);

// Command payload: just the opcode byte.
uint8_t cmd[1] = { static_cast<uint8_t>(UartCmd::GetStatus) };

// Response buffer.
uint8_t resp[1] = {};

Request req{};
req.sub_address  = kUartSubAddr;
req.priority     = MessagePriorities::Medium;
req.is_read      = true;                        // expects a response payload
req.cmd_size     = bytes_to_payload_size(1);    // next valid PayloadSizes slot
req.cmd_len      = 1;
req.cmd_inline[0] = cmd[0];
req.resp_ext     = resp;
req.resp_buf_max = sizeof(resp);

struct Ctx { bool done; Status s; uint8_t flags; };
Ctx ctx{};

Status reject;
bool ok = bc.submit(req, [](Status s, const MessageHeader&,
                             uint8_t const* payload, uint32_t len, void* user) {
    auto* c = static_cast<Ctx*>(user);
    c->s = s;
    if (s == Status::Ok && len >= 1) { c->flags = payload[0]; }
    c->done = true;
}, &ctx, reject);

if (!ok) { /* queue full or invalid sub-address: inspect reject */ }

// Drive the state machine until the callback fires.
const uint32_t deadline = now_ms() + 8 * kFrameTimeoutMs;
while (!ctx.done && now_ms() < deadline) { bc.tick(); }
```

`bytes_to_payload_size()` is declared in
`autogen_content/cpp/communication/scheduler.h` and maps any byte count to the
smallest valid `PayloadSizes` slot.

---

## 3. Payload Size Constraints

Protocol T1 supports a sparse set of payload sizes:
`0, 2, 4, 6, 8, 10, 12, 14, 16, 32, 64, 128, 256, 512, 1024, 2048` bytes.
Every command and response payload must fit into one of these slots; zero-padding
to the slot boundary is automatic on the BC side.

The RT's scratch buffer is bounded by `kMaxPayloadBytes` (defined in
`autogen_content/cpp/communication/scheduler.h`, default `64`).  The RT rejects
any inbound command whose declared payload size exceeds this limit.  If a new
command needs a larger payload, raise `kMaxPayloadBytes` to the next valid slot
size (128, 256, etc.).

Payloads at or below `kInlinePayloadBytes` (16 bytes) are carried in
`Request::cmd_inline`; payloads above that threshold must point to a
caller-owned buffer via `Request::cmd_ext`, which must remain valid until the
`CompletionFn` callback fires.

---

## 4. Fingerprint and Version Mismatch

Every time the catalog changes, the generator re-computes a 32-bit FNV-1a
fingerprint over all manifest files and stamps it into `api_fingerprint.h` as
`kApiFingerprint`.  Both the RT and BC projects embed the same constant.

At runtime, the BC's `RtClient::verify_fingerprint()` sends a
`SystemCmd::GetFingerprint` query (sub-address 1, opcode `0x01`) and compares
the RT's runtime reply against the compile-time constant.  A mismatch means
at least one side was built from a different catalog revision.

**Rebuild both projects after every catalog change.**

---

## 5. Verification Checklist

After completing the steps above, confirm:

- [ ] New enum value is present in `autogen_content/cpp/generated/command_ids.h`.
- [ ] `api_fingerprint.h` — `kApiFingerprint` differs from the pre-change value.
- [ ] RT firmware compiles cleanly (new `case` block has no warnings).
- [ ] BC host project compiles cleanly (new `RtClient` method is present).
- [ ] `RtClient::verify_fingerprint()` returns `Status::Ok` at runtime.
- [ ] New command returns the expected payload when called from the BC.
- [ ] Sending an unrecognized opcode to the same sub-address still returns
      `Status::HandlerError` (the `default:` path is intact).

---

## 6. Key Files Reference

| File | Purpose |
|------|---------|
| `src/engine/commands/catalog/<type>.yaml` | Source of truth — add the new opcode here |
| `autogen_content/cpp/generated/command_ids.h` | Generated enum (verify new member after regen) |
| `autogen_content/cpp/generated/api_fingerprint.h` | Generated fingerprint (must change after regen) |
| `autogen_content/cpp/devices/<type>/<type>_handler.cpp` | Add the RT `case` block here |
| `autogen_content/cpp/communication/scheduler.h` | `Request`, `BusController`, `RemoteTerminal`, `bytes_to_payload_size()` |
| `autogen_content/cpp/communication/i_sub_address_handler.h` | `ISubAddressHandler` interface and `Status` enum |
| `autogen_content/cpp/system/system_handler.cpp` | Reference example of a complete handler |
| `exhibition_bc/rt_client.h` / `rt_client.cpp` | Generated BC façade (verify new method after regen) |
