# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

# CMake build for the C++ unit tests (Catch2 v3 amalgamated).
#
# Portable across Linux, macOS, and Windows (MSVC or MinGW) with no Unix-shell
# dependency. Configure + build + run with the bundled Ninja presets:
#
#   cmake --preset Debug
#   cmake --build --preset Debug
#   ctest --preset Debug --output-on-failure
#
# Or without presets:
#
#   cmake -S . -B build -G Ninja
#   cmake --build build
#   ctest --test-dir build --output-on-failure

cmake_minimum_required(VERSION 3.15)
project(exhibition_cpp_tests CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

enable_testing()

# Parent C++ tree (../<path>.cpp) and the BC host sources (../../cpp_bc/host).
set(CPP "${CMAKE_CURRENT_SOURCE_DIR}/..")
set(BC_HOST "${CMAKE_CURRENT_SOURCE_DIR}/../../cpp_bc/host")

# Catch2 amalgamated translation unit, built once and linked into every test.
add_library(catch2 STATIC thirdparty/catch_amalgamated.cpp)
target_include_directories(catch2 PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}")

# Define one test binary. First argument is the name (and its <name>.cpp TU);
# the rest are the extra sources to link. Mirrors the makefile's *_OBJS lists.
# The include search path (this dir + the parent cpp dir) reproduces the
# makefile's `-I. -I..`, so `thirdparty/...` and `../<path>` includes resolve
# the same way -- including the BC host serial transport's `../communication`
# includes, which fall through to ../communication (cpp/communication).
function(exhibition_test name)
    add_executable(${name} ${name}.cpp ${ARGN})
    target_include_directories(${name} PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}"
        "${CMAKE_CURRENT_SOURCE_DIR}/..")
    target_link_libraries(${name} PRIVATE catch2)
    add_test(NAME ${name} COMMAND ${name})
endfunction()

exhibition_test(test_crcs
    "${CPP}/common/crc.cpp")

exhibition_test(test_message_header
    "${CPP}/common/crc.cpp"
    "${CPP}/communication/message_header.cpp")

exhibition_test(test_scheduler
    "${CPP}/common/crc.cpp"
    "${CPP}/communication/message_header.cpp"
    "${CPP}/communication/scheduler.cpp")

# The host serial transport lives outside this tree (../../cpp_bc/host); the
# USB-CDC transport comes from ../transports.
exhibition_test(test_transports
    "${CPP}/common/crc.cpp"
    "${CPP}/communication/message_header.cpp"
    "${CPP}/transports/usb_cdc_frame_transport.cpp"
    "${BC_HOST}/serial_frame_transport.cpp")

exhibition_test(test_helper_functions
    "${CPP}/common/helper_functions.cpp")

exhibition_test(test_sync
    "${CPP}/communication/sync.cpp"
    "${CPP}/common/helper_functions.cpp")

# End-to-end sync test: drives a real BusController <-> RemoteTerminal pair with
# the real SystemHandler, so it links the scheduler, system handler, and sync.
exhibition_test(test_sync_e2e
    "${CPP}/communication/sync.cpp"
    "${CPP}/communication/scheduler.cpp"
    "${CPP}/communication/message_header.cpp"
    "${CPP}/system/system_handler.cpp"
    "${CPP}/common/helper_functions.cpp"
    "${CPP}/common/crc.cpp")

# The clock filter is header-only; the test TU links sync.cpp only for the
# offset_pack_micro / offset_unpack_micro helpers it round-trips through.
exhibition_test(test_clock_filter
    "${CPP}/communication/sync.cpp"
    "${CPP}/common/helper_functions.cpp")

exhibition_test(test_adc_handler
    "${CPP}/devices/adc/adc_handler.cpp"
    "${CPP}/common/helper_functions.cpp"
    "${CPP}/communication/message_header.cpp"
    "${CPP}/common/crc.cpp")
