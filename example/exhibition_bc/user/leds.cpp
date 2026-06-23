// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "leds.h"
#include "host/delay.h"


Status leds_do_loop(RtClient&client, const uint32_t d)
{
        Status ans = Status::Ok;

        ans = client.gpio_PD12.set();
        if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD12.clear();
                if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD13.set();
                if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD13.clear();
        if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD14.set();
        if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD14.clear();
        if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD15.set();
        if(Status::Ok != ans)
        {
                return ans;
        }
        delay(d);
        ans = client.gpio_PD15.clear();
        delay(d);

        return ans;
}
