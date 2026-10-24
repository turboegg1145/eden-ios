#!/usr/bin/env bash
#
# build_ffmpeg_ios.sh — cross-compile the minimal libavcodec/libavformat set
# used by EdenVideo.m for the retail WMV3 / MPEG-4 ASP movies.
#
# Runs on macOS with Xcode installed (the GitHub macos-14 runner). The result
# is a static arm64 iPhoneOS library tree in platforms/ios/ffmpeg-ios/.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="${SCRIPT_DIR}/ffmpeg-ios"
BUILD="${SCRIPT_DIR}/build/ffmpeg"
VERSION="${FFMPEG_VERSION:-6.1.1}"
MIN_IOS="${IPHONEOS_DEPLOYMENT_TARGET:-12.0}"

if [[ -f "${PREFIX}/lib/libavcodec.a" && -f "${PREFIX}/lib/libavformat.a" ]]; then
    echo "==> FFmpeg iOS libraries already present in ${PREFIX}"
    exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "build_ffmpeg_ios.sh requires macOS + Xcode" >&2
    exit 1
fi

mkdir -p "${BUILD}"
cd "${BUILD}"
ARCHIVE="ffmpeg-${VERSION}.tar.xz"
if [[ ! -f "${ARCHIVE}" ]]; then
    echo "==> downloading ${ARCHIVE}"
    curl -fL --retry 3 -o "${ARCHIVE}" "https://ffmpeg.org/releases/${ARCHIVE}"
fi
if [[ ! -d "ffmpeg-${VERSION}" ]]; then
    tar -xf "${ARCHIVE}"
fi

SDK="$(xcrun --sdk iphoneos --show-sdk-path)"
CC="$(xcrun --sdk iphoneos -f clang)"

cd "ffmpeg-${VERSION}"
echo "==> configuring FFmpeg ${VERSION} for arm64 iPhoneOS ${MIN_IOS}"
./configure \
    --prefix="${PREFIX}" \
    --enable-cross-compile \
    --arch=arm64 \
    --target-os=darwin \
    --cc="${CC}" \
    --sysroot="${SDK}" \
    --extra-cflags="-arch arm64 -miphoneos-version-min=${MIN_IOS}" \
    --extra-ldflags="-arch arm64 -miphoneos-version-min=${MIN_IOS}" \
    --disable-programs \
    --disable-doc \
    --disable-network \
    --disable-avdevice \
    --disable-avfilter \
    --disable-postproc \
    --disable-zlib \
    --disable-bzlib \
    --disable-lzma \
    --disable-iconv \
    --disable-audiotoolbox \
    --disable-videotoolbox \
    --enable-static \
    --disable-shared \
    --enable-pic \
    --disable-everything \
    --enable-decoder=wmv3,mpeg4,mp3 \
    --enable-demuxer=avi,mp3 \
    --enable-parser=mpeg4video,mpegaudio \
    --enable-protocol=file \
    --enable-swscale

echo "==> building FFmpeg"
make -j"$(sysctl -n hw.ncpu)"
make install

echo "==> FFmpeg iOS libs:"
ls -lh "${PREFIX}/lib"/libavcodec.a "${PREFIX}/lib"/libavformat.a \
    "${PREFIX}/lib"/libavutil.a "${PREFIX}/lib"/libswscale.a
