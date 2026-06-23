// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include <cstdio>

#include "mems.h"
#include "host/delay.h"




/// @brief Chip-select pin for the on-board MEMS accelerometer (PE3).
/// @details On the STM32F407VG Discovery board PE3 is the LIS302DL/LIS3DSH/
///          LIS3DH CS line; driving it low also selects the part's SPI mode.

/// @brief Read @p n register bytes from the MEMS accelerometer over SPI1.
/// @details Asserts CS (PE3 low), clocks a single address byte, then clocks
///          @p n dummy bytes to capture the register data, and finally
///          de-asserts CS. The whole access happens with CS held low so the
///          split TX (address) / RX (data) pair is one transaction on the
///          wire. The read bit (0x80) is always set; the multiple-byte
///          auto-increment (MS) bit (0x40) is set only when @p ms is true,
///          because the three candidate parts disagree on whether bit 6 is
///          an address bit (LIS3DSH) or the MS bit (LIS302DL/LIS3DH).
/// @param client RT client bound to gpio_PE3 and spi_hspi1.
/// @param reg    6-bit register address (0x00..0x3F).
/// @param n      Number of data bytes to read into @p out.
/// @param out    Caller-owned buffer of at least @p n bytes.
/// @param ms     Set the MS auto-increment bit (bit 6) in the address byte.
/// @return Status::Ok on success, or the first non-Ok Status encountered.
static Status mems_read(RtClient& client, uint8_t reg, uint8_t n,
                        uint8_t* out, bool ms)
{
    const uint8_t addr = static_cast<uint8_t>(0x80u | (ms ? 0x40u : 0x00u) | (reg & 0x3Fu));

    Status s = client.gpio_PE3.clear();          // assert CS
    if (s != Status::Ok) {
        return s;
    }
    delay(1);
    s = client.spi_hspi1.tx(&addr, 1u);          // address phase (1 byte)
    if (s == Status::Ok) {
        s = client.spi_hspi1.rx(n, out);         // data phase (n bytes)
    }
    const Status cs = client.gpio_PE3.set();     // de-assert CS regardless
    return (s != Status::Ok) ? s : cs;
}

/// @brief Write one register byte to the MEMS accelerometer over SPI1.
/// @details Asserts CS (PE3 low), clocks the address byte (write bit clear,
///          bits 6:0 = register) followed by the value, then de-asserts CS.
/// @param client RT client bound to gpio_PE3 and spi_hspi1.
/// @param reg    6-bit register address (0x00..0x3F).
/// @param val    Value to write.
/// @return Status::Ok on success, or the first non-Ok Status encountered.
static Status mems_write(RtClient& client, uint8_t reg, uint8_t val)
{
    const uint8_t cmd[2] = { static_cast<uint8_t>(reg & 0x3Fu), val };

    Status s = client.gpio_PE3.clear();          // assert CS
    if (s != Status::Ok) {
        return s;
    }
    delay(1);
    s = client.spi_hspi1.tx(cmd, sizeof(cmd));
    const Status cs = client.gpio_PE3.set();     // de-assert CS regardless
    return (s != Status::Ok) ? s : cs;
}


Status mems_identify(RtClient&client, MemsConfig& cfg)
{
  
    // --- MEMS accelerometer: identify the chip via WHO_AM_I (register 0x0F).
    // The STM32F407VG Discovery board has shipped with three different parts
    // depending on board revision; they share the WHO_AM_I register but use
    // different control registers, data layouts, and address-byte semantics.
    // Auto-detect rather than hard-coding one of them. WHO_AM_I is a single
    // register read, so the MS auto-increment bit is irrelevant here.
    uint8_t who = 0u;
    Status s = mems_read(client, 0x0Fu, 1u, &who, /*ms=*/false);
    if (s != Status::Ok) {
        std::fprintf(stderr, "MEMS: WHO_AM_I read failed (transport/RT error)\n");
        return s;
    }
    std::printf("Accel  WHO_AM_I = 0x%02X\n", who);


    switch (who) {
        case 0x3Bu:  // LIS302DL: 8-bit data at 0x29/0x2B/0x2D, MS bit required.
            cfg.chip_name = "LIS302DL";
            cfg.ctrl_val  = 0x47u;        // CTRL_REG1: PD=1 (active) + Zen/Yen/Xen
            cfg.data_ms   = true;
            cfg.data_16b  = false;
            break;
        case 0x3Fu:  // LIS3DSH: 16-bit LE at 0x28..0x2D, bit 6 is an address bit.
            cfg.chip_name = "LIS3DSH";
            cfg.ctrl_val  = 0x67u;        // CTRL_REG4: ODR=100Hz + Zen/Yen/Xen
            cfg.data_ms   = false;
            cfg.data_16b  = true;
            break;
        case 0x33u:  // LIS3DH: 16-bit LE at 0x28..0x2D, MS bit required.
            cfg.chip_name = "LIS3DH";
            cfg.ctrl_val  = 0x57u;        // CTRL_REG1: ODR=100Hz + Zen/Yen/Xen
            cfg.data_ms   = true;
            cfg.data_16b  = true;
            break;
        default:
            break;
    }

    if (cfg.chip_name == nullptr) {
        std::fprintf(stderr,
            "MEMS: unrecognized WHO_AM_I (0x%02X). The host SPI sequence is now\n"
            "      correct (single-byte address phase, CS held across TX+RX), so a\n"
            "      persistent misread points to the RT side. Check that SPI1 is\n"
            "      configured for Mode 3 (CPOL=1, CPHA=1), 8-bit frames, MSB-first,\n"
            "      baud < ~10 MHz, and that MISO is wired to the sensor.\n",
            who);
        return Status::HandlerError;
    }
    std::printf("Accel  detected %s\n", cfg.chip_name);
    return Status::Ok;
}


Status mems_init(RtClient&client, const MemsConfig &cfg)
{
    // Enable the accelerometer (power up + enable all three axes).
    Status s = mems_write(client, cfg.ctrl_reg, cfg.ctrl_val);
    if (s != Status::Ok) {
        std::fprintf(stderr, "MEMS: enable write failed\n");
        return s;
    }
    return Status::Ok;
}

Status mems_update(RtClient&client, const MemsConfig & cfg,
                   int16_t* x_out, int16_t* y_out, int16_t* z_out)
{
        uint8_t buf[6] = {0};
        Status s = mems_read(client, 0x28u, 6u, buf, cfg.data_ms);
        if (s != Status::Ok) {
            std::fprintf(stderr, "MEMS: data read failed\n");
            return s;
        }

        int16_t x = 0;
        int16_t y = 0;
        int16_t z = 0;
        if (cfg.data_16b) {
            // 16-bit little-endian per axis: X_L, X_H, Y_L, Y_H, Z_L, Z_H.
            x = static_cast<int16_t>(buf[0] | (buf[1] << 8));
            y = static_cast<int16_t>(buf[2] | (buf[3] << 8));
            z = static_cast<int16_t>(buf[4] | (buf[5] << 8));
        } else {
            // 8-bit data on the odd registers (0x29, 0x2B, 0x2D).
            x = static_cast<int8_t>(buf[1]);
            y = static_cast<int8_t>(buf[3]);
            z = static_cast<int8_t>(buf[5]);
        }
        std::printf("Accel  X=%6d  Y=%6d  Z=%6d\n", x, y, z);

        if (x_out != nullptr) { *x_out = x; }
        if (y_out != nullptr) { *y_out = y; }
        if (z_out != nullptr) { *z_out = z; }
        return Status::Ok;
}
