#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

QPRO_DIR="${QPRO_DIR:-../qpro-ft}"
HEADSET_STREAM_SCRIPT="${HEADSET_STREAM_SCRIPT:-$QPRO_DIR/headset-stream.sh}"
TONGUE_PORT="${TONGUE_PORT:-27273}"

if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
    exec python3 contrib/qpro_tongue_capture.py --help
fi

command -v python3 >/dev/null || { echo "python3 is required." >&2; exit 1; }
[[ -f "$HEADSET_STREAM_SCRIPT" ]] || { echo "Missing $HEADSET_STREAM_SCRIPT" >&2; exit 1; }
echo "Close OscAvMgr and other camera viewers before capture. Keep WiVRn face tracking enabled."

cleanup() {
    trap - EXIT INT TERM
    PORT="$TONGUE_PORT" bash "$HEADSET_STREAM_SCRIPT" stop || true
    adb forward --remove "tcp:$TONGUE_PORT" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

PORT="$TONGUE_PORT" MODE=mouth MAX_FPS=12 bash "$HEADSET_STREAM_SCRIPT" start
adb forward "tcp:$TONGUE_PORT" tcp:27273 >/dev/null
python3 contrib/qpro_tongue_capture.py \
    --port "$TONGUE_PORT" \
    --output "captures/qpro-calibration/$(date +%Y%m%d-%H%M%S)" "$@"
