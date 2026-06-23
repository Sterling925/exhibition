// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef USB_CDC_FRAME_TRANSPORT_H
#define USB_CDC_FRAME_TRANSPORT_H

#include <stdint.h>

#include "../communication/i_frame_transport.h"
#include "../communication/scheduler.h"


/// @brief Polling-based IFrameTransport over an STM32 USB CDC virtual COM
///        port.
/// @details
/// STM32's USB CDC middleware is interrupt-driven on the receive side: each
/// OUT transfer ends in a `CDC_Receive_HS()` / `CDC_Receive_FS()` callback
/// inside the user-edited `usbd_cdc_if.c`. This transport bridges that
/// callback into the scheduler's polling model with a single-producer /
/// single-consumer ring buffer: the callback calls `deposit_rx()` (forwarded
/// by the generated `exhibition_usb_rx` C-linkage hook in
/// `device_registry.cpp`) and `receive()` drains the ring from the main
/// loop, reassembling header/payload chunks the same way
/// `UartFrameTransport` does.
///
/// Transmits are blocking from the caller's point of view: `send()` retries
/// the supplied `transmit_fn` while it returns `USBD_BUSY` and gives up
/// after a bounded number of retries.
///
/// This MVP transport assumes:
///   - The CubeMX USB middleware has been initialised (`MX_USB_DEVICE_Init()`)
///     and is enumerated by the host before any byte is sent.
///   - The user has installed the `exhibition_usb_rx` hook inside
///     `CDC_Receive_HS()` / `CDC_Receive_FS()` per `INTEGRATION_GUIDE.md`.
///   - The transmit function pointer matches the CubeMX-generated
///     `CDC_Transmit_HS` / `CDC_Transmit_FS` signature
///     (`uint8_t (*)(uint8_t*, uint16_t)`).
class UsbCdcFrameTransport : public IFrameTransport {
    public:

    /// @brief Pointer to the CubeMX-generated CDC transmit function.
    /// @details Both `CDC_Transmit_HS` and `CDC_Transmit_FS` share this
    ///          signature; codegen picks the one matching the selected USB
    ///          speed and passes it to the constructor.
    using CdcTransmitFn = uint8_t (*)(uint8_t* buf, uint16_t len);

    /// @brief Construct bound to a CDC transmit function.
    /// @param transmit_fn Non-null pointer to `CDC_Transmit_HS` or
    ///                    `CDC_Transmit_FS`. Must outlive *this*.
    explicit UsbCdcFrameTransport(CdcTransmitFn transmit_fn);

    /// @brief Send one frame chunk via the CDC transmit function.
    /// @details Retries on `USBD_BUSY` up to a bounded count; returns -1
    ///          if the link never accepts the chunk.
    int32_t send(uint8_t const* bytes, uint32_t length) override;

    /// @brief Receive at most one frame chunk from the ring. Non-blocking.
    int32_t receive(uint8_t* out, uint32_t max_length) override;

    /// @brief Abort an in-progress receive and resync to header hunting.
    /// @details Called by the scheduler when it abandons a frame; see
    ///          IFrameTransport::reset_rx().
    void reset_rx() override;

    /// @brief Push bytes received in the CDC OUT callback into the ring.
    /// @details Called from interrupt context by `exhibition_usb_rx()`.
    ///          Bytes that would overflow the ring are dropped.
    void deposit_rx(uint8_t const* buf, uint32_t len);

    private:

    /// @brief What kind of chunk we are currently collecting from the wire.
    enum class RxState : uint8_t {
        Header,    ///< Next 6 bytes form a MessageHeader.
        Payload,   ///< Next m_expected_payload bytes form a payload.
    };

    /// @brief Pop one byte from the ring. Returns false if empty.
    bool pop_ring_byte(uint8_t* out);

    /// @brief Pop up to `want` bytes from the ring into `dst`.
    /// @return Number of bytes actually popped (0..want); 0 means the ring is
    ///         currently empty.
    uint32_t pop_ring_bytes(uint8_t* dst, uint32_t want);

    CdcTransmitFn       m_transmit_fn;
    RxState             m_rx_state;
    uint32_t            m_expected_payload;
    uint8_t             m_rx_buf[kMaxPayloadBytes];
    uint32_t            m_rx_filled;

    // Ring buffer: written by deposit_rx() (ISR context), read by
    // receive() (main loop). Single producer + single consumer means a
    // pair of volatile indices is sufficient on Cortex-M.
    static constexpr uint32_t kRingSize = 4096;
    volatile uint8_t    m_ring[kRingSize];
    volatile uint32_t   m_head; // next write slot
    volatile uint32_t   m_tail; // next read slot
};


#endif // USB_CDC_FRAME_TRANSPORT_H
