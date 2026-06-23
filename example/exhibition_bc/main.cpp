// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file main.cpp
/// @brief Generated starting point for a BC test sequence.
/// @details
/// Opens the serial port to the RT, verifies that the BC and RT
/// were built from the same .ioc (fingerprint check), and runs one
/// representative call per peripheral kind. Edit the USER CODE
/// regions to script your real test sequences; everything you put
/// between a `USER CODE BEGIN`/`END` pair is preserved when the
/// project is regenerated. Note: because the example calls live
/// inside the `Sequence` region, peripherals added to the .ioc
/// later will not be auto-inserted once you have edited it.

#include <cstdio>
#include <cstdlib>

#include "rt_client.h"
#include "host/serial_frame_transport.h"
#include "host/logging_frame_transport.h"
#include "host/logger.h"
#include "host/std_steady_clock.h"
#include "host/delay.h"

/* USER CODE BEGIN Includes */

#include "user/leds.h"
#include "user/mems.h"
#include "user/rtc.h"
#include "user/systick_sync.h"

/* USER CODE END Includes */

// NOTE: the .ioc file did not specify a baud rate for the chosen UART.
//       The value below is a best-guess default — pass the actual
//       baud as argv[2] if this does not match the firmware.
static constexpr uint32_t kDefaultBaud = 115200u;

/* USER CODE BEGIN Decls */

/* USER CODE END Decls */

int main(int argc, char** argv)
{
    if (argc < 2) {
        std::fprintf(stderr,
            "Usage: %s <serial-device> [<baud>]\n"
            "  Linux/macOS: %s /dev/ttyUSB0 115200\n"
            "  Windows:     %s COM7 115200\n"
            "  (NOTE: baud above is a fallback; .ioc did not specify one)\n",
            argv[0], argv[0], argv[0]);
        return 1;
    }
    char const* dev = argv[1];
    uint32_t baud = kDefaultBaud;
    if (argc >= 3) {
        baud = static_cast<uint32_t>(std::atoi(argv[2]));
    }

    // Construct the logger first so even an open() failure is recorded.
    Logger logger(Logger::default_log_path());
    logger.line("INFO", "exhibition_bc starting");
    std::printf("Logging to %s\n", logger.path().c_str());

    SerialFrameTransport serial;
    if (!serial.open(dev, baud)) {
        std::fprintf(stderr, "Failed to open %s at %u baud\n", dev, baud);
        logger.line("ERROR", "failed to open serial port");
        return 2;
    }

    // Decorate the real transport so all wire traffic is logged.
    LoggingFrameTransport transport(serial, logger, &op_name, &sub_address_name);
    StdSteadyClock clock;
    RtClient client(transport, clock, logger);

    /* USER CODE BEGIN Init */
    // Reset to get systick closer to local zero point.
    client.soft_reset();
    // Wait for RT to reboot
    delay(1000);
    /* USER CODE END Init */

    const Status fps = client.verify_fingerprint();
    if (fps != Status::Ok) {
        std::fprintf(stderr,
            "Fingerprint check failed: %s (expected 0x%08X)\n",
            status_to_string(fps),
            static_cast<unsigned>(kApiFingerprint));
        // Localize the divergence: per-service hashes pinpoint which
        // sub-address(es) differ (each mismatch is logged as ERROR).
        client.verify_service_hashes();
        return 3;
    }
    std::printf("Fingerprint OK (0x%08X)\n",
                static_cast<unsigned>(kApiFingerprint));

    uint16_t count = 0;
    if (client.get_device_count(count) == Status::Ok) {
        std::printf("RT reports %u registered devices.\n", count);
    }

    // PTP-style time sync: run the exchange a few times to let the RT's
    // offset filter settle. The offset is the RT systick relative to this
    // host's clock; add it to an RT timestamp to express it here.
    // Note 32 is the smoother window size.
    for (int i = 0; i < 32; ++i) {
        SyncMeasurement m;
        if (client.sync_once(&m) != Status::Ok) {
            break;
        }
        delay(50);
    }
    if (client.has_sync()) {
        std::printf("Time sync: offset = %.3f, path delay = %.3f (units 1e-%u s), "
                    "systick reload = %u\n",
                    client.clock_offset_units(), client.path_delay_units(),
                    static_cast<unsigned>(client.sync_units()),
                    static_cast<unsigned>(client.reload_ticks()));
    }

    /* USER CODE BEGIN Sequence */
    MemsConfig mems_cfg;
    if( mems_identify(client, mems_cfg)  != Status::Ok) 
    {
        printf("mems ident failed\n");
        return 1;
    }
    else if( mems_init(client, mems_cfg)  != Status::Ok)
    {
        printf("mems init failed\n");
        return 1;
    }

    // Set the RTC once from the host PC's local time.
    RtcDateTime rtc_now;
    rtc_now_local(rtc_now);
    if (rtc_set(client, rtc_now) != Status::Ok)
    {
        printf("rtc set failed\n");
        return 1;
    }


    SyncHelper sync_helper;
    if(sync_helper.initial_sync( client ) != Status::Ok )
    {
        return 0;
    }
    
    for(uint32_t i = 0; i< 1000; ++i)
    {
     
        if(sync_helper.single_sync(client, clock, logger) != Status::Ok )
        {
            break;
        }
        delay(500);

        if( mems_update(client, mems_cfg)  != Status::Ok)
        {
            break;
        }

        // ADC MyAdc (adc_hadc1_ch1): single sample.
        {
            uint16_t adc_counts = 0;
            uint32_t adc_timestamp_ms = 0;
            const uint32_t before = clock.now_ms();
            if (client.adc_hadc1_ch1.sample(adc_counts, adc_timestamp_ms) == Status::Ok) {
                const uint32_t after = clock.now_ms();
                printf("About to read ADC  @ %u ms\n", before);
                printf("MyAdc = %u counts @ %u ms\n", adc_counts, adc_timestamp_ms);
                printf("Just read ADC      @ %u ms\n", after);
            }
        }


        // RTC (rtc): read the current date and time.
        rtc_read(client, nullptr);

        uint32_t remote_faults = 0;
        client.get_message_fault_counter(remote_faults);
        uint32_t local_faults = client.bc_fault_count();
        printf("faults: local= %u, remote= %u\n",local_faults, remote_faults);

        leds_do_loop(client, 50);
        

    }
    /* USER CODE END Sequence */

    /* USER CODE BEGIN PreReturn */
    
    /* USER CODE END PreReturn */

    return 0;
}
