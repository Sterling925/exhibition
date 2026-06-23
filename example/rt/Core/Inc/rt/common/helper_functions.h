// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef HELPER_FUNCTIONS_H
#define HELPER_FUNCTIONS_H

#include <stdint.h>

/// @brief Emplace a 16-bit value little-endian into @p dst.
/// @warning No bounds checking. Developer must ensure correct indexing.
/// @param dst Destination buffer; must have room for 2 bytes.
/// @param value Value to serialize.
void emplace_u16_le(uint8_t*const dst, const uint16_t value);


/// @brief Emplace a 32-bit value little-endian into @p dst.
/// @warning No bounds checking. Developer must ensure correct indexing.
/// @param dst Destination buffer; must have room for 4 bytes.
/// @param value Value to serialize.
void emplace_u32_le(uint8_t*const dst, const uint32_t value);


/// @brief Extract a uint16_t packed little-endian from an array @p src
/// @warning No bounds checking. Developer must ensure correct indexing.
/// @param src The array location to extract a value
/// @return uint16_t value from little-endian packed bytes
uint16_t extract_u16_le(uint8_t const*const src);


/// @brief Extract a uint32_t packed little-endian from an array @p src
/// @warning No bounds checking. Developer must ensure correct indexing.
/// @param src The array location to extract a value
/// @return uint32_t value from little-endian packed bytes
uint32_t extract_u32_le(uint8_t const*const src);




/// @brief Calculate an absolute minimum timeout for a bus transaction
/// @param baud_rate : Baud rate of bus in question
/// @param byte_qty : How many bytes are being sent/received during the timeout duration
/// @param bits_per_byte : How many bits are put on the wire per byte of actual data
/// @return duration in milliseconds of transmission with a little padding
/// @details
/// Currently assumes UART with stop bits on each byte.
/// Does not account for gaps between chunks such as between tx and rx operations.
///
/// If there are start/stop/parity bits per frame (bits_per_byte), use:
/// Duration (ms) = (bytes × bits_per_byte) / baud × 1000
/// where bits_per_byte = 1 start + data_bits + parity_bit + stop_bits (e.g., 1+8+0+1 = 10).
constexpr uint32_t calc_timeout_min(const uint32_t baud_rate, 
  const uint32_t byte_qty, 
  const uint32_t bits_per_byte) 
{
    const uint32_t padding_and_rounding_ms = 2;
    uint32_t milliseconds = padding_and_rounding_ms + (1000 * (byte_qty * bits_per_byte)) / baud_rate;
    return milliseconds;
}

#endif // HELPER_FUNCTIONS_H