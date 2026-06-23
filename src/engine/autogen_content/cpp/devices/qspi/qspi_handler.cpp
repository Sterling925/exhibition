// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "qspi_handler.h"

#include "../../generated/command_ids.h"
#include "../common/helper_functions.h"

static constexpr uint32_t kQspiTimeoutMs = 1000;


/// @brief Map a wire `data_lines` selector (1/2/4) to a HAL data mode.
/// @details Any unrecognized value defaults to single-line, the universally
///          supported mode.
static uint32_t qspi_data_mode(const uint8_t data_lines)
{
    switch (data_lines) {
        case 4:  return QSPI_DATA_4_LINES;
        case 2:  return QSPI_DATA_2_LINES;
        default: return QSPI_DATA_1_LINE;
    }
}


/// @brief Initialise @p cmd with the fields common to every QUADSPI command.
/// @details Instruction phase on a single line; no address, alternate bytes,
///          dummy cycles, or data by default. Each opcode handler overrides
///          only the fields it needs (address, dummy cycles, data mode/count).
static void qspi_init_command(QSPI_CommandTypeDef& cmd, const uint8_t instruction)
{
    cmd.Instruction        = instruction;
    cmd.Address            = 0;
    cmd.AlternateBytes     = 0;
    cmd.AddressSize        = QSPI_ADDRESS_24_BITS;
    cmd.AlternateBytesSize = QSPI_ALTERNATE_BYTES_8_BITS;
    cmd.DummyCycles        = 0;
    cmd.InstructionMode    = QSPI_INSTRUCTION_1_LINE;
    cmd.AddressMode        = QSPI_ADDRESS_NONE;
    cmd.AlternateByteMode  = QSPI_ALTERNATE_BYTES_NONE;
    cmd.DataMode           = QSPI_DATA_NONE;
    cmd.NbData             = 0;
    cmd.DdrMode            = QSPI_DDR_MODE_DISABLE;
    cmd.DdrHoldHalfCycle   = QSPI_DDR_HHC_ANALOG_DELAY;
    cmd.SIOOMode           = QSPI_SIOO_INST_EVERY_CMD;
}


/// @brief Decode a big-endian 24-bit address from three payload bytes.
static uint32_t qspi_address_be24(uint8_t const* src)
{
    return (static_cast<uint32_t>(src[0]) << 16)
         | (static_cast<uint32_t>(src[1]) << 8)
         |  static_cast<uint32_t>(src[2]);
}


QspiHandler::QspiHandler(QSPI_HandleTypeDef* hqspi, const IClock& clock)
    : m_hqspi(hqspi)
    , m_clock(clock)
{
}


Status QspiHandler::handle(const MessageHeader& cmd_header,
                           uint8_t const* cmd_payload,
                           uint8_t* resp_buf,
                           uint32_t resp_buf_max,
                           uint32_t& resp_len_out)
{
    resp_len_out = 0;

    const uint32_t cmd_len = cmd_header.get_size_bytes();
    if (cmd_len < 1 || cmd_payload == nullptr) {
        if (resp_buf_max >= 1) {
            resp_buf[0] = 0xE0;
            resp_len_out = 1;
        }
        return Status::HandlerError;
    }

    const uint8_t op = cmd_payload[0];
    switch (static_cast<QspiCmd>(op)) {
        case QspiCmd::Read: {
            // [op, instr, addr_be24, dummy, data_lines, len_le16]
            if (cmd_len < 9u) {
                return Status::HandlerError;
            }
            const uint8_t  instr      = cmd_payload[1];
            const uint32_t address    = qspi_address_be24(cmd_payload + 2);
            const uint8_t  dummy      = cmd_payload[5];
            const uint8_t  data_lines = cmd_payload[6];
            const uint32_t len        = extract_u16_le(cmd_payload + 7);

            if (len == 0u) {
                return Status::Ok;
            }
            if (len > resp_buf_max) {
                return Status::PayloadTooLarge;
            }

            QSPI_CommandTypeDef cmd;
            qspi_init_command(cmd, instr);
            cmd.Address     = address;
            cmd.AddressMode = QSPI_ADDRESS_1_LINE;
            cmd.DummyCycles = dummy;
            cmd.DataMode    = qspi_data_mode(data_lines);
            cmd.NbData      = len;

            if (HAL_QSPI_Command(m_hqspi, &cmd, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_QSPI_Receive(m_hqspi, resp_buf, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            resp_len_out = len;
            return Status::Ok;
        }
        case QspiCmd::Program: {
            // [op, instr, addr_be24, data_lines, len_le16, data...]
            if (cmd_len < 8u) {
                return Status::HandlerError;
            }
            const uint8_t  instr      = cmd_payload[1];
            const uint32_t address    = qspi_address_be24(cmd_payload + 2);
            const uint8_t  data_lines = cmd_payload[5];
            const uint32_t len        = extract_u16_le(cmd_payload + 6);
            if (len > (cmd_len - 8u)) {
                return Status::HandlerError;
            }

            QSPI_CommandTypeDef cmd;
            qspi_init_command(cmd, instr);
            cmd.Address     = address;
            cmd.AddressMode = QSPI_ADDRESS_1_LINE;
            if (len > 0u) {
                cmd.DataMode = qspi_data_mode(data_lines);
                cmd.NbData   = len;
            }

            if (HAL_QSPI_Command(m_hqspi, &cmd, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            if (len > 0u) {
                if (HAL_QSPI_Transmit(m_hqspi,
                                      const_cast<uint8_t*>(cmd_payload + 8),
                                      kQspiTimeoutMs) != HAL_OK) {
                    return Status::HandlerError;
                }
            }
            return Status::Ok;
        }
        case QspiCmd::Erase: {
            // [op, instr, addr_be24]
            if (cmd_len < 5u) {
                return Status::HandlerError;
            }
            const uint8_t  instr   = cmd_payload[1];
            const uint32_t address = qspi_address_be24(cmd_payload + 2);

            QSPI_CommandTypeDef cmd;
            qspi_init_command(cmd, instr);
            cmd.Address     = address;
            cmd.AddressMode = QSPI_ADDRESS_1_LINE;

            if (HAL_QSPI_Command(m_hqspi, &cmd, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        case QspiCmd::ReadStatus: {
            // [op, instr] -> 1 status byte
            if (cmd_len < 2u) {
                return Status::HandlerError;
            }
            if (resp_buf_max < 1u) {
                return Status::PayloadTooLarge;
            }
            const uint8_t instr = cmd_payload[1];

            QSPI_CommandTypeDef cmd;
            qspi_init_command(cmd, instr);
            cmd.DataMode = QSPI_DATA_1_LINE;
            cmd.NbData   = 1;

            if (HAL_QSPI_Command(m_hqspi, &cmd, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            if (HAL_QSPI_Receive(m_hqspi, resp_buf, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            resp_len_out = 1;
            return Status::Ok;
        }
        case QspiCmd::WriteEnable: {
            // [op, instr] -> command only, no data
            if (cmd_len < 2u) {
                return Status::HandlerError;
            }
            const uint8_t instr = cmd_payload[1];

            QSPI_CommandTypeDef cmd;
            qspi_init_command(cmd, instr);

            if (HAL_QSPI_Command(m_hqspi, &cmd, kQspiTimeoutMs) != HAL_OK) {
                return Status::HandlerError;
            }
            return Status::Ok;
        }
        default:
            if (resp_buf_max >= 1) {
                resp_buf[0] = op;
                resp_len_out = 1;
            }
            return Status::HandlerError;
    }
}
