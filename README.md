# Exhibition

A code-generation CLI that reads an STM32CubeMX `.ioc` file and produces two ready-to-use
C++ projects for PCBA test firmware — one that runs on the target STM32 and one that drives
it from a host PC.

## Status

[![No Maintenance Intended](https://img.shields.io/badge/No%20Maintenance%20Intended-x-red.svg)](https://unmaintained.tech/)

This is a personal project I maintain for my own experimentation. I'm not accepting pull
requests or feature requests, but you're very welcome to **fork** it — it's dual-licensed
under MIT OR Apache-2.0, so build on it however you like. See
[CONTRIBUTING.md](CONTRIBUTING.md) for details.

## About

Exhibition auto-generates a test firmware project for PCBA testing.

The tool reads a `.ioc` file from an STM32CubeMX project and generates a complete interface
to control the defined hardware. The generated projects allow setting and reading GPIO pins,
sampling ADC/DAC channels, driving PWM outputs, and performing bus operations over SPI,
UART, I2C, and CAN.

The goal is to expose a structured API and the drivers needed for PCBA test operations
without requiring production firmware. Hardware validation can begin as soon as the board is
assembled, independent of any application software.

## How It Works

Exhibition runs a three-phase pipeline:

1. **Parse** — The `ioc_parser` module reads the `.ioc` file and extracts every configured
   peripheral (GPIO, ADC, DAC, PWM, SPI, UART, I2C, CAN/FDCAN, USB) into a `DeviceSet`.

2. **Plan** — The `engine` assigns a deterministic sub-address (1–31) to each peripheral
   and, from an authoritative per-device-type command catalog, writes a set of
   per-sub-address YAML manifest files documenting every opcode and payload shape. It then
   computes a 32-bit FNV-1a fingerprint over the manifest set. The fingerprint is embedded in
   both generated projects so a version mismatch is detected at runtime before any test runs.
   Each sub-address also carries a *per-service hash* over just its own command vocabulary,
   read at runtime via a universal command (opcode `0x00`) so a mismatch can be traced to the
   specific handler that diverged.

3. **Generate** — Two self-contained C++ projects are written to the output directory.

## Generated Output

### `exhibition_rt/` — Remote Terminal (on-target firmware)

Runs on the STM32 board. Listens on a designated UART for commands from the host.

| File / Directory | Contents |
|---|---|
| `device_registry.h/.cpp` | Generated — instantiates every peripheral handler and registers it with the RemoteTerminal scheduler |
| `api_fingerprint.h` | Generated — `constexpr uint32_t kApiFingerprint` matching the host side |
| `service_hashes.h` | Generated — `constexpr uint32_t kServiceHash<Kind>` per device kind, served by the universal `GET_SERVICE_HASH` command (opcode `0x00`) |
| `command_ids.h` | Generated — `enum class …Cmd : uint8_t` of command opcodes per device kind |
| `sub_addresses.h` | Generated — `enum class SubAddress : uint8_t` naming every assigned sub-address |
| `api/` | Generated — per-sub-address API manifest files (`NN_<name>.yaml`: sub-address, opcodes, payload shapes) |
| `INTEGRATION_GUIDE.md` | Generated — step-by-step instructions for adding the RT to an STM32CubeIDE project |
| `communication/` | Static — half-duplex protocol scheduler, message header, CRC-16 |
| `devices/` | Static — per-peripheral handlers (GPIO, ADC, DAC, PWM, SPI, UART, I2C, CAN) |
| `transports/` | Static — UART frame transport and system clock interface |
| `system/` | Static — system sub-address handler (fingerprint query, device count) |

**Integration points:**
- Call `init_rt()` once during startup (after HAL init).
- Call `tick_rt()` on every main-loop (super-loop) iteration to advance the RT state machine.

Exhibition wires both calls (plus `#include "rt/device_registry.h"`) into your
CubeMX `main.c`/`main.cpp` automatically at the end of generation, each inside
its `USER CODE` fence so CubeMX preserves it; lines already present are left
untouched. If your `main.c` isn't found next to the generated sources, or a
`USER CODE` region is missing, the generator prints the exact lines to add by
hand (see `INTEGRATION_GUIDE.md`).

**Adding your own handlers:** define `void rt_user_init(RemoteTerminal& rt)` in
your own source file, outside the generated folders. `init_rt()` calls it after
every generated handler is registered, so it can register extra
`ISubAddressHandler`s on unused sub-addresses
(`rt.register_handler(30, &my_handler, kMyServiceHash)`). The generated
registry provides an empty weak default, so projects that don't define it are
unaffected. On the host, call them with `RtClient::transact()` (below).

### `exhibition_bc/` — Bus Controller (host-side test client)

Runs on a desktop or laptop. Communicates with the RT over a serial port.

| File / Directory | Contents |
|---|---|
| `rt_client.h/.cpp` | Generated — `RtClient` class with one blocking method per peripheral operation |
| `api_fingerprint.h` | Generated — same fingerprint constant as the RT side |
| `service_hashes.h` | Generated — same per-service `kServiceHash<Kind>` constants as the RT side |
| `command_ids.h` / `sub_addresses.h` | Generated — same opcode/sub-address enums as the RT side |
| `api/` | Generated — identical per-sub-address manifest set |
| `main.cpp` | Generated — `main()` that opens the port, verifies the fingerprint, and exercises every peripheral kind. Edits inside its `USER CODE` regions survive regeneration |
| `CMakeLists.txt` / `CMakePresets.json` | Generated — portable CMake + Ninja build (Linux, macOS, Windows via MSVC or MinGW). Sources are auto-discovered with `file(GLOB)`, so files you add (including anything in `user/`) build without editing it and survive regeneration |
| `communication/` | Static — shared protocol layer (re-used from RT) |
| `host/` | Static — serial frame transport, `std::steady_clock` adapter, `rt_status` enum |
| `user/` | Yours — drop hand-written `.cpp`/`.h` here; the CMake glob picks them up automatically and regeneration never touches them |

**Typical usage:**

```cpp
SerialFrameTransport transport("/dev/ttyUSB0", 115200);
StdChronoClock clock;
RtClient client(transport, clock);

client.verify_fingerprint();          // aborts if RT/BC versions differ
client.verify_service_hashes();       // on mismatch, logs which sub-address diverged

client.gpio_PA5.set();                // drive PA5 high
uint16_t counts = client.adc_ADC1_IN0.sample();  // read ADC channel
client.uart_USART2.tx(buf, len);      // send bytes over USART2
```

For a handler you registered yourself from `rt_user_init()`, use the raw
`transact()` call. The command starts with your opcode and is a read when
`resp_cap` is non-zero. Pass `log_success = false` when polling at a high rate,
so only failures are logged:

```cpp
uint8_t cmd[2] = {0x01, 4};
uint8_t resp[4];
uint32_t got = 0;
client.transact(30, cmd, sizeof(cmd), resp, sizeof(resp), &got, /*log_success=*/false);
```

## Wire Protocol

Exhibition uses a half-duplex protocol over UART or USB.

- The **BC drives every transaction**; the RT only responds.
- Each transaction begins with a **6-byte packed header** containing: priority (2 bits),
  command/response flag (1 bit), large-frame flag (1 bit), payload-size index (3 bits),
  fault flag (1 bit), sub-address (8 bits), payload CRC-16 (16 bits), and header CRC-16
  (16 bits).
- Valid payload sizes are sparse: 0, 2, 4, 6, 8, 10, 12, 14, 16, 32, 64, 128, 256, 512,
  1024, and 2048 bytes.
- Commands with a payload use a four-phase handshake (cmd header → RT ack → payload →
  RT ack/response). Zero-payload commands use a two-phase flow.

The `api_fingerprint.h` constant is identical on both sides. If the BC reads a different
fingerprint from the RT, it reports a version mismatch before any peripheral commands are
sent. To localize a mismatch, every sub-address answers a universal `GET_SERVICE_HASH`
command (opcode `0x00`) returning a per-service hash over just its command vocabulary; the
`kServiceHash<Kind>` constants in `service_hashes.h` are identical on both sides, and
`verify_service_hashes()` reports exactly which handler(s) differ.

## Supported Peripherals

| Peripheral | Operations |
|---|---|
| GPIO Output | SET, CLEAR |
| GPIO Input | READ |
| ADC | SAMPLE |
| DAC | SET_COUNTS |
| PWM | SET_DUTY |
| SPI | TX, RX |
| UART | TX, RX |
| I2C | WRITE_REG, READ_REG |
| CAN / FDCAN | TX_FRAME, RX_FRAME |

## Prerequisites

| Requirement | Purpose |
|---|---|
| Rust toolchain (stable) | Build Exhibition itself |
| [STM32CubeMX](https://www.st.com/en/development-tools/stm32cubemx.html) | Produce the `.ioc` file and generate HAL driver code |
| STM32CubeIDE or ARM GCC toolchain | Build and flash `exhibition_rt/` |
| CMake (≥ 3.15), Ninja, and a C++17 compiler (GCC, Clang, or MSVC) | Build `exhibition_bc/` and the C++ unit tests |

## How to Build

```
cargo build --release
```

## How to Test Rust

```
cargo test
```

## How to Test C++

The C++ unit tests (Catch2 v3) cover the generated communication, scheduler,
transport, and device-handler code. Build and run every test binary with:

```
cd src/engine/autogen_content/cpp/tests
cmake --preset Debug
cmake --build --preset Debug
ctest --preset Debug
```

`cmake --build --preset Debug` alone builds the binaries without running them.
This builds the same way on Linux, macOS, and Windows.

## How to Use

1. Build Exhibition (`cargo build --release`).
1. Use STM32CubeMX to configure your board and generate HAL driver code; note the path to
   the `.ioc` file.
1. Run Exhibition (`./target/release/exhibition`).
1. When prompted, provide the path to the `.ioc` file.
1. When prompted, select which UART or USB CDC interface will serve as the RT control bus
   (Exhibition lists all UARTs that have both TX and RX configured, and any USB CDC
   device interfaces).
1. Optionally generate the BC project when prompted.
1. Review any errors reported by Exhibition.
1. Open `exhibition_rt/INTEGRATION_GUIDE.md` for step-by-step instructions on adding the
   generated files to your STM32CubeIDE project.
1. Build and flash the RT firmware to your board.
1. Build the BC project (`cmake --preset Debug && cmake --build --preset Debug` inside
    `exhibition_bc/`) and run `exhibition_bc` to verify the connection and exercise the API.
1. Write your own test sequences in the BC project, using `rt_client.h` as the interface.
