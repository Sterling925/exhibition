<!--
SPDX-License-Identifier: MIT OR Apache-2.0
SPDX-FileCopyrightText: 2026 https://github.com/Sterling925
-->

# Integrating `exhibition_rt` into an STM32CubeIDE project

This guide was auto-generated alongside the rest of `exhibition_rt/` and references the actual transport and devices you selected.

## 1. What this is

The Remote Terminal (RT) is a single-threaded, polling-based slave that listens on **USART2** (HAL handle `huart2`) and dispatches incoming commands to the device handlers listed below. There is no RTOS dependency: the RT advances one step per call to `tick_rt()` and returns immediately if there is nothing to do.

API fingerprint for this build: `0xA084EED3`. The BC can read this back over the wire via the system handler to confirm both sides agree on the device layout.

Every sub-address (including the system handler) also answers a universal command, opcode `0x00` (`GET_SERVICE_HASH`), returning a per-service API hash over just that service's command vocabulary. When the global fingerprint check fails, the BC's `verify_service_hashes()` reads each sub-address's hash to pinpoint which device handler diverged.

## 2. Devices registered

| Sub-address | Kind | Instance | Label |
|------------:|------|----------|-------|
| 1 | System | `g_system_handler` | reserved (fingerprint / device count) |
| 2 | ADC | `g_adc_hadc1_ch1` | MyAdc |
| 3 | CAN | `g_can_hcan1` | CAN1 |
| 4 | GPIO input | `g_gpio_in_bank_B` | GPIO input bank B |
| 5 | GPIO input | `g_gpio_in_bank_D` | GPIO input bank D |
| 6 | GPIO output | `g_gpio_out_bank_C` | GPIO output bank C |
| 7 | GPIO output | `g_gpio_out_bank_D` | GPIO output bank D |
| 8 | GPIO output | `g_gpio_out_bank_E` | GPIO output bank E |
| 9 | I2C | `g_i2c_hi2c1` | I2C1 |
| 10 | RTC | `g_rtc` | RTC |
| 11 | SPI | `g_spi_hspi1` | SPI1 |

## 3. STM32CubeMX prerequisites

Before integrating, confirm the following in your `.ioc` (open in STM32CubeMX and regenerate code if you change anything):

- **USART2** is enabled and `MX_USART2_UART_Init()` (or `MX_USART2_Init()`, depending on your HAL version) is called from the generated `main` before the main loop. Polling mode is sufficient — no global interrupt is required.
- **SysTick** is running so `HAL_GetTick()` returns a monotonically-increasing millisecond count. This is the default for CubeMX projects; do not disable it.
- Every peripheral listed in section 2 above is enabled in the `.ioc` and the CubeMX-generated HAL handle (`hadc1`, `htim2`, etc.) matches the name baked into `device_registry.cpp`. If you rename a peripheral, re-run `exhibition` against the updated `.ioc`.

## 4. Pick up the new files in STM32CubeIDE

Generated headers live under `Core/Inc/rt/` and generated implementations live under `Core/Src/rt/`. Both locations are already covered by a CubeMX-generated project's default include path and source folders, so no *Paths and Symbols* configuration is required.

1. Right-click the project in the *Project Explorer* and choose *Refresh* (or press **F5**) so the indexer notices the new files.
2. *Project → Clean...* and then rebuild. CubeIDE should compile every `Core/Src/rt/**/*.cpp` automatically.

Every internal `#include` between the generated files uses the form `"rt/<area>/<file>.h"`, resolved through the default `Core/Inc` include path.

**C / C++ note:** the RT is C++. STM32CubeMX defaults to C (`main.c`). If your project has not been switched to C++ yet, either rename `main.c` to `main.cpp` (and right-click the project → *Convert to C++*) or keep `main.c` and include `"rt/device_registry.h"` from C — the entry points are already declared with C linkage on the RT side via `device_registry.h`.

**C++ standard:** build the project as **C++17** (or later). The generated RT uses `if constexpr`, a C++17 feature; under an older standard the compiler emits a `-Wc++17-extensions` warning and rejects it outright with `-pedantic-errors`. In STM32CubeIDE this is *Project → Properties → C/C++ Build → Settings → Tool Settings → MCU G++ Compiler → General → Language standard* → **GNU C++17 (`-std=gnu++17`)** (or ISO C++17); *Apply and Close* and rebuild.

## 5. Wire `init_rt()` and `tick_rt()` into `main.cpp`

Exhibition normally does this for you: at the end of generation it inserts the three lines below into your `main.c` (or `main.cpp`), each inside its *USER CODE* fence, and skips any that are already present. The steps here are the manual fallback for when your `main.c` lives somewhere exhibition did not find it, or a USER CODE region is missing. The lines belong inside the *USER CODE* fences so they survive CubeMX regeneration:

```cpp
/* USER CODE BEGIN Includes */
#include "rt/device_registry.h"
/* USER CODE END Includes */

int main(void)
{
    HAL_Init();
    SystemClock_Config();
    MX_GPIO_Init();
    MX_USART2_UART_Init();   // and the other MX_*_Init() calls CubeMX generated
    /* ...other MX_*_Init() calls... */

    /* USER CODE BEGIN 2 */
    init_rt();
    /* USER CODE END 2 */

    while (1)
    {
        /* USER CODE BEGIN 3 */
        tick_rt();
        /* USER CODE END 3 */
    }
}
```

Call `tick_rt()` as often as you can afford. It is non-blocking and returns immediately when the transport has no pending bytes; long stretches between calls translate directly into request latency on the bus.

## 6. Verify the integration end-to-end

Your generated devices include a GPIO output (`OTG_FS_PowerSwitchOn`, on bank sub-address 6). Use it as the smoke test:

1. Build and flash the firmware.
2. On the host side, build the BC project that was generated in the sibling `exhibition_bc/` folder (`cmake --preset Debug && cmake --build --preset Debug` in that directory).
3. Wire **USART2** (TX/RX/GND) between the board and the host serial adapter, then run `./exhibition_bc /dev/ttyUSB0 115200` (Linux) or `./exhibition_bc.exe COM7 115200` (Windows)

> **Note:** Your `.ioc` did not specify a baud rate for the chosen UART; the value above is a fallback. Override it from the BC command line if your firmware uses a different baud.
4. The relevant lines in `main.cpp` are:

```cpp
client.gpio_PC0.set();    // drives OTG_FS_PowerSwitchOn high
client.gpio_PC0.clear();  // drives OTG_FS_PowerSwitchOn low
```

5. Confirm the LED tied to **OTG_FS_PowerSwitchOn** toggles. If it does, the wire protocol, transport, scheduler, and your `tick_rt()` call site are all correct.

If the BC reports timeouts instead of `Status::Ok`, the most common causes are: `init_rt()` not being called (no handler registered), `tick_rt()` not being called from the main loop (no scheduler progress), or the host serial adapter being connected to the wrong UART.

## 7. Where to learn more

See [`communication/README.md`](communication/README.md) (shipped in this same folder) for the wire protocol details: message-header layout, CRC-16 (MODBUS) coverage, the `RemoteTerminal` / `BusController` APIs, and the configuration constants you can tune for your platform.
