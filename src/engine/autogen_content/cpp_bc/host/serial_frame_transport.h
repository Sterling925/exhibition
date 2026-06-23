// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SERIAL_FRAME_TRANSPORT_H
#define SERIAL_FRAME_TRANSPORT_H

#include <stdint.h>

#include "../communication/i_frame_transport.h"
#include "../communication/scheduler.h"


/// @file serial_frame_transport.h
/// @brief Desktop IFrameTransport backed by a USB-serial port.
/// @details
/// Wraps a host serial device (POSIX `/dev/ttyUSB0` or Windows `COMx`) and
/// re-assembles the on-target byte stream into frame chunks that match the
/// IFrameTransport contract: every receive() call returns at most one
/// complete chunk — either a 6-byte header or a payload of the size declared
/// in the prior header.
///
/// The framing algorithm is identical to the on-target UartFrameTransport
/// adapter: bytes are accumulated into a scratch buffer, the first six bytes
/// are validated against their own CRC (a bad CRC drops the oldest byte and
/// retries), and once a valid header is parsed the next receive() call
/// collects exactly the advertised payload length.
class SerialFrameTransport : public IFrameTransport {
    public:

    /// @brief Construct an unopened transport. Call open() before passing
    ///        the instance to a BusController.
    SerialFrameTransport();
    ~SerialFrameTransport() override;

    SerialFrameTransport(const SerialFrameTransport&) = delete;
    SerialFrameTransport& operator=(const SerialFrameTransport&) = delete;

    /// @brief Open a serial device at the given baud.
    /// @param device_path POSIX: "/dev/ttyUSB0", "/dev/ttyACM0".
    ///                    Windows: "COM7" or "\\\\.\\COM27".
    /// @param baud One of {9600, 19200, 38400, 57600, 115200, 230400,
    ///        460800, 921600}.
    /// @return true on success, false on any failure (the port stays closed).
    bool open(char const* device_path, uint32_t baud);

    /// @brief Close the port if open. Safe to call when not open.
    void close();

    /// @brief True between a successful open() and a close().
    bool is_open() const;

    /// @brief Non-blocking write of `length` bytes.
    /// @return `length` on success, -1 on transport error, 0 if the OS
    ///         could not accept any bytes (rare on a blocking-write port).
    int32_t send(uint8_t const* bytes, uint32_t length) override;

    /// @brief Non-blocking return of one complete frame chunk if available.
    /// @return Number of bytes written to `out`, 0 if no complete chunk is
    ///         ready yet, -1 on transport error or if max_length is smaller
    ///         than the next chunk.
    int32_t receive(uint8_t* out, uint32_t max_length) override;

    /// @brief Abort an in-progress receive and resync to header hunting.
    /// @details Called by the scheduler when it abandons a frame; see
    ///          IFrameTransport::reset_rx().
    void reset_rx() override;

    private:

#ifdef _WIN32
    void* m_handle;            ///< HANDLE; void* to keep windows.h out of the header.
#else
    int   m_fd;
#endif

    enum class RxState : uint8_t { Header, Payload };

    RxState  m_rx_state;
    uint32_t m_expected_payload;
    uint8_t  m_rx_buf[kMaxPayloadBytes]; ///< Large enough for any frame up to kMaxPayloadBytes.
    uint32_t m_rx_filled;
};


#endif // SERIAL_FRAME_TRANSPORT_H
