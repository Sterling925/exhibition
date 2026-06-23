// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include <cstdio>
#include <ctime>

#include "rtc.h"


void rtc_now_local(RtcDateTime& dt)
{
    const std::time_t now = std::time(nullptr);
    std::tm local{};
#if defined(_WIN32)
    localtime_s(&local, &now);
#else
    localtime_r(&now, &local);
#endif

    // tm_year is years since 1900; the RTC stores years since 2000.
    const int year_2000 = local.tm_year - 100;
    dt.year_offset = static_cast<uint8_t>((year_2000 < 0) ? 0 : year_2000);
    dt.month       = static_cast<uint8_t>(local.tm_mon + 1);   // tm_mon is 0..11
    dt.day         = static_cast<uint8_t>(local.tm_mday);
    // tm_wday is 0=Sunday..6=Saturday; the RTC wants 1=Monday..7=Sunday.
    dt.weekday     = static_cast<uint8_t>((local.tm_wday == 0) ? 7 : local.tm_wday);
    dt.hours       = static_cast<uint8_t>(local.tm_hour);
    dt.minutes     = static_cast<uint8_t>(local.tm_min);
    dt.seconds     = static_cast<uint8_t>(local.tm_sec);
}


Status rtc_set(RtClient& client, const RtcDateTime& dt)
{
    const Status s = client.rtc.set_datetime(dt.year_offset, dt.month, dt.day,
                                             dt.weekday, dt.hours, dt.minutes,
                                             dt.seconds);
    if (s != Status::Ok) {
        std::fprintf(stderr, "RTC: set_datetime failed\n");
    }
    return s;
}


Status rtc_read(RtClient& client, RtcDateTime* dt_out)
{
    RtcDateTime dt{};
    const Status s = client.rtc.get_datetime(dt.year_offset, dt.month, dt.day,
                                             dt.weekday, dt.hours, dt.minutes,
                                             dt.seconds);
    if (s != Status::Ok) {
        std::fprintf(stderr, "RTC: get_datetime failed\n");
        return s;
    }

    std::printf("RTC 20%02u-%02u-%02u %02u:%02u:%02u (weekday %u)\n",
                dt.year_offset, dt.month, dt.day,
                dt.hours, dt.minutes, dt.seconds, dt.weekday);

    if (dt_out != nullptr) {
        *dt_out = dt;
    }
    return s;
}
