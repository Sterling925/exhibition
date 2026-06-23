// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file main.h
/// @brief Minimal HAL stub for host-based device handler unit tests.
/// @details
/// Replaces the STM32 CubeMX main.h / HAL headers when compiling device
/// handler sources for the host test build. Test files define the globals
/// declared here (g_hal_adc_*) and set them before each test case to inject
/// the desired HAL behavior without touching real hardware.

#ifndef MAIN_H
#define MAIN_H

#include <stdint.h>

// ---------------------------------------------------------------------------
// ADC peripheral types and stubs
// ---------------------------------------------------------------------------

typedef struct { int dummy; } ADC_HandleTypeDef;

typedef enum {
    HAL_OK      = 0,
    HAL_ERROR   = 1,
    HAL_BUSY    = 2,
    HAL_TIMEOUT = 3
} HAL_StatusTypeDef;

/// Configurable return value for HAL_ADC_Start() — set in each test case.
extern HAL_StatusTypeDef g_hal_adc_start_result;

/// Configurable return value for HAL_ADC_PollForConversion() — set in each test case.
extern HAL_StatusTypeDef g_hal_adc_poll_result;

/// Configurable value returned by HAL_ADC_GetValue() — set in each test case.
extern uint32_t g_hal_adc_value;

inline HAL_StatusTypeDef HAL_ADC_Start(ADC_HandleTypeDef*)
{
    return g_hal_adc_start_result;
}

inline HAL_StatusTypeDef HAL_ADC_PollForConversion(ADC_HandleTypeDef*, uint32_t)
{
    return g_hal_adc_poll_result;
}

inline uint32_t HAL_ADC_GetValue(ADC_HandleTypeDef*)
{
    return g_hal_adc_value;
}

inline HAL_StatusTypeDef HAL_ADC_Stop(ADC_HandleTypeDef*)
{
    return HAL_OK;
}

// ---------------------------------------------------------------------------
// Core / CPU identity stubs
// ---------------------------------------------------------------------------

/// The system handler references these for its GET_CORE_DEVICE_ID / GET_CPU_ID
/// opcodes. Constant values suffice for the host tests, which do not exercise
/// those opcodes (the sync end-to-end test only drives SYNC_PROBE / SYNC_FINAL).
inline uint32_t HAL_GetDEVID() { return 0u; }
inline uint32_t HAL_GetREVID() { return 0u; }
inline uint32_t HAL_GetUIDw0() { return 0u; }
inline uint32_t HAL_GetUIDw1() { return 0u; }
inline uint32_t HAL_GetUIDw2() { return 0u; }

#endif // MAIN_H
