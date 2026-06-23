// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SUB_ADDRESSES_H
#define SUB_ADDRESSES_H

#include <stdint.h>

/// @file sub_addresses.h
/// @brief Named sub-addresses assigned for this project.
/// @details Auto-generated from the device set. `System` is always 1;
///          peripheral sub-addresses (2..31) are assigned deterministically.

enum class SubAddress : uint8_t {
    System = 1,
    AdcHadc1Ch1 = 2,
    CanHcan1 = 3,
    GpioInBankB = 4,
    GpioInBankD = 5,
    GpioOutBankC = 6,
    GpioOutBankD = 7,
    GpioOutBankE = 8,
    I2cHi2c1 = 9,
    Rtc = 10,
    SpiHspi1 = 11,
};

#endif // SUB_ADDRESSES_H
