// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef USER_MEMS_H
#define USER_MEMS_H

#include <stdint.h>

#include "rt_client.h"


/// @brief Configuration data for mems chip
struct MemsConfig
{
    // Per-chip configuration selected from the detected WHO_AM_I value.
    char const* chip_name = nullptr;  // non-null => recognized chip
    uint8_t     ctrl_reg  = 0x20u;    // control register is 0x20 on all three
    uint8_t     ctrl_val  = 0u;       // power-up + enable X/Y/Z
    bool        data_ms   = false;    // set MS bit (0x40) in the read address?
    bool        data_16b  = false;    // true: 16-bit LE per axis; false: 8-bit
};


/// @brief Identify chip model and read config
/// @param client : Client to communicate with chip
/// @param[out] cfg : Config struct to fill with data from chip
/// @return Status::Ok on success, or the first non-Ok Status encountered.
Status mems_identify(RtClient&client, MemsConfig& cfg);


/// @brief Initialize the chip once the chip config is known
/// @param client Client to communicate with chip
/// @param cfg : Config for given chip model
/// @return Status::Ok on success, or the first non-Ok Status encountered.
Status mems_init(RtClient&client, const MemsConfig &cfg);


/// @brief Read accelerometer, print output to std::out, and optionally return it
/// @note The X/Y/Z readings are still printed; pass non-null out-pointers to
///       also receive them. For 8-bit parts the values are sign-extended into
///       the 16-bit out-parameters.
/// @param client Client to communicate with chip
/// @param cfg : Config for given chip model
/// @param[out] x_out : If non-null, receives the X-axis reading.
/// @param[out] y_out : If non-null, receives the Y-axis reading.
/// @param[out] z_out : If non-null, receives the Z-axis reading.
/// @return Status::Ok on success, or the first non-Ok Status encountered.
Status mems_update(RtClient&client, const MemsConfig & cfg,
                   int16_t* x_out = nullptr,
                   int16_t* y_out = nullptr,
                   int16_t* z_out = nullptr);

#endif // USER_MEMS_H