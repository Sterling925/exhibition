// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "serial_frame_transport.h"

#include <cstring>

#include "../communication/message_header.h"


#ifdef _WIN32

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>

namespace {

bool platform_open(char const* device_path,
                   uint32_t baud,
                   void*& handle_out)
{
    handle_out = nullptr;

    HANDLE h = CreateFileA(device_path,
                           GENERIC_READ | GENERIC_WRITE,
                           0,
                           nullptr,
                           OPEN_EXISTING,
                           0,
                           nullptr);
    if (h == INVALID_HANDLE_VALUE) {
        return false;
    }

    DCB dcb;
    std::memset(&dcb, 0, sizeof(dcb));
    dcb.DCBlength = sizeof(dcb);
    if (!GetCommState(h, &dcb)) {
        CloseHandle(h);
        return false;
    }
    dcb.BaudRate = baud;
    dcb.ByteSize = 8;
    dcb.Parity   = NOPARITY;
    dcb.StopBits = ONESTOPBIT;
    dcb.fBinary  = TRUE;
    dcb.fParity  = FALSE;
    dcb.fOutxCtsFlow    = FALSE;
    dcb.fOutxDsrFlow    = FALSE;
    dcb.fDtrControl     = DTR_CONTROL_DISABLE;
    dcb.fRtsControl     = RTS_CONTROL_DISABLE;
    dcb.fOutX           = FALSE;
    dcb.fInX            = FALSE;
    dcb.fErrorChar      = FALSE;
    dcb.fNull           = FALSE;
    dcb.fAbortOnError   = FALSE;
    if (!SetCommState(h, &dcb)) {
        CloseHandle(h);
        return false;
    }

    // Return immediately from ReadFile with whatever bytes are buffered,
    // 0 if none — matches the non-blocking contract of receive().
    COMMTIMEOUTS to;
    std::memset(&to, 0, sizeof(to));
    to.ReadIntervalTimeout         = MAXDWORD;
    to.ReadTotalTimeoutMultiplier  = 0;
    to.ReadTotalTimeoutConstant    = 0;
    to.WriteTotalTimeoutMultiplier = 0;
    to.WriteTotalTimeoutConstant   = 0;
    if (!SetCommTimeouts(h, &to)) {
        CloseHandle(h);
        return false;
    }

    PurgeComm(h, PURGE_RXCLEAR | PURGE_TXCLEAR);
    handle_out = h;
    return true;
}


void platform_close(void*& handle)
{
    if (handle != nullptr) {
        CloseHandle(static_cast<HANDLE>(handle));
        handle = nullptr;
    }
}


int32_t platform_send(void* handle, uint8_t const* bytes, uint32_t length)
{
    DWORD written = 0;
    if (!WriteFile(static_cast<HANDLE>(handle),
                   bytes,
                   length,
                   &written,
                   nullptr)) {
        return -1;
    }
    return static_cast<int32_t>(written);
}


// Reads up to `want` bytes; returns the number read (0..want), or -1 on hard
// error. The configured COMMTIMEOUTS make ReadFile return immediately with
// whatever is currently buffered, so a short read (including 0) just means no
// more bytes are available right now. Matches the POSIX path's contract.
int32_t platform_read_bytes(void* handle, uint8_t* dst, uint32_t want)
{
    DWORD n = 0;
    if (!ReadFile(static_cast<HANDLE>(handle),
                  dst,
                  static_cast<DWORD>(want),
                  &n,
                  nullptr)) {
        return -1;
    }
    return static_cast<int32_t>(n);
}

} // namespace

#else // POSIX

#include <errno.h>
#include <fcntl.h>
#include <termios.h>
#include <unistd.h>

namespace {

bool baud_to_speed(uint32_t baud, speed_t& speed_out)
{
    switch (baud) {
        case   9600: speed_out = B9600;   return true;
        case  19200: speed_out = B19200;  return true;
        case  38400: speed_out = B38400;  return true;
        case  57600: speed_out = B57600;  return true;
        case 115200: speed_out = B115200; return true;
        case 230400: speed_out = B230400; return true;
#ifdef B460800
        case 460800: speed_out = B460800; return true;
#endif
#ifdef B921600
        case 921600: speed_out = B921600; return true;
#endif
        default: return false;
    }
}


bool platform_open(char const* device_path, uint32_t baud, int& fd_out)
{
    fd_out = -1;

    speed_t speed;
    if (!baud_to_speed(baud, speed)) {
        return false;
    }

    int const fd = ::open(device_path, O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (fd < 0) {
        return false;
    }

    termios tio;
    std::memset(&tio, 0, sizeof(tio));
    if (tcgetattr(fd, &tio) != 0) {
        ::close(fd);
        return false;
    }
    cfmakeraw(&tio);
    if (cfsetispeed(&tio, speed) != 0 || cfsetospeed(&tio, speed) != 0) {
        ::close(fd);
        return false;
    }
    tio.c_cflag |= (CLOCAL | CREAD);
    tio.c_cflag &= ~static_cast<tcflag_t>(CRTSCTS);
    tio.c_cflag &= ~static_cast<tcflag_t>(PARENB);
    tio.c_cflag &= ~static_cast<tcflag_t>(CSTOPB);
    tio.c_cflag &= ~static_cast<tcflag_t>(CSIZE);
    tio.c_cflag |= CS8;
    tio.c_cc[VMIN]  = 0;
    tio.c_cc[VTIME] = 0;
    if (tcsetattr(fd, TCSANOW, &tio) != 0) {
        ::close(fd);
        return false;
    }
    tcflush(fd, TCIOFLUSH);
    fd_out = fd;
    return true;
}


void platform_close(int& fd)
{
    if (fd >= 0) {
        ::close(fd);
        fd = -1;
    }
}


int32_t platform_send(int fd, uint8_t const* bytes, uint32_t length)
{
    uint32_t total = 0;
    while (total < length) {
        ssize_t n = ::write(fd, bytes + total, length - total);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                return static_cast<int32_t>(total);
            }
            return -1;
        }
        total += static_cast<uint32_t>(n);
    }
    return static_cast<int32_t>(total);
}


// Reads up to `want` bytes; returns the number read (0..want), or -1 on hard
// error. The fd is O_NONBLOCK with VMIN=0/VTIME=0, so a single read() returns
// whatever is immediately available; a short read (including 0) means no more
// bytes are waiting right now.
int32_t platform_read_bytes(int fd, uint8_t* dst, uint32_t want)
{
    ssize_t n = ::read(fd, dst, want);
    if (n > 0) {
        return static_cast<int32_t>(n);
    }
    if (n == 0) {
        return 0;
    }
    if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) {
        return 0;
    }
    return -1;
}

} // namespace

#endif


static constexpr uint32_t kHeaderBytes = 6;


SerialFrameTransport::SerialFrameTransport()
    :
#ifdef _WIN32
      m_handle(nullptr)
#else
      m_fd(-1)
#endif
    , m_rx_state(RxState::Header)
    , m_expected_payload(0)
    , m_rx_filled(0)
{
    std::memset(m_rx_buf, 0, sizeof(m_rx_buf));
}


SerialFrameTransport::~SerialFrameTransport()
{
    close();
}


bool SerialFrameTransport::open(char const* device_path, uint32_t baud)
{
    if (is_open()) {
        close();
    }
    if (device_path == nullptr) {
        return false;
    }
#ifdef _WIN32
    return platform_open(device_path, baud, m_handle);
#else
    return platform_open(device_path, baud, m_fd);
#endif
}


void SerialFrameTransport::reset_rx()
{
    m_rx_state         = RxState::Header;
    m_expected_payload = 0;
    m_rx_filled        = 0;
}


void SerialFrameTransport::close()
{
#ifdef _WIN32
    platform_close(m_handle);
#else
    platform_close(m_fd);
#endif
    m_rx_state         = RxState::Header;
    m_expected_payload = 0;
    m_rx_filled        = 0;
}


bool SerialFrameTransport::is_open() const
{
#ifdef _WIN32
    return m_handle != nullptr;
#else
    return m_fd >= 0;
#endif
}


int32_t SerialFrameTransport::send(uint8_t const* bytes, uint32_t length)
{
    if (!is_open() || bytes == nullptr) {
        return -1;
    }
#ifdef _WIN32
    return platform_send(m_handle, bytes, length);
#else
    return platform_send(m_fd, bytes, length);
#endif
}


int32_t SerialFrameTransport::receive(uint8_t* out, uint32_t max_length)
{
    if (!is_open() || out == nullptr) {
        return -1;
    }

    const uint32_t needed =
        (m_rx_state == RxState::Header) ? kHeaderBytes : m_expected_payload;

    // Payload size parsed when a header completes in-loop below; reused after
    // the loop so the header is never deserialized twice.
    uint32_t hdr_payload = 0;

    while (m_rx_filled < needed) {
        // Request exactly the bytes still missing for this chunk in one read:
        // the full header/payload when starting fresh, or a single byte after
        // a resync drop (see below). needed <= sizeof(m_rx_buf), so the
        // destination range is always in bounds.
        const uint32_t want = needed - m_rx_filled;
#ifdef _WIN32
        const int32_t rs = platform_read_bytes(m_handle, m_rx_buf + m_rx_filled, want);
#else
        const int32_t rs = platform_read_bytes(m_fd, m_rx_buf + m_rx_filled, want);
#endif
        if (rs < 0) {
            return -1;
        }
        if (rs == 0) {
            return 0;     // No more bytes available right now.
        }
        m_rx_filled += static_cast<uint32_t>(rs);

        // Header just completed: validate its CRC. On failure, drop the
        // oldest byte (resync) and keep collecting; the next iteration
        // re-requests just the one missing byte and re-parses.
        if (m_rx_state == RxState::Header && m_rx_filled == kHeaderBytes) {
            MessageHeader hdr;
            if (!hdr.deserialize(m_rx_buf)) {
                std::memmove(m_rx_buf, m_rx_buf + 1, kHeaderBytes - 1u);
                m_rx_filled = kHeaderBytes - 1u;
                continue;
            }
            // A size-2 (Protocol T2 inline) header carries its payload in the
            // header itself, so only sizes >= 4 are followed by a chunk.
            hdr_payload = hdr.has_payload_chunk() ? hdr.get_size_bytes() : 0u;
            break;
        }
    }

    if (m_rx_filled < needed) {
        return 0;
    }
    if (needed > max_length) {
        return -1;
    }

    std::memcpy(out, m_rx_buf, needed);
    const int32_t ret = static_cast<int32_t>(needed);

    if (m_rx_state == RxState::Header) {
        if (hdr_payload > 0) {
            m_rx_state         = RxState::Payload;
            m_expected_payload = hdr_payload;
        } else {
            m_rx_state         = RxState::Header;
            m_expected_payload = 0;
        }
    } else {
        m_rx_state         = RxState::Header;
        m_expected_payload = 0;
    }
    m_rx_filled = 0;
    return ret;
}
