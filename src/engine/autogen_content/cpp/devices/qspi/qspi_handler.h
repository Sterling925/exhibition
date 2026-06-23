// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef QSPI_HANDLER_H
#define QSPI_HANDLER_H

#include <stdint.h>

#include "main.h"
#include "../../communication/i_sub_address_handler.h"
#include "../../communication/i_clock.h"


/// @brief Sub-address handler for the on-board QUADSPI (Quad SPI) controller.
/// @details
/// A flash-oriented wrapper over the indirect-mode HAL API
/// (`HAL_QSPI_Command` followed by `HAL_QSPI_Transmit`/`HAL_QSPI_Receive`).
/// Every command carries the flash instruction byte explicitly so the handler
/// is vendor-agnostic. The instruction and address phases are always issued on
/// a single line; the data phase line width is selectable per command via the
/// `data_lines` field (1, 2, or 4). Addresses are 24-bit, big-endian on the
/// wire (covers flash up to 16 MB).
///
/// Recognized opcodes:
///   - 0x01 READ : read @p len bytes from @p address.
///           Command payload is 9 bytes:
///             [0] opcode
///             [1] flash instruction (e.g. 0x03 read, 0x6B fast-read quad)
///             [2..4] address, big-endian 24-bit
///             [5] dummy cycles
///             [6] data_lines (1, 2, or 4)
///             [7..8] length, little-endian uint16
///           Response payload is @p len bytes.
///   - 0x02 PROGRAM : program (write) the trailing data bytes to @p address.
///           Command payload is 8+len bytes:
///             [0] opcode
///             [1] flash instruction (e.g. 0x02 page program)
///             [2..4] address, big-endian 24-bit
///             [5] data_lines (1, 2, or 4)
///             [6..7] length, little-endian uint16
///             [8..]  data bytes
///           Response payload is 0 bytes.
///   - 0x03 ERASE : issue an erase instruction at @p address.
///           Command payload is 5 bytes: [opcode, instruction, address_be24].
///           Response payload is 0 bytes.
///   - 0x04 READ_STATUS : read one status-register byte.
///           Command payload is 2 bytes: [opcode, instruction].
///           Response payload is 1 byte.
///   - 0x05 WRITE_ENABLE : issue a single instruction with no data.
///           Command payload is 2 bytes: [opcode, instruction].
///           Response payload is 0 bytes.
///
/// @note Erase/program completion is not polled here; the bus controller is
///       expected to poll READ_STATUS for the write-in-progress bit and to send
///       WRITE_ENABLE before each program/erase as the target flash requires.
class QspiHandler : public ISubAddressHandler {
    public:

    /// @brief Construct bound to the HAL QUADSPI handle.
    /// @param hqspi HAL QUADSPI handle (e.g. `&hqspi`). Must outlive *this*.
    /// @param clock Read-only BC-synchronized clock; not yet consumed. Must outlive *this*.
    QspiHandler(QSPI_HandleTypeDef* hqspi, const IClock& clock);

    Status handle(const MessageHeader& cmd_header,
                  uint8_t const* cmd_payload,
                  uint8_t* resp_buf,
                  uint32_t resp_buf_max,
                  uint32_t& resp_len_out) override;

    private:

    QSPI_HandleTypeDef* m_hqspi;

    /// @brief Read-only BC-synchronized clock; not yet consumed.
    const IClock& m_clock;
};


#endif // QSPI_HANDLER_H
