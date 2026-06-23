// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef USER_LEDS_H
#define USER_LEDS_H

#include <stdint.h>

#include "rt_client.h"



/// @brief Flash the LED's in a circle
/// @param client : The client to send the commands to
/// @param d : A delay in milliseconds between each LED flash
/// @return Ok on success
Status leds_do_loop(RtClient&client, const uint32_t d);


#endif // USER_LEDS_H