// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef CRC_H
#define CRC_H

#include <stdint.h>

/// @brief Calculate a CRC16 of array
/// @note Uses lookup table to improve performance
/// @param data Data to calculate CRC16 over
/// @param length Length of data to calculate CRC16 on
/// @return CRC16 value
/// @details
/// Calculates CRC16 using lookup table with formula 
/// [CRC-16/MODBUS](https://modbustools.com/modbus_crc16.html)
/// Polynomial=0xA001	InitalValue=0xFFFF
uint16_t calc_crc16_lt(uint8_t const*const data, const uint32_t length);



/// @brief Calculate a CRC16 of array
/// @note Non lookup table to consume less memory
/// @param data Data to calculate CRC16 over
/// @param length Length of data to calculate CRC16 on
/// @return CRC16 value
/// @details
/// Calculates CRC16 with formula 
/// [CRC-16/MODBUS](https://github.com/LacobusVentura/MODBUS-CRC16)
/// [or CRC-16/MODBUS](https://ctlsys.com/support/how_to_compute_the_modbus_rtu_message_crc/)
/// Polynomial=0xA001	InitalValue=0xFFFF
uint16_t calc_crc16(uint8_t const*const data, const uint32_t length);

#endif // CRC_H