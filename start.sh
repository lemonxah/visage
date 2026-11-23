#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

VISAGE="${VISAGE:-./target/release/visage}"
FT_MODE="${FT_MODE:-auto}"
INDEPENDENT_EYES="${INDEPENDENT_EYES:-1}"
GUI="${GUI:-1}"
HEADSET_STREAM_SCRIPT="${HEADSET_STREAM_SCRIPT:-./headset-stream.sh}"
ADB="${ADB:-adb}"

# Pretrained developer model from the verified Qpro Companion v3.0.3 release.
TONGUE_ENABLED="${TONGUE_ENABLED:-1}"
BABBLE_ENABLED="${BABBLE_ENABLED:-0}"
BABBLE_PORT="${BABBLE_PORT:-9400}"
TONGUE_MODEL_DIR="${TONGUE_MODEL_DIR:-./models/qpro-companion-v3.0.3}"
TONGUE_MODEL="${TONGUE_MODEL:-$TONGUE_MODEL_DIR/qpro-stereo-tongue-v15-gate.onnx}"
TONGUE_DIRECTION_MODEL="${TONGUE_DIRECTION_MODEL-$TONGUE_MODEL_DIR/qpro-stereo-tongue-v15-direction.onnx}"
TONGUE_PORT="${TONGUE_PORT:-27273}"
HEADSET_TONGUE_PORT="${HEADSET_TONGUE_PORT:-27273}"
TONGUE_SMOOTHING="${TONGUE_SMOOTHING:-55}"
TONGUE_VISIBILITY="${TONGUE_VISIBILITY:-weighted}"
# With v15's camera weight 0.6 and a zero native score, 0.575 requires
# camera confidence of about 0.958 while staying below the fused ceiling of 0.6.
TONGUE_THRESHOLD="${TONGUE_THRESHOLD-0.575}"
TONGUE_DEBUG="${TONGUE_DEBUG:-0}"
TONGUE_THREADS="${TONGUE_THREADS:-2}"

WIVRN_PACKAGE="${WIVRN_PACKAGE:-org.meumeu.wivrn}"
LAUNCH_WIVRN="${LAUNCH_WIVRN:-1}"

STREAMER_ACTIVITY="${STREAMER_ACTIVITY:-}"
STREAMER_SHELL="${STREAMER_SHELL:-}"
STREAMER_NEEDS_ROOT="${STREAMER_NEEDS_ROOT:-1}"
streamer_managed=0
forward_managed=0
gui_relay=0
osc_pid=""

say() { printf '\033[1;32m[start]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[start]\033[0m %s\n' "$*" >&2; }
die() {
  printf '\033[1;31m[start]\033[0m %s\n' "$*" >&2
  exit 1
}

preflight() {
  [ -x "$VISAGE" ] || die "visage not found or not executable at $VISAGE"
  [[ "$INDEPENDENT_EYES" == 0 || "$INDEPENDENT_EYES" == 1 ]] || die "INDEPENDENT_EYES must be 0 or 1."
  [[ "$GUI" == 0 || "$GUI" == 1 ]] || die "GUI must be 0 or 1."
  if [ "$GUI" = "1" ] && ! "$VISAGE" --help | grep -q -- '--gui'; then
    warn "This visage build has no window; rebuild with: cargo build --release --features qpro-tongue,gui,cameras"
    GUI=0
  fi
  [[ "$TONGUE_ENABLED" == 0 || "$TONGUE_ENABLED" == 1 ]] || die "TONGUE_ENABLED must be 0 or 1."
  [[ "$BABBLE_ENABLED" == 0 || "$BABBLE_ENABLED" == 1 ]] || die "BABBLE_ENABLED must be 0 or 1."
  if [ "$BABBLE_ENABLED" = "1" ]; then
    [ "$TONGUE_ENABLED" = "0" ] || die "Babble mouth and the camera tongue model cannot be used together. Set BABBLE_ENABLED=0 for camera model diagnostics."
    if ! [[ "$BABBLE_PORT" =~ ^[0-9]{1,5}$ ]]; then
      die "BABBLE_PORT must be between 1 and 65535."
    fi
    if ((10#$BABBLE_PORT < 1 || 10#$BABBLE_PORT > 65535)); then
      die "BABBLE_PORT must be between 1 and 65535."
    fi
    if ! "$VISAGE" openxr --help | grep -q -- '--babble'; then
      die "Rebuild combined eye/mouth support: cargo build --release --features openxr,babble"
    fi
  fi
  [ "$TONGUE_ENABLED" = "1" ] || return 0
  [ -f "$TONGUE_MODEL" ] || die "Tongue visibility model not found: $TONGUE_MODEL"
  if [ -n "$TONGUE_DIRECTION_MODEL" ]; then
    [ -f "$TONGUE_DIRECTION_MODEL" ] || die "Tongue direction model not found: $TONGUE_DIRECTION_MODEL"
  fi
  if ! "$VISAGE" openxr --help | grep -q -- '--tongue-model'; then
    die "Rebuild with tongue support: cargo build --release --features qpro-tongue"
  fi
  if [ -z "$STREAMER_ACTIVITY" ] && [ -z "$STREAMER_SHELL" ]; then
    [ -f "$HEADSET_STREAM_SCRIPT" ] || die "Headset stream script not found: $HEADSET_STREAM_SCRIPT"
    [ "$HEADSET_TONGUE_PORT" = "27273" ] || die "The bundled headset relay listens on port 27273."
    if [ "$GUI" = "1" ] && "$VISAGE" openxr --help | grep -q -- '--tongue-relay-script'; then
      gui_relay=1
    fi
  fi
}

headset_problem() {
  [ "$gui_relay" = "1" ] || die "$1"
  warn "$1"
  warn "Continuing without the camera relay; use Start headset cameras in the Tongue tab once the headset is ready."
}

require_device() {
  command -v "$ADB" >/dev/null 2>&1 || die "adb not found: $ADB"
  "$ADB" start-server >/dev/null 2>&1 || true
  if ! "$ADB" get-state >/dev/null 2>&1; then
    headset_problem "No adb device. Plug in the headset and allow USB debugging."
    return 1
  fi
  say "adb device: $("$ADB" get-serialno 2>/dev/null)"
}

require_headset_root() {
  local uid
  uid="$("$ADB" shell 'su -c "id -u"' 2>/dev/null | tr -d '\r')"
  if [ "$uid" != "0" ]; then
    headset_problem "Headset not rooted this session. Root with Singularity first, then re-run."
    return 1
  fi
  say "headset root: ok"
}

launch_wivrn() {
  [ "$LAUNCH_WIVRN" = "1" ] || {
    say "skipping WiVRn client launch (LAUNCH_WIVRN=0)"
    return
  }
  local activity
  activity="$("$ADB" shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER "$WIVRN_PACKAGE" 2>/dev/null | tr -d '\r' | tail -n1)"
  [[ "$activity" == */* ]] || die "WiVRn package '$WIVRN_PACKAGE' not found. Set WIVRN_PACKAGE."
  say "launching WiVRn client: $activity"
  "$ADB" shell am start -n "$activity" >/dev/null
}

start_streamer() {
  if [ -n "$STREAMER_ACTIVITY" ]; then
    say "starting camera streamer: $STREAMER_ACTIVITY"
    "$ADB" shell am start -n "$STREAMER_ACTIVITY" >/dev/null
  elif [ -n "$STREAMER_SHELL" ]; then
    say "starting camera streamer (shell)"
    if [ "$STREAMER_NEEDS_ROOT" = "1" ]; then
      "$ADB" shell "su -c 'nohup $STREAMER_SHELL >/dev/null 2>&1 &'" >/dev/null
    else
      "$ADB" shell "nohup $STREAMER_SHELL >/dev/null 2>&1 &" >/dev/null
    fi
  else
    say "starting Quest Pro mouth-camera relay"
    streamer_managed=1
    forward_managed=1
    PORT="$TONGUE_PORT" MODE=mouth bash "$HEADSET_STREAM_SCRIPT" start
  fi
}

setup_forward() {
  forward_managed=1
  "$ADB" forward --remove "tcp:${TONGUE_PORT}" >/dev/null 2>&1 || true
  "$ADB" forward "tcp:${TONGUE_PORT}" "tcp:${HEADSET_TONGUE_PORT}" >/dev/null
  say "forwarding localhost:${TONGUE_PORT} -> headset:${HEADSET_TONGUE_PORT}"
}

cleanup() {
  trap - EXIT INT TERM
  if [ -n "$osc_pid" ]; then
    kill "$osc_pid" 2>/dev/null || true
    wait "$osc_pid" 2>/dev/null || true
  fi
  if [ "$streamer_managed" = "1" ]; then
    say "stopping Quest Pro mouth-camera relay"
    PORT="$TONGUE_PORT" bash "$HEADSET_STREAM_SCRIPT" stop || warn "Could not stop headset relay."
  fi
  if [ "$forward_managed" = "1" ]; then
    "$ADB" forward --remove "tcp:${TONGUE_PORT}" >/dev/null 2>&1 || true
  fi
}

run_visage() {
  local args=(--ft-mode "$FT_MODE")
  if [ "$GUI" = "1" ]; then
    args+=(--gui)
  fi
  args+=(openxr)
  if [ "$INDEPENDENT_EYES" = "1" ]; then
    local independent_eye_arg=0 eye_arg
    for eye_arg in "$@"; do
      if [ "$eye_arg" = "--independent-eyes" ]; then
        independent_eye_arg=1
      fi
    done
    if [ "$independent_eye_arg" = "0" ]; then
      args+=(--independent-eyes)
    fi
  fi
  if [ "$BABBLE_ENABLED" = "1" ]; then
    local babble_arg=0 arg
    for arg in "$@"; do
      if [[ "$arg" == --babble || "$arg" == --babble=* ]]; then
        babble_arg=1
      fi
    done
    if [ "$babble_arg" = "0" ]; then
      args+=(--babble "$BABBLE_PORT")
    fi
    say "using WiVRn eyes and brows with Babble mouth data"
  fi
  if [ "$TONGUE_ENABLED" = "1" ]; then
    args+=(--tongue-model "$TONGUE_MODEL"
      --tongue-port "$TONGUE_PORT"
      --tongue-smoothing "$TONGUE_SMOOTHING"
      --tongue-visibility "$TONGUE_VISIBILITY"
      --tongue-threads "$TONGUE_THREADS")
    if [ -n "$TONGUE_DIRECTION_MODEL" ]; then
      args+=(--tongue-direction-model "$TONGUE_DIRECTION_MODEL")
    fi
    if [ -n "$TONGUE_THRESHOLD" ]; then
      args+=(--tongue-threshold "$TONGUE_THRESHOLD")
    fi
    if [ "$TONGUE_DEBUG" = "1" ]; then
      args+=(--tongue-debug)
    fi
    if [ "$gui_relay" = "1" ]; then
      args+=(--tongue-relay-script "$HEADSET_STREAM_SCRIPT")
    fi
  else
    say "camera tongue tracking is disabled (TONGUE_ENABLED=1 to enable)"
  fi

  say "starting visage ${args[*]}"
  "$VISAGE" "${args[@]}" "$@" &
  osc_pid=$!
  local result=0
  wait "$osc_pid" || result=$?
  osc_pid=""
  return "$result"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

preflight
if [ "$gui_relay" = "1" ]; then
  streamer_managed=1
  forward_managed=1
fi
headset=1
if [ "$TONGUE_ENABLED" = "1" ] || [ "$LAUNCH_WIVRN" = "1" ]; then
  require_device || headset=0
fi
if [ "$headset" = "1" ]; then
  launch_wivrn
fi
if [ "$TONGUE_ENABLED" = "1" ] && [ "$headset" = "1" ]; then
  if require_headset_root; then
    if [ "$gui_relay" = "1" ]; then
      if start_streamer; then
        setup_forward
      else
        headset_problem "Quest Pro mouth-camera relay did not start."
      fi
    else
      start_streamer
      setup_forward
    fi
  fi
fi
run_visage "$@"
