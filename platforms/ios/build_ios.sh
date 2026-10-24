#!/usr/bin/env bash
#
# build_ios.sh — cross-compile the pure-Rust runtime and (on macOS) build the
# unsigned Eden.app for iOS 12.0–14.4 device deployment.
#
# Usage:
#   ./build_ios.sh                 # release staticlib + xcodebuild
#   ./build_ios.sh --cargo-only    # staticlib only (works on Linux)
#   ./build_ios.sh --debug         # debug staticlib + Debug configuration
#
# Environment:
#   IPHONEOS_DEPLOYMENT_TARGET (default 12.0)
#   CODE_SIGN_IDENTITY         (default "-", ad-hoc; use ldid/TrollStore later)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET="aarch64-apple-ios"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-12.0}"
PROFILE="release"
CARGO_FLAGS="--release"
CONFIGURATION="Release"
CARGO_ONLY=0

for arg in "$@"; do
    case "$arg" in
        --cargo-only) CARGO_ONLY=1 ;;
        --debug) PROFILE="debug"; CARGO_FLAGS=""; CONFIGURATION="Debug" ;;
        -h|--help)
            sed -n '2,20p' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

echo "==> Rust target ${TARGET} (iOS ${IPHONEOS_DEPLOYMENT_TARGET})"
if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed | grep -qx "${TARGET}"; then
        rustup target add "${TARGET}"
    fi
fi

echo "==> cargo build -p eden-ffi ${CARGO_FLAGS} --target ${TARGET}"
# Intentionally unquoted: empty in --debug, one word in release.
cargo build -p eden-ffi ${CARGO_FLAGS} --target "${TARGET}" --manifest-path "${ROOT}/Cargo.toml"

LIB_SRC="${ROOT}/target/${TARGET}/${PROFILE}/libeden_ffi.a"
if [[ ! -f "${LIB_SRC}" ]]; then
    echo "missing ${LIB_SRC}" >&2
    exit 1
fi

mkdir -p "${SCRIPT_DIR}/EdenApp/lib" "${SCRIPT_DIR}/EdenApp/include"
cp "${LIB_SRC}" "${SCRIPT_DIR}/EdenApp/lib/libeden_ffi.a"
cp "${ROOT}/crates/eden-ffi/include/eden_ffi.h" "${SCRIPT_DIR}/EdenApp/include/eden_ffi.h"
echo "==> libeden_ffi.a $(du -h "${SCRIPT_DIR}/EdenApp/lib/libeden_ffi.a" | cut -f1)"
echo "==> eden_ffi.h synced from crates/eden-ffi/include"

if [[ "${CARGO_ONLY}" -eq 1 ]]; then
    exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "==> Not macOS: staticlib is ready, run xcodebuild on a Mac."
    exit 0
fi

if ! command -v xcodebuild >/dev/null 2>&1; then
    echo "xcodebuild not found; install Xcode 12.5 or newer." >&2
    exit 1
fi

DERIVED="${SCRIPT_DIR}/build"
EXTRA_SETTINGS=()
if [[ -f "${SCRIPT_DIR}/ffmpeg-ios/lib/libavcodec.a" ]]; then
    echo "==> linking FFmpeg iOS (libavcodec/libavformat/libswscale)"
    EXTRA_SETTINGS=(
        "HEADER_SEARCH_PATHS=${SCRIPT_DIR}/EdenApp/include ${SCRIPT_DIR}/ffmpeg-ios/include"
        "LIBRARY_SEARCH_PATHS=${SCRIPT_DIR}/EdenApp/lib ${SCRIPT_DIR}/ffmpeg-ios/lib"
        "OTHER_LDFLAGS=-lavformat -lavcodec -lswscale -lavutil"
    )
fi

echo "==> xcodebuild (${CONFIGURATION})"
if [[ ${#EXTRA_SETTINGS[@]} -gt 0 ]]; then
    xcodebuild \
        -project "${SCRIPT_DIR}/Eden.xcodeproj" \
        -scheme Eden \
        -configuration "${CONFIGURATION}" \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        -derivedDataPath "${DERIVED}" \
        IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET}" \
        CODE_SIGN_IDENTITY="${CODE_SIGN_IDENTITY:--}" \
        CODE_SIGNING_REQUIRED=NO \
        CODE_SIGNING_ALLOWED=NO \
        "${EXTRA_SETTINGS[@]}" \
        build
else
    xcodebuild \
        -project "${SCRIPT_DIR}/Eden.xcodeproj" \
        -scheme Eden \
        -configuration "${CONFIGURATION}" \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        -derivedDataPath "${DERIVED}" \
        IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET}" \
        CODE_SIGN_IDENTITY="${CODE_SIGN_IDENTITY:--}" \
        CODE_SIGNING_REQUIRED=NO \
        CODE_SIGNING_ALLOWED=NO \
        build
fi

APP="${DERIVED}/Build/Products/${CONFIGURATION}-iphoneos/Eden.app"
echo "==> built ${APP}"

# Bundle the CJK font so text never falls back to tofu boxes.
if [[ -f "${SCRIPT_DIR}/EdenApp/font.ttf" ]]; then
    cp "${SCRIPT_DIR}/EdenApp/font.ttf" "${APP}/font.ttf"
    echo "==> bundled font.ttf ($(du -h "${APP}/font.ttf" | cut -f1))"
fi

# Optional: embed retail data for a self-contained IPA.
if [[ -d "${SCRIPT_DIR}/EdenApp/gamedata" ]]; then
    rm -rf "${APP}/gamedata"
    cp -R "${SCRIPT_DIR}/EdenApp/gamedata" "${APP}/gamedata"
    echo "==> embedded gamedata ($(du -sh "${APP}/gamedata" | cut -f1))"
fi

if command -v ldid >/dev/null 2>&1; then
    ldid -S "${APP}/Eden"
    echo "==> ad-hoc signed with ldid (TrollStore ready)"
fi
