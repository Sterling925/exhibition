// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef MESSAGE_HEADER_H
#define MESSAGE_HEADER_H

#include <stdint.h>


/// @brief Compile-time selector for the CRC-16/MODBUS implementation used by
///        MessageHeader for its header CRC.
/// @details
/// Define MESSAGE_HEADER_USE_LOOKUP_CRC on the compiler command line
/// (e.g. -DMESSAGE_HEADER_USE_LOOKUP_CRC=0) before including this header to
/// override the default.
///   1 (default) -> calc_crc16_lt (lookup table, faster, ~512 B of tables)
///   0           -> calc_crc16    (bit-shift, smaller footprint)
/// Both implementations produce identical CRC values; this is a pure
/// speed/size trade-off.
#ifndef MESSAGE_HEADER_USE_LOOKUP_CRC
#define MESSAGE_HEADER_USE_LOOKUP_CRC 1
#endif

/// @brief `true` if MessageHeader uses the lookup-table CRC, else `false`.
constexpr bool kMessageHeaderUseLookupCrc = (MESSAGE_HEADER_USE_LOOKUP_CRC != 0);


/// @brief Supported payload sizes in bytes
enum class PayloadSizes : uint8_t {
    size_0    = 0x00,
    size_2    = 0x01,
    size_4    = 0x02,
    size_6    = 0x03,
    size_8    = 0x04,
    size_10   = 0x05,
    size_12   = 0x06,
    size_14   = 0x07,
    size_16   = 0x10,
    size_32   = 0x11,
    size_64   = 0x12,
    size_128  = 0x13,
    size_256  = 0x14,
    size_512  = 0x15,
    size_1024 = 0x16,
    size_2048 = 0x17,
};

enum class MessagePriorities : uint8_t  {
    Critical = 0x00,
    High = 0x01,
    Medium = 0x02,
    Low = 0x03,
};

/// @brief 6-byte packed Protocol T1 frame header. CRC-protected. Carried at
///        the start of every Protocol T1 frame on the wire.
class MessageHeader {
    private:

    /// @brief Protocol T1 header byte layout (6 bytes, packed)
    /// @details
    /// 2 bit priority, 0 is highest priority, 3 is lowest priority
    /// 1 bit command/response, 0 = response, 1 = command
    /// 1 bit large frame /// 0 small, 1 large
    /// 3 bit transmitted size // small sizes 0,2,4,6,8,10,12,14 // large sizes 16, 32, 64, 128, 256, 512, 1024, 2048
    /// 1 bit Communication Fault detected, 0 is fault, 1 is no fault payload will provide details, will cause clearing of bus states in RT if sent by BC
    /// 8 bit sub address // Value 0 is reserved
    /// 16 bit payload CRC
    /// 16 bit header CRC
    uint8_t m_header[6];


    public:

    MessageHeader();
    ~MessageHeader() = default;

    ///////////////////////////////////////////////////////////////////////////////////
    // setters

    /// @brief Set priority
    /// @param p Message priority to use
    /// @details
    /// Note encoding of `MessagePriorities` matches bit setting of first two bits.
    /// Thus could be type cast and shifted into place.
    void set_priority(MessagePriorities p);

    /// @brief Mark if this message is a command or a response
    /// @param f `true` if command, `false` if response
    void set_command_flag(bool f);

    /// @brief Set payload size to one of the supported values
    /// @param s Size to use
    /// @details 
    /// Sizes are a sparse definition, padding may be required for some use cases.
    /// The size enum will be mapped to bit mask in the following way
    /// if `s > size_14`  `large frame` = 1 else 0. AKA `s & 0x10`
    /// `transmitted size` = `s & 0x07`
    void set_size(PayloadSizes s);

    /// @brief Set flag to mark that communication fault has been detected
    /// @param fault `true` if fault, else `false`
    /// @details
    /// Bus controller setting this will clear buffers in remote terminal.
    /// Remote terminal setting this will trigger errro handing by Bus Controller
    void set_fault_flag(bool fault);

    /// @brief Set sub address of device
    /// @param addr The assigned sub address for the given device
    /// @details
    /// Used to route traffic to sub component of a given device.
    /// For example a specific motor axis on a board controlling multiple motors.
    /// The goal is to mapy a given payload to a specific handler that know hso to parse it.
    void set_sub_address(uint8_t addr);

    /// @brief Set the CRC16 of the payload that will go with this header
    /// @param crc CRC16 for payload size > 0, else zero
    void set_payload_crc(uint16_t crc);

    /// @brief Calculate CRC16 of header and place in header CRC location
    void update_header_crc();

    /// @brief Load this message header from serialized raw data
    /// @param buffer The buffer holding the serialized data, of length 6 or greater
    /// @return true if header CRC passes, else false
    bool deserialize(uint8_t const*const buffer);

    ///////////////////////////////////////////////////////////////////////////////////
    // getters

    /// @brief Get priority as enum type
    MessagePriorities get_priority() const;

    /// @brief Is this a command or a response
    /// @return `true` if command `false` if response
    bool get_command_flag() const;

    /// @brief Get size as assigned enum
    /// @return Assigned size enum value
    PayloadSizes get_size() const;

    /// @brief Get message payload size as byte qty
    /// @return Message payload size in bytes
    /// @note This is still the sparse definition, padding may be required in some payloads
    uint32_t get_size_bytes() const;

    /// @brief Check if the fault flag has been set in this header
    /// @return `true` if detected fault, else `false`
    bool get_fault_flag() const;

    /// @brief Get byte that includes priority data for sorting
    /// @return Priority byte (Lower value is higher priority)
    /// @details
    /// The priority byte indicates priority where lower values are higher priority.
    /// 0x00 is the highest priority, 0xFF would be the lowest priority.
    uint8_t get_priority_byte() const;

    /// @brief  Get sub address of specified component
    /// @return Assigned address
    /// @details
    /// On a command this is the address to route the command to.
    /// On a response, this is the sub component the message is coming from. 
    uint8_t get_sub_address() const;

    /// @brief Get CRC16 of payload data
    /// @return CRC16 value
    uint16_t get_payload_crc() const;

    /// @brief Get CRC16 of header
    /// @return CRC16 value
    uint16_t get_header_crc() const;

    /// @brief Check if header data matches header CRC
    /// @return `true` if CRC16 matches, else false
    bool check_header_crc() const;

    /// @brief Copy serialized header into buffer
    /// @param buffer Location to place six byte header
    /// @param buffer_max_size Max size of buffer (6 or greater)
    /// @return true if ok, false if header CRC sanity check fails
    bool serialize(uint8_t *const buffer, uint32_t buffer_max_size) const;
    
};


#endif // MESSAGE_HEADER_H