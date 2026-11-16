#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
OUT_DIR="${OUT_DIR:-$HERE/bin}"

[[ -n "$NDK" ]] || {
    echo "Set ANDROID_NDK_HOME to your Android NDK directory (tested with r29)." >&2
    exit 1
}
case "$(uname -s)" in
    Linux) host=linux-x86_64 ;;
    Darwin) host=darwin-x86_64 ;;
    *) echo "Build on Linux or macOS with the Android NDK." >&2; exit 1 ;;
esac
CLANG="$NDK/toolchains/llvm/prebuilt/$host/bin/clang"
[[ -x "$CLANG" ]] || { echo "Android NDK compiler not found: $CLANG" >&2; exit 1; }

mkdir -p "$OUT_DIR"
common=(--target=aarch64-linux-android28 -std=c11 -Wall -Wextra "-Wl,-z,max-page-size=16384")
"$CLANG" "${common[@]}" -O3 -fPIC -shared "$HERE/streamer.c" \
    -o "$OUT_DIR/libquestpro-camera-streamer-v8.so"
"$CLANG" "${common[@]}" -O3 -fPIE -pie -DSTREAM_PORT=27273 "$HERE/relay.c" \
    -o "$OUT_DIR/questpro-camera-relay-v8"
"$CLANG" "${common[@]}" -O2 -fPIE -pie "$HERE/injector.c" \
    -o "$OUT_DIR/questpro-camera-injector" -ldl
echo "Built Quest Pro headset binaries in $OUT_DIR"
