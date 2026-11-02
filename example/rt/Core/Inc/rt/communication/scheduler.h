// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef SCHEDULER_H
#define SCHEDULER_H

#include <stdint.h>

#include "rt/communication/message_header.h"
#include "rt/communication/i_clock.h"
#include "rt/communication/i_frame_transport.h"
#include "rt/communication/i_sub_address_handler.h"
#include "i_bus_fault_observer.h"
#include "rt/communication/fault_counter.h"
#include "rt/common/helper_functions.h"

/// @file scheduler.h
/// @brief Scheduler for T1 protocol messages
/// @details
/// Two cooperating classes:
///   - BusController (BC): submits commands, drives the bus, owns a priority
///     queue and one in-flight transaction per sub-address.
///   - RemoteTerminal (RT): listens, dispatches commands to per-sub-address
///     handlers, and emits acks / responses / fault frames.
///
/// Both classes are single-threaded; the caller drives them by polling
/// tick() from a main loop or RTOS task. Transport, clock, and handlers are
/// dependency-injected so the classes can be unit-tested with fakes.


///////////////////////////////////////////////////////////////////////////////
// Tuneable constants
///////////////////////////////////////////////////////////////////////////////

/// @brief Payload size at or below which the bytes are carried inline in the
///        Request / response slot. Matches the small/large frame boundary in
///        PayloadSizes (small caps at 14 B, large starts at 16 B).
constexpr uint32_t kInlinePayloadBytes = 16;

/// @brief Size of the per-sub-address slot tables in BC and RT.
/// @details
/// Sub-address 0 is reserved by the protocol, so the usable range is
/// 1..kMaxSubAddresses-1. Raise this if a deployment needs more endpoints;
/// the memory cost scales linearly.
constexpr uint8_t kMaxSubAddresses = 32;

/// @brief Capacity of the BC priority queue.
constexpr uint8_t kBcQueueDepth = 16;

/// @brief Per-frame deadline before a transaction is declared Timeout.
constexpr uint32_t kFrameTimeoutMs = 50;

/// @brief Largest payload the RT scratch buffers can hold.
/// @details
/// Must be one of the values supported by PayloadSizes (16, 32, 64, 128, 256,
/// 512, 1024, 2048). The wire format is unaffected — the header can still
/// encode any size up to 2048, but the RT will reject commands whose declared
/// payload exceeds this limit. Raise to reclaim larger payloads; lower to
/// save RAM.
constexpr uint32_t kMaxPayloadBytes = 64;


static_assert( kFrameTimeoutMs > calc_timeout_min(115'200 ,64+6, 9));

///////////////////////////////////////////////////////////////////////////////
// Public helpers
///////////////////////////////////////////////////////////////////////////////

/// @brief Map a byte count to the smallest PayloadSizes enum that fits it.
/// @details
/// Caller must zero-pad to the resulting enum's natural size before sending.
/// Returns PayloadSizes::size_2048 for any input larger than 2048. Shared
/// between BC and RT so a generated host-side client can pick the same wire
/// slot the on-target scheduler would.
static inline PayloadSizes bytes_to_payload_size(uint32_t bytes)
{
    struct Row { uint32_t bytes; PayloadSizes ps; };
    static const Row k_table[] = {
        {    0, PayloadSizes::size_0    },
        {    2, PayloadSizes::size_2    },
        {    4, PayloadSizes::size_4    },
        {    6, PayloadSizes::size_6    },
        {    8, PayloadSizes::size_8    },
        {   10, PayloadSizes::size_10   },
        {   12, PayloadSizes::size_12   },
        {   14, PayloadSizes::size_14   },
        {   16, PayloadSizes::size_16   },
        {   32, PayloadSizes::size_32   },
        {   64, PayloadSizes::size_64   },
        {  128, PayloadSizes::size_128  },
        {  256, PayloadSizes::size_256  },
        {  512, PayloadSizes::size_512  },
        { 1024, PayloadSizes::size_1024 },
        { 2048, PayloadSizes::size_2048 },
    };
    for (auto const& e : k_table) {
        if (e.bytes >= bytes) {
            return e.ps;
        }
    }
    return PayloadSizes::size_2048;
}


/// @brief Number of payload bytes carried on the wire for a given size enum.
/// @details
/// Inverse of bytes_to_payload_size for the discrete PayloadSizes values.
/// Mirrors MessageHeader::get_size_bytes: the enum's low 3 bits index the
/// size and bit 0x10 selects the small/large table.
static inline uint32_t payload_size_to_bytes(PayloadSizes ps)
{
    static constexpr uint32_t small[8] = { 0, 2, 4, 6, 8, 10, 12, 14 };
    static constexpr uint32_t large[8] = { 16, 32, 64, 128, 256, 512, 1024, 2048 };
    const uint8_t raw = static_cast<uint8_t>(ps);
    return (raw & 0x10) ? large[raw & 0x07] : small[raw & 0x07];
}


///////////////////////////////////////////////////////////////////////////////
// Public types
///////////////////////////////////////////////////////////////////////////////

/// @brief Caller-supplied descriptor for one BC transaction.
/// @details
/// Buffer ownership is hybrid:
///   - cmd_len <= kInlinePayloadBytes : bytes copied into cmd_inline.
///   - cmd_len  > kInlinePayloadBytes : cmd_ext must outlive the transaction
///                                      until the completion callback fires.
/// The response side mirrors this: if resp_buf_max <= kInlinePayloadBytes
/// the BC writes into its internal slot inline buffer; otherwise the BC
/// writes into the caller-supplied resp_ext (which must remain valid until
/// the callback fires).
struct Request {
    uint8_t            sub_address;
    MessagePriorities  priority;
    bool               is_read;        ///< true = expect a response payload back.
    PayloadSizes       cmd_size;       ///< Must match cmd_len's natural slot.
    uint32_t           cmd_len;        ///< 0..kMaxPayloadBytes.
    uint8_t const*     cmd_ext;        ///< Used iff cmd_len > kInlinePayloadBytes.
    uint8_t            cmd_inline[kInlinePayloadBytes];
    uint8_t*           resp_ext;       ///< Used iff resp_buf_max > kInlinePayloadBytes.
    uint32_t           resp_buf_max;   ///< 0 for pure writes.
};


/// @brief Completion callback invoked by BusController::tick() when a
///        transaction reaches a terminal state.
/// @param status Final status code.
/// @param resp_header Response header (zeroed for write transactions).
/// @param resp_payload Pointer to response bytes; valid only for the
///        duration of the call. nullptr if resp_len == 0.
/// @param resp_len Length of resp_payload.
/// @param user The user_ctx originally passed to submit().
using CompletionFn = void (*)(Status status,
                              const MessageHeader& resp_header,
                              uint8_t const* resp_payload,
                              uint32_t resp_len,
                              void* user);


///////////////////////////////////////////////////////////////////////////////
// BusController
///////////////////////////////////////////////////////////////////////////////

/// @brief Drives commands onto the bus and routes responses back to callers.
/// @details
/// Single-threaded. tick() is not reentrant. Completion callbacks fire from
/// inside tick(); a callback may call submit() (the slot is reset to Idle
/// before the callback runs) but must not call tick().
class BusController {
    public:

    /// @brief Construct with injected dependencies. The references must
    ///        outlive *this.
    /// @param fault_observer Optional observer notified on every failed
    ///        transaction attempt (timeout / CRC / fault frame), including
    ///        ones recovered by the automatic retry. Pass nullptr (the
    ///        default) to opt out; the on-target RT build does so. When
    ///        non-null it must outlive *this.
    BusController(IFrameTransport& transport, IClock& clock,
                  IBusFaultObserver* fault_observer = nullptr);
    ~BusController() = default;

    /// @brief Queue a request for transmission.
    /// @param req Caller-owned; copied (inline) or referenced (cmd_ext /
    ///        resp_ext) per the kInlinePayloadBytes rule.
    /// @param cb Invoked exactly once from a future tick().
    /// @param user Passed through to cb.
    /// @param status_out Reason for rejection when false is returned.
    /// @return true if accepted, false otherwise.
    bool submit(const Request& req, CompletionFn cb, void* user, Status& status_out);

    /// @brief Advance the BC state machines by one step.
    /// @details
    /// Pops the highest-priority queued request whose sub-address slot is
    /// Idle and starts it, then walks every active slot and attempts at
    /// most one send + one receive across the whole BC. Never blocks.
    void tick();

    /// @brief Number of queued (not-yet-started) requests. Diagnostic only.
    uint8_t pending_count() const;

    /// @brief Running count of messages that failed due to a timeout or a
    ///        CRC / malformed-frame error, including ones later recovered by
    ///        the single automatic retry. Diagnostic only.
    uint32_t fault_count() const;

    private:

    enum class BcSlotState : uint8_t {
        Idle = 0,
        SendCmdHeader,
        WaitCmdAck,
        SendCmdPayload,
        WaitWriteAck,       ///< Write path: final size-0 ack.
        WaitRespHeader,     ///< Read path: response header.
        RecvRespPayload,
    };

    struct QueuedReq {
        Request      req;
        CompletionFn cb;
        void*        user;
        uint8_t      priority_byte;
    };

    struct BcSlot {
        BcSlotState   state;
        Request       req;
        CompletionFn  cb;
        void*         user;
        uint32_t      deadline_ms;
        MessageHeader pending_header;     ///< Cmd header sent (for payload CRC reference).
        MessageHeader resp_header;        ///< Response header captured on read path.
        uint8_t       resp_inline[kInlinePayloadBytes];
        uint32_t      resp_len;
        bool          ack_was_fault;      ///< Latched when an ack/resp header carries fault bit.
        bool          retried;            ///< true once this transaction has used its one automatic retry.
    };

    /// @brief Build the cmd header the caller's Request would produce.
    void build_cmd_header(const Request& req, MessageHeader& out) const;

    /// @brief Insert into the priority queue maintaining stable order on ties.
    /// @return true if inserted, false if the queue is full.
    bool enqueue_sorted(const QueuedReq& q);

    /// @brief Try to start the highest-priority queued request whose slot is Idle.
    void start_next_if_possible();

    /// @brief Per-tick state-machine step for one active slot.
    /// @param sub_addr Sub-address whose slot to advance.
    /// @param did_send Set true if a send was attempted this tick (in or out).
    /// @param did_recv Set true if a receive was attempted this tick.
    void advance_slot(uint8_t sub_addr, bool& did_send, bool& did_recv);

    /// @brief Reset a slot to Idle and fire its completion callback.
    void complete_slot(uint8_t sub_addr, Status status);

    /// @brief Finish a slot whose response header declared size 2: copy the
    ///        inline payload (header bytes 2-3) into the response buffer, then
    ///        complete Ok, or fail RemoteFault if the fault bit was latched.
    void complete_inline_resp(uint8_t sub_addr);

    /// @brief Handle a failed transaction. Timeout / CRC / malformed failures
    ///        are counted and, if the slot has not retried yet, re-sent once
    ///        starting from the command header. Otherwise (non-retryable
    ///        failure, or retry already spent) the slot is completed via
    ///        complete_slot(). In all cases the injected IBusFaultObserver (if
    ///        any) is notified of the failed attempt before the retry /
    ///        completion decision is acted on.
    void fail_slot(uint8_t sub_addr, Status status);

    /// @brief Return the cmd payload pointer for a slot (inline or external).
    static uint8_t const* slot_cmd_ptr(const BcSlot& slot);

    /// @brief Return the response buffer pointer/capacity for a slot.
    static uint8_t* slot_resp_buf(BcSlot& slot, uint32_t& cap_out);

    IFrameTransport&   m_transport;
    IClock&            m_clock;
    IBusFaultObserver* m_fault_observer;   ///< Optional; notified on failed attempts. May be null.
    QueuedReq        m_queue[kBcQueueDepth];
    uint8_t          m_queue_len;
    uint8_t          m_active_sa;   ///< Sub-address of the one in-flight slot, 0 if none.
    BcSlot           m_slots[kMaxSubAddresses];
    FaultCounter     m_fault_counter;   ///< Timeout / CRC fault total for this BC.
};


///////////////////////////////////////////////////////////////////////////////
// RemoteTerminal
///////////////////////////////////////////////////////////////////////////////

/// @brief Listens on the bus, dispatches commands to per-sub-address handlers.
/// @details
/// Single-threaded. tick() is not reentrant. Handlers run synchronously
/// inside tick(); they may register/unregister other handlers but must not
/// call tick().
class RemoteTerminal {
    public:

    /// @brief Construct with injected dependencies. The references must
    ///        outlive *this.
    /// @param fault_counter Shared counter incremented on every inbound message
    ///        that fails due to a timeout or a CRC / malformed-frame error. Must
    ///        outlive *this; the system handler reads it back over the wire.
    RemoteTerminal(IFrameTransport& transport, IClock& clock, FaultCounter& fault_counter);
    ~RemoteTerminal() = default;

    /// @brief Bind a handler to a sub-address. Pass nullptr to unregister.
    /// @param service_hash Per-service API hash for this sub-address. The RT
    ///        answers the universal GET_SERVICE_HASH command (opcode 0x00) with
    ///        this value directly, before the handler is consulted, so every
    ///        sub-address reports its hash uniformly without per-handler code.
    /// @return true on success; false if sub_addr == 0 or out of range.
    bool register_handler(uint8_t sub_addr, ISubAddressHandler* handler,
                          uint32_t service_hash = 0u);

    /// @brief Advance the RT state machine by one step.
    void tick();

    /// @brief Clock time (IClock::now_ms) at which the most recent valid
    ///        header arrived. While a handler runs this is the arrival of the
    ///        command being handled; time sync uses it as the receive stamp.
    uint32_t cmd_header_rx_ms() const { return m_cmd_rx_ms; }

    private:

    enum class RtState : uint8_t {
        Idle = 0,
        RxPayload,
        TxRespPayload,
    };

    /// @brief Send a serialized header followed (optionally) by a payload.
    /// @details Sets the fault bit before serializing if fault is true.
    /// @return true on full success, false on transport error.
    bool send_ack(uint8_t sub_addr,
                  MessagePriorities prio,
                  PayloadSizes size,
                  bool fault,
                  uint8_t const* payload,
                  uint32_t payload_len);

    /// @brief Send a fault response carrying a single Status code as its payload.
    /// @details Wraps send_ack with a 2-byte frame {code, 0}; the protocol has
    ///          no 1-byte payload size, so the code occupies byte 0 and byte 1
    ///          is zero padding. Used for RT-detected protocol faults so the BC
    ///          can report the specific reason instead of a bare fault bit.
    /// @return true on full success, false on transport error.
    bool send_fault(uint8_t sub_addr, MessagePriorities prio, Status code);

    /// @brief Run a fully received command (in m_cmd_payload, described by
    ///        m_active_cmd_header) and send its response: the universal
    ///        GET_SERVICE_HASH check, the handler, App-tag fault prepending,
    ///        padding, then the ack / inline response / response header.
    /// @param now Tick time, used for the TxRespPayload deadline.
    void dispatch_command(uint32_t now);

    /// @brief Abandon the in-flight inbound frame: return to Idle and resync
    ///        the transport framer (reset_rx) so a stuck partial payload cannot
    ///        desync framing from protocol state.
    void abort_rx();

    IFrameTransport&    m_transport;
    IClock&             m_clock;
    FaultCounter&       m_fault_counter;
    ISubAddressHandler* m_handlers[kMaxSubAddresses];
    /// Per-sub-address API hash, indexed by sub-address. Answered directly for
    /// the universal GET_SERVICE_HASH command (opcode 0x00); 0 for unregistered
    /// slots. Set alongside m_handlers in register_handler().
    uint32_t            m_service_hash[kMaxSubAddresses];

    RtState       m_state;
    MessageHeader m_active_cmd_header;
    uint32_t      m_deadline_ms;
    uint32_t      m_cmd_rx_ms;    ///< Arrival time of the current command header.
    uint8_t       m_cmd_payload[kMaxPayloadBytes];
    uint8_t       m_resp_payload[kMaxPayloadBytes];
    uint32_t      m_resp_len;
};


#endif // SCHEDULER_H
