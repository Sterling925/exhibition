// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef USER_RTC_H
#define USER_RTC_H

#include <stdint.h>

#include "rt_client.h"


/// @brief A calendar date and wall-clock time held by the RTC.
/// @details Field ranges and encodings match the RT-side STM32 HAL RTC:
///          @c year_offset is years since 2000, @c weekday is
///          1=Monday .. 7=Sunday, and @c hours uses the 24-hour clock.
struct RtcDateTime
{
    uint8_t year_offset = 0u;  ///< Years since 2000 (0..99).
    uint8_t month       = 1u;  ///< Month (1..12).
    uint8_t day         = 1u;  ///< Day of month (1..31).
    uint8_t weekday     = 1u;  ///< Day of week (1=Monday .. 7=Sunday).
    uint8_t hours       = 0u;  ///< Hour, 24-hour clock (0..23).
    uint8_t minutes     = 0u;  ///< Minute (0..59).
    uint8_t seconds     = 0u;  ///< Second (0..59).
};


/// @brief Fill an RtcDateTime from the host PC's current local time.
/// @details Reads the host wall clock and converts the C @c tm fields into
///          the RTC encoding (years since 2000, weekday 1=Monday..7=Sunday).
/// @param[out] dt : Struct populated from the host clock.
void rtc_now_local(RtcDateTime& dt);


/// @brief Write a date and time into the RTC.
/// @param client : Client to communicate with the RTC.
/// @param dt : Date and time to program.
/// @return Status::Ok on success, or the first non-Ok Status encountered.
Status rtc_set(RtClient& client, const RtcDateTime& dt);


/// @brief Read the current date and time from the RTC and print it.
/// @note The date/time is printed to std::out; pass a non-null out-pointer to
///       also receive it.
/// @param client : Client to communicate with the RTC.
/// @param[out] dt_out : If non-null, receives the value read.
/// @return Status::Ok on success, or the first non-Ok Status encountered.
Status rtc_read(RtClient& client, RtcDateTime* dt_out = nullptr);

#endif // USER_RTC_H