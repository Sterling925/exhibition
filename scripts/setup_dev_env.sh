#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: 2026 https://github.com/Sterling925
#
# Set up an Ubuntu host for Exhibition development:
#   - host C/C++ build tools (g++, make, CMake >= 3.22, Ninja) for the BC app
#     and the Catch2 unit tests
#   - the arm-none-eabi GCC cross toolchain + newlib + libstdc++ needed to
#     compile the RT firmware (example/rt, cmake/gcc-arm-none-eabi.cmake)
#   - stlink-tools (st-flash / st-info / st-util) and gdb-multiarch to flash
#     and debug ST boards over an ST-LINK USB probe
#   - Rust (via rustup) to build the Exhibition generator itself
#   - udev rules + dialout/plugdev group membership for non-root access to the
#     ST-LINK probe and the board's serial port
#
# Only tools that are not already present are installed; an existing CMake
# (e.g. from snap) is kept as long as it is new enough.
#
# Usage: scripts/setup_dev_env.sh [--dry-run] [--no-smoke-test] [--no-rust] [-h]
#
# Run as your normal user; sudo is invoked for the steps that need it.

set -euo pipefail

readonly CMAKE_MIN_VERSION="3.22"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly RT_EXAMPLE_DIR="${REPO_ROOT}/example/rt"

DRY_RUN=0
SMOKE_TEST=1
INSTALL_RUST=1

# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------
if [[ -t 1 ]]; then
    C_BLUE=$'\e[1;34m'; C_GREEN=$'\e[1;32m'; C_YELLOW=$'\e[1;33m'; C_RED=$'\e[1;31m'; C_OFF=$'\e[0m'
else
    C_BLUE=""; C_GREEN=""; C_YELLOW=""; C_RED=""; C_OFF=""
fi
step() { printf '\n%s==> %s%s\n' "${C_BLUE}" "$*" "${C_OFF}"; }
ok()   { printf '  %s[ok]%s   %s\n' "${C_GREEN}" "${C_OFF}" "$*"; }
need() { printf '  %s[need]%s %s\n' "${C_YELLOW}" "${C_OFF}" "$*"; }
warn() { printf '%sWARNING:%s %s\n' "${C_YELLOW}" "${C_OFF}" "$*" >&2; }
die()  { printf '%sERROR:%s %s\n' "${C_RED}" "${C_OFF}" "$*" >&2; exit 1; }

# Run a command, or just print it in --dry-run mode.
run() {
    if (( DRY_RUN )); then
        printf '  (dry-run) %s\n' "$*"
    else
        "$@"
    fi
}

usage() {
    sed -n '5,21p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    cat <<'EOF'

Options:
  --dry-run        Show what would be installed/changed without doing it
  --no-smoke-test  Skip the trial build of example/rt with the ARM toolchain
  --no-rust        Skip installing Rust via rustup
  -h, --help       Show this help
EOF
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while (( $# )); do
    case "$1" in
        --dry-run)       DRY_RUN=1 ;;
        --no-smoke-test) SMOKE_TEST=0 ;;
        --no-rust)       INSTALL_RUST=0 ;;
        -h|--help)       usage; exit 0 ;;
        *)               usage >&2; die "unknown option: $1" ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
[[ ${EUID} -ne 0 ]] || die "run this as your normal user, not root (sudo is used where needed,
       and rustup / group membership must target your account)."

[[ -r /etc/os-release ]] || die "/etc/os-release not found; this script supports Ubuntu only."
# shellcheck source=/dev/null
. /etc/os-release
if [[ "${ID:-}" != "ubuntu" && "${ID_LIKE:-}" != *ubuntu* ]]; then
    if [[ "${ID:-}" == "debian" || "${ID_LIKE:-}" == *debian* ]]; then
        warn "detected ${PRETTY_NAME:-unknown}; this script targets Ubuntu, continuing anyway."
    else
        die "detected ${PRETTY_NAME:-unknown}; this script supports Ubuntu (apt) only."
    fi
fi
command -v apt-get >/dev/null || die "apt-get not found."
if (( ! DRY_RUN )); then
    command -v sudo >/dev/null || die "sudo not found."
fi

# ---------------------------------------------------------------------------
# Detection helpers
# ---------------------------------------------------------------------------
have_cmd() { command -v "$1" >/dev/null 2>&1; }

pkg_installed() {
    [[ "$(dpkg-query -W -f='${Status}' "$1" 2>/dev/null)" == "install ok installed" ]]
}

# version_ge A B -> true if version A >= version B
version_ge() {
    [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

# The ARM C++ runtime is usable if the cross compiler can find libstdc++.a and
# newlib-nano's specs (the RT links with --specs=nano.specs).
arm_cxx_runtime_ok() {
    have_cmd arm-none-eabi-g++ || return 1
    local f
    f="$(arm-none-eabi-g++ -mcpu=cortex-m4 -mfpu=fpv4-sp-d16 -mfloat-abi=hard \
            -print-file-name=libstdc++.a 2>/dev/null)"
    [[ "${f}" == /* && -f "${f}" ]] || return 1
    f="$(arm-none-eabi-gcc -print-file-name=nano.specs 2>/dev/null)"
    [[ "${f}" == /* && -f "${f}" ]]
}

APT_PACKAGES=()
want_pkg() { APT_PACKAGES+=("$1"); }

# check_cmd <command> <apt-package> <description>
check_cmd() {
    if have_cmd "$1"; then
        ok "$3 ($(command -v "$1"))"
    else
        need "$3 -> apt: $2"
        want_pkg "$2"
    fi
}

# ---------------------------------------------------------------------------
# 1. Survey the host
# ---------------------------------------------------------------------------
step "Checking host tools on ${PRETTY_NAME:-Ubuntu} ($(uname -m))"

# Host build essentials
if have_cmd gcc && have_cmd g++ && have_cmd make; then
    ok "gcc/g++/make ($(g++ -dumpfullversion 2>/dev/null || g++ -dumpversion))"
else
    need "gcc/g++/make -> apt: build-essential"
    want_pkg build-essential
fi
check_cmd pkg-config pkg-config "pkg-config"
check_cmd git        git        "git"
check_cmd curl       curl       "curl"
check_cmd ninja      ninja-build "Ninja"

# CMake: keep any existing install (snap, Kitware apt repo, ...) if new enough.
if have_cmd cmake; then
    cmake_ver="$(cmake --version | head -n1 | awk '{print $3}')"
    if version_ge "${cmake_ver}" "${CMAKE_MIN_VERSION}"; then
        ok "CMake ${cmake_ver} ($(command -v cmake))"
    else
        need "CMake ${cmake_ver} is older than ${CMAKE_MIN_VERSION} -> apt: cmake"
        want_pkg cmake
        [[ "$(command -v cmake)" == /usr/bin/cmake ]] \
            || warn "the old cmake at $(command -v cmake) may shadow the apt one on PATH."
    fi
else
    need "CMake -> apt: cmake"
    want_pkg cmake
fi

# ARM cross toolchain for the RT firmware
check_cmd arm-none-eabi-gcc     gcc-arm-none-eabi     "arm-none-eabi-gcc/g++ (RT cross compiler)"
check_cmd arm-none-eabi-objcopy binutils-arm-none-eabi "arm-none-eabi binutils (objcopy/size)"
if arm_cxx_runtime_ok; then
    ok "ARM newlib + libstdc++ (C++ runtime for the RT)"
else
    pkg_installed libnewlib-arm-none-eabi        || { need "ARM newlib -> apt: libnewlib-arm-none-eabi"; want_pkg libnewlib-arm-none-eabi; }
    pkg_installed libstdc++-arm-none-eabi-newlib || { need "ARM libstdc++ -> apt: libstdc++-arm-none-eabi-newlib"; want_pkg libstdc++-arm-none-eabi-newlib; }
fi

# Flashing / debugging over ST-LINK
check_cmd st-flash      stlink-tools  "stlink-tools (st-flash/st-info/st-util + ST-LINK udev rules)"
check_cmd gdb-multiarch gdb-multiarch "gdb-multiarch (ARM debugger)"

# Rust for the Exhibition generator
if (( INSTALL_RUST )); then
    if have_cmd cargo || [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
        ok "Rust/cargo ($(command -v cargo || echo "${HOME}/.cargo/bin/cargo"))"
        NEED_RUST=0
    else
        need "Rust -> rustup (stable)"
        NEED_RUST=1
    fi
else
    NEED_RUST=0
fi

# Group membership: dialout for /dev/ttyACM* / ttyUSB* (BC <-> RT serial
# link), plugdev for the ST-LINK udev rules shipped by stlink-tools.
NEED_GROUPS=()
for g in dialout plugdev; do
    if id -nG "${USER}" | tr ' ' '\n' | grep -qx "${g}"; then
        ok "user ${USER} is in group ${g}"
    else
        need "add user ${USER} to group ${g}"
        NEED_GROUPS+=("${g}")
    fi
done

# ---------------------------------------------------------------------------
# 2. Install missing apt packages
# ---------------------------------------------------------------------------
step "Installing apt packages"
if (( ${#APT_PACKAGES[@]} )); then
    printf '  %s\n' "${APT_PACKAGES[@]}"
    run sudo apt-get update
    run sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${APT_PACKAGES[@]}"
else
    ok "nothing to install"
fi

# ---------------------------------------------------------------------------
# 3. Rust
# ---------------------------------------------------------------------------
if (( NEED_RUST )); then
    step "Installing Rust (rustup, stable toolchain)"
    if (( DRY_RUN )); then
        printf '  (dry-run) curl https://sh.rustup.rs | sh -s -- -y --profile default\n'
    else
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile default
        # shellcheck source=/dev/null
        . "${HOME}/.cargo/env"
    fi
fi

# ---------------------------------------------------------------------------
# 4. udev + groups
# ---------------------------------------------------------------------------
step "Configuring USB/serial device access"
for g in "${NEED_GROUPS[@]}"; do
    run sudo usermod -aG "${g}" "${USER}"
done
if [[ " ${APT_PACKAGES[*]} " == *" stlink-tools "* ]]; then
    run sudo udevadm control --reload-rules
    run sudo udevadm trigger
    ok "udev rules reloaded (unplug/replug the ST-LINK if it is connected)"
else
    ok "udev rules unchanged"
fi

# ---------------------------------------------------------------------------
# 5. Smoke test: build the example RT firmware
# ---------------------------------------------------------------------------
if (( SMOKE_TEST )); then
    step "Smoke test: building ${RT_EXAMPLE_DIR#"${REPO_ROOT}"/} with arm-none-eabi-gcc (Debug)"
    if (( DRY_RUN )); then
        printf '  (dry-run) cmake -S example/rt -B <tmp> -G Ninja -DCMAKE_TOOLCHAIN_FILE=cmake/gcc-arm-none-eabi.cmake -DCMAKE_BUILD_TYPE=Debug\n'
        printf '  (dry-run) cmake --build <tmp>\n'
    elif [[ ! -f "${RT_EXAMPLE_DIR}/CMakeLists.txt" ]]; then
        warn "${RT_EXAMPLE_DIR} not found; skipping smoke test."
    else
        build_dir="$(mktemp -d -t exhibition-rt-smoke.XXXXXX)"
        trap 'rm -rf "${build_dir}"' EXIT
        log="${build_dir}/build.log"
        if cmake -S "${RT_EXAMPLE_DIR}" -B "${build_dir}" -G Ninja \
                -DCMAKE_TOOLCHAIN_FILE="${RT_EXAMPLE_DIR}/cmake/gcc-arm-none-eabi.cmake" \
                -DCMAKE_BUILD_TYPE=Debug >"${log}" 2>&1 \
           && cmake --build "${build_dir}" >>"${log}" 2>&1; then
            elf="$(find "${build_dir}" -maxdepth 1 -name '*.elf' | head -n1)"
            ok "built $(basename "${elf}")"
            arm-none-eabi-size "${elf}" | sed 's/^/         /'
        else
            tail -n 40 "${log}" >&2
            die "RT smoke-test build failed (last 40 lines above)."
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
step "Done"
if (( DRY_RUN )); then
    echo "  Dry run only; nothing was changed."
fi
if (( ${#NEED_GROUPS[@]} )); then
    echo "  * Log out and back in (or reboot) so the new group membership (${NEED_GROUPS[*]}) applies."
fi
if (( NEED_RUST )); then
    echo "  * Open a new shell (or run: . \"\$HOME/.cargo/env\") to get cargo on PATH."
fi
cat <<'EOF'
  * Build the RT:   cd example/rt && cmake --preset Debug && cmake --build --preset Debug
  * Probe check:    st-info --probe
  * Flash the RT:   st-flash --reset write build/Debug/<project>.bin 0x08000000
                    (make the .bin with: arm-none-eabi-objcopy -O binary <project>.elf <project>.bin)
  * Debug:          st-util   # GDB server on :4242, then: gdb-multiarch <project>.elf -ex 'target extended-remote :4242'
EOF
