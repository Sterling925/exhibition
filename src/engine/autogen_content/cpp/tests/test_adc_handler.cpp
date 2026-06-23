// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

/// @file test_adc_handler.cpp
/// @brief Unit tests for AdcHandler::handle().
/// @details
/// HAL functions are replaced by the stubs in main.h (found first on the
/// -I. include path). The IClock is supplied as a FakeClock so timestamp
/// output is deterministic.

#include "thirdparty/catch_amalgamated.hpp"

#include "../devices/adc/adc_handler.h"
#include "../generated/command_ids.h"
#include "../communication/message_header.h"
#include "../communication/i_clock.h"

#include <cstring>


///////////////////////////////////////////////////////////////////////////////
// HAL stub definitions (declared extern in main.h)
///////////////////////////////////////////////////////////////////////////////

HAL_StatusTypeDef g_hal_adc_start_result = HAL_OK;
HAL_StatusTypeDef g_hal_adc_poll_result  = HAL_OK;
uint32_t          g_hal_adc_value        = 0u;

/// Reset all stubs to a clean pass-through state.
static void reset_hal_stubs()
{
    g_hal_adc_start_result = HAL_OK;
    g_hal_adc_poll_result  = HAL_OK;
    g_hal_adc_value        = 0u;
}


///////////////////////////////////////////////////////////////////////////////
// FakeClock
///////////////////////////////////////////////////////////////////////////////

namespace {

class FakeClock : public IClock {
    public:
    uint32_t m_now_ms    = 0u;
    uint32_t m_now_bc_ms = 0u;

    uint32_t now_ms() const override { return m_now_ms; }
    uint32_t now_bc_ms() const override { return m_now_bc_ms; }
    void set_bc_offset(const int32_t, const int32_t) override {}
    void set_bc_discipline(const uint32_t, const int32_t, const int32_t, const int32_t) override {}
    void get_bc_offset(int32_t& w, int32_t& f) const override { w = 0; f = 0; }
    int32_t bc_skew_ppb() const override { return 0; }
    bool has_sync() const override { return true; }
    void set_has_sync(const bool) override {}
};


/// Build a MessageHeader announcing `size` bytes of payload.
MessageHeader make_header(PayloadSizes size)
{
    MessageHeader h;
    h.set_priority(MessagePriorities::Medium);
    h.set_command_flag(true);
    h.set_size(size);
    h.set_fault_flag(false);
    h.set_sub_address(2u);
    h.set_payload_crc(0u);
    h.update_header_crc();
    return h;
}

} // namespace


///////////////////////////////////////////////////////////////////////////////
// Tests
///////////////////////////////////////////////////////////////////////////////

/// t1
///
/// Happy path: HAL succeeds, clock returns a known timestamp.
/// Verify the 6-byte response carries counts in bytes 0-1 and timestamp in
/// bytes 2-5, both little-endian, and that resp_len_out == 6.
TEST_CASE("t1 AdcHandler SAMPLE happy path returns counts and timestamp", "[adc_handler]")
{
    reset_hal_stubs();
    g_hal_adc_value = 0x1234u;

    FakeClock clock;
    clock.m_now_bc_ms = 0xDEADBEEFu;

    ADC_HandleTypeDef hadc{};
    AdcHandler handler(&hadc, 3u, clock);

    const uint8_t opcode = static_cast<uint8_t>(AdcCmd::Sample);
    MessageHeader hdr = make_header(PayloadSizes::size_2);

    uint8_t resp[16] = {0};
    uint32_t resp_len = 0u;

    const Status s = handler.handle(hdr, &opcode, resp, sizeof(resp), resp_len);

    REQUIRE(s == Status::Ok);
    REQUIRE(resp_len == 6u);

    // Counts: little-endian uint16 at bytes 0-1.
    const uint16_t counts = static_cast<uint16_t>(resp[0])
                          | (static_cast<uint16_t>(resp[1]) << 8);
    REQUIRE(counts == 0x1234u);

    // Timestamp: little-endian uint32 at bytes 2-5.
    const uint32_t ts = static_cast<uint32_t>(resp[2])
                      | (static_cast<uint32_t>(resp[3]) <<  8)
                      | (static_cast<uint32_t>(resp[4]) << 16)
                      | (static_cast<uint32_t>(resp[5]) << 24);
    REQUIRE(ts == 0xDEADBEEFu);
}


/// t2
///
/// Buffer too small: resp_buf_max < 6 must return HandlerError and leave
/// resp_len_out at zero (no partial write).
TEST_CASE("t2 AdcHandler SAMPLE rejects undersized response buffer", "[adc_handler]")
{
    reset_hal_stubs();

    FakeClock clock;
    ADC_HandleTypeDef hadc{};
    AdcHandler handler(&hadc, 0u, clock);

    const uint8_t opcode = static_cast<uint8_t>(AdcCmd::Sample);
    MessageHeader hdr = make_header(PayloadSizes::size_2);

    uint8_t resp[5] = {0};   // Only 5 bytes — too small for 6-byte response.
    uint32_t resp_len = 99u;

    const Status s = handler.handle(hdr, &opcode, resp, sizeof(resp), resp_len);

    REQUIRE(s == Status::HandlerError);
    REQUIRE(resp_len == 0u);
}


/// t3
///
/// Empty payload (cmd_header reports 0 bytes): must return HandlerError and
/// write 0xE0 into byte 0 of the response buffer.
TEST_CASE("t3 AdcHandler rejects empty command payload", "[adc_handler]")
{
    reset_hal_stubs();

    FakeClock clock;
    ADC_HandleTypeDef hadc{};
    AdcHandler handler(&hadc, 0u, clock);

    // Header announces zero payload bytes — the handler must reject it.
    MessageHeader hdr = make_header(PayloadSizes::size_0);

    uint8_t resp[16] = {0};
    uint32_t resp_len = 0u;

    const Status s = handler.handle(hdr, nullptr, resp, sizeof(resp), resp_len);

    REQUIRE(s == Status::HandlerError);
    REQUIRE(resp_len == 1u);
    REQUIRE(resp[0] == 0xE0u);
}


/// t4
///
/// Unknown opcode: must return HandlerError and echo the opcode in byte 0
/// of the response buffer.
TEST_CASE("t4 AdcHandler echoes unknown opcode in response byte 0", "[adc_handler]")
{
    reset_hal_stubs();

    FakeClock clock;
    ADC_HandleTypeDef hadc{};
    AdcHandler handler(&hadc, 0u, clock);

    const uint8_t bad_opcode = 0xFFu;
    MessageHeader hdr = make_header(PayloadSizes::size_2);

    uint8_t resp[16] = {0};
    uint32_t resp_len = 0u;

    const Status s = handler.handle(hdr, &bad_opcode, resp, sizeof(resp), resp_len);

    REQUIRE(s == Status::HandlerError);
    REQUIRE(resp_len == 1u);
    REQUIRE(resp[0] == 0xFFu);
}


/// t5
///
/// HAL_ADC_Start failure: must return HandlerError without writing any
/// response bytes.
TEST_CASE("t5 AdcHandler returns HandlerError when HAL_ADC_Start fails", "[adc_handler]")
{
    reset_hal_stubs();
    g_hal_adc_start_result = HAL_ERROR;

    FakeClock clock;
    ADC_HandleTypeDef hadc{};
    AdcHandler handler(&hadc, 0u, clock);

    const uint8_t opcode = static_cast<uint8_t>(AdcCmd::Sample);
    MessageHeader hdr = make_header(PayloadSizes::size_2);

    uint8_t resp[16] = {0};
    uint32_t resp_len = 0u;

    const Status s = handler.handle(hdr, &opcode, resp, sizeof(resp), resp_len);

    REQUIRE(s == Status::HandlerError);
    REQUIRE(resp_len == 0u);
}
