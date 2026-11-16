#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HERE/headset/bin"
ADB="${ADB:-adb}"
PORT="${PORT:-27273}"
MODE="${MODE:-mouth}"
MAX_FPS="${MAX_FPS:-24}"

REMOTE=/data/local/tmp
STREAMER=libquestpro-camera-streamer-v8.so
RELAY=questpro-camera-relay-v8
INJECTOR=questpro-camera-injector

require_root() {
    if ! "$ADB" shell su -c id 2>/dev/null | grep -q 'uid=0(root)'; then
        echo "Magisk root is not granted to ADB shell. On the headset: Magisk > Superuser > enable Shell, then retry." >&2
        exit 1
    fi
}

start() {
    local binary
    for binary in "$STREAMER" "$RELAY" "$INJECTOR"; do
        [[ -f "$BIN/$binary" ]] || {
            echo "Missing $BIN/$binary. Restore the bundled binaries or run headset/build.sh with ANDROID_NDK_HOME set." >&2
            exit 1
        }
    done
    if ! [[ "$PORT" =~ ^[0-9]{1,5}$ ]] || ((10#$PORT < 1 || 10#$PORT > 65535)); then
        echo "PORT must be between 1 and 65535." >&2
        exit 1
    fi
    case "$MODE" in
        all|eyes|face|mouth) ;;
        *) echo "MODE must be all, eyes, face, or mouth." >&2; exit 1 ;;
    esac
    if ! [[ "$MAX_FPS" =~ ^[0-9]{1,3}$ ]] || ((10#$MAX_FPS > 120)); then
        echo "MAX_FPS must be between 0 and 120." >&2
        exit 1
    fi
    "$ADB" get-state >/dev/null 2>&1 || { echo "No authorized headset over adb. Connect USB and accept the debugging prompt." >&2; exit 1; }
    require_root

    "$ADB" push "$BIN/$STREAMER" "$REMOTE/$STREAMER"
    "$ADB" push "$BIN/$RELAY" "$REMOTE/$RELAY"
    "$ADB" push "$BIN/$INJECTOR" "$REMOTE/$INJECTOR"

    "$ADB" shell chmod 755 "$REMOTE/$RELAY" "$REMOTE/$INJECTOR"
    "$ADB" shell chmod 644 "$REMOTE/$STREAMER"
    "$ADB" shell "rm -f $REMOTE/questpro-live-v3.log $REMOTE/questpro-live-v8.log; touch $REMOTE/questpro-live-v3.log $REMOTE/questpro-live-v8.log; chmod 666 $REMOTE/questpro-live-v3.log $REMOTE/questpro-live-v8.log"

    "$ADB" forward --remove "tcp:$PORT" >/dev/null 2>&1 || true

    # Roll back a partial start when this helper is used on its own, too.
    trap 'stop >/dev/null 2>&1 || true' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    "$ADB" shell su -c ": > $REMOTE/questpro-relay-v8.log; nohup $REMOTE/$RELAY --mode $MODE --max-fps $MAX_FPS > $REMOTE/questpro-relay-v8.log 2>&1 < /dev/null &"
    sleep 1
    if ! "$ADB" shell su -c "cat $REMOTE/questpro-relay-v8.log" | grep -q RELAY_LISTENING; then
        echo "Relay did not start:" >&2
        "$ADB" shell su -c "cat $REMOTE/questpro-relay-v8.log" >&2
        exit 1
    fi

    "$ADB" shell su -c "$REMOTE/$INJECTOR $REMOTE/$STREAMER"
    sleep 1
    # The bundled relay is built for headset port 27273. PORT selects only
    # the PC port, which may differ when another application uses the default.
    "$ADB" forward "tcp:$PORT" tcp:27273
    trap - EXIT INT TERM
    echo "Headset camera stream is on 127.0.0.1:$PORT (mode $MODE, cap $MAX_FPS fps)."
}

stop() {
    "$ADB" shell su -c "$REMOTE/$RELAY --stop" 2>/dev/null || true
    "$ADB" forward --remove "tcp:$PORT" >/dev/null 2>&1 || true
    echo "Stopped the relay and removed the port forward. Reboot the headset to clear the injected streamer."
}

logs() {
    echo "=== streamer ==="; "$ADB" shell su -c "cat $REMOTE/questpro-live-v8.log" 2>/dev/null || true
    echo "=== relay ==="; "$ADB" shell su -c "cat $REMOTE/questpro-relay-v8.log" 2>/dev/null || true
}

case "${1:-start}" in
    start) start ;;
    stop) stop ;;
    logs) logs ;;
    *) echo "Usage: $0 [start|stop|logs]" >&2; exit 2 ;;
esac
