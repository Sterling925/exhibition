// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "rt/common/helper_functions.h"


void emplace_u16_le(uint8_t*const dst, const uint16_t value)
{
    dst[0] = static_cast<uint8_t>( value        & 0xFFu);
    dst[1] = static_cast<uint8_t>((value >> 8)  & 0xFFu);
}


void emplace_u32_le(uint8_t*const dst, const uint32_t value)
{
    dst[0] = static_cast<uint8_t>( value        & 0xFFu);
    dst[1] = static_cast<uint8_t>((value >> 8)  & 0xFFu);
    dst[2] = static_cast<uint8_t>((value >> 16) & 0xFFu);
    dst[3] = static_cast<uint8_t>((value >> 24) & 0xFFu);
}


uint16_t extract_u16_le(uint8_t const*const src)
{

    uint16_t ans = static_cast<uint16_t>(src[0]);
    ans |= (static_cast<uint16_t>(src[1]) << 8);

    return ans;
}


uint32_t extract_u32_le(uint8_t const*const src)
{
    uint32_t ans = src[0];
    ans |= (static_cast<uint32_t>(src[1]) << 8);
    ans |= (static_cast<uint32_t>(src[2]) << 16);
    ans |= (static_cast<uint32_t>(src[3]) << 24);

    return ans;
}


