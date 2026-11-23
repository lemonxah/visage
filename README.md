# Visage

Linux eye, face and tongue tracking for VRChat and Resonite, with a live
control window and in-process camera models for the Quest Pro.

Visage is a fork of [OSC Avatar Manager (oscavmgr)](https://github.com/galister/oscavmgr)
by galister, and keeps its MIT license and copyright. The face tracking relay,
OSCQuery advertising, autopilot, Gogo Loco and external storage features come
from that project.

Supported:

- Quest Pro (eye + face, plus camera tongue tracking on rooted headsets)
- Pico 4 Pro/Enterprise (eye + face)
- HTC XR Elite (eye only)
- Samsung Galaxy XR and other `XR_ANDROID_face_tracking` runtimes via WiVRn/Monado (face + tongue)
- Pico via ALVR (52 ARKit face weights) and HTC face trackers via ALVR
- Steam Frame / Quest Pro through SteamVR Steam Link OSC
- iPhone/Android face apps: iFacialMocap, VTube Studio iOS, MeowFace
- Project Babble
- EyeTrackVR

## Installation

Visage advertises itself to VRChat through OSCQuery, so VrcAdvert is not needed.

**From a checkout (Rust 1.89+, [just](https://github.com/casey/just))**:
```bash
just install
```

**Cargo**:
```bash
cargo install --git https://github.com/lemonxah/visage.git --features qpro-tongue,gui,cameras
```

Or grab the portable binary from releases.

### Migrating from oscavmgr

Visage uses its own names, so it can be installed next to oscavmgr, but the two
should not run at the same time.

- Settings now live in `~/.config/visage/`. On first start Visage moves
  `~/.config/oscavmgr-tongue.json`, `extMem.json` and `extGogo.json` there.
- VRChat sees the OSCQuery service as `Visage` instead of `OscAvMgr`.
- The binary, desktop entry, icon and data directory are named `visage`. If you
  used `just install` from an older checkout, remove the old copy with
  `rm -f ~/.local/bin/oscavmgr ~/.local/share/applications/oscavmgr.desktop`
  and `rm -rf ~/.local/share/oscavmgr`.

## Starting

```bash
# If using WiVRn
visage openxr

## If using ALVR
visage alvr

## If using Project Babble and/or EyeTrackVR
visage babble

## If using SteamVR Steam Link (set Steam Link's OSC Output Port to 9015)
visage steamlink --listen 9015

## If using a phone app on the same network (iFacialMocap by default)
visage phone 192.168.1.50
visage phone 192.168.1.50 --protocol vts
```

`openxr` picks up `XR_ANDROID_face_tracking` automatically when the runtime
exposes it and no `FB_face_tracking2` tracker is available. `alvr` accepts
Quest (FB), Pico (ARKit-52) and HTC face data. For Steam Link, enable OSC plus
eye and face tracking sharing in the headset's Steam Link settings. For
`phone`, iFacialMocap uses UDP 49983 and VTube Studio iOS / MeowFace
(`--protocol vts`, enable "3rd Party PC Clients" in VTube Studio) uses UDP
21412; allow that port through the PC firewall. Override with `--phone-port`
and `--listen`. Each input shows a green/red status item (`SL-FACE`,
`SL-EYE`, `PHONE`) and falls back to a neutral face one second after data
stops.

For a rooted Quest Pro, this repository includes the headset camera helper and
the v15 pretrained ONNX models. The easiest setup is a desktop install:

```bash
just install
```

This builds with `qpro-tongue,gui,cameras`, installs the binary to `~/.local/bin`, the
models and headset relay to `~/.local/share/visage`, the app icon, and a
`Visage` application menu entry. The entry carries the same defaults as
`start.sh`: it launches the WiVRn client over adb (`--launch-wivrn`), starts the
mouth-camera relay at launch and stops it on exit (`--tongue-relay-autostart`),
and enables the v15 models, independent eyes and the 0.575 threshold. If the
headset is not ready at launch, use **Start headset cameras** in the Tongue tab.
Set `PREFIX` to install elsewhere, `just run` to start the same setup from the
checkout, and `just uninstall` to remove it.

To run from a terminal instead, build Visage once, then run:

```bash
cargo build --release --features qpro-tongue,gui,cameras
./start.sh
```

This launches WiVRn, starts the mouth-camera relay and port forward, and runs the
local release binary with the Qpro Companion v3.0.3 Developer v15 visibility model
and its paired direction model. Personal calibration is optional. WiVRn supplies
ordinary face, brow, blink and gaze data; the camera models add tongue tracking.
Babble/Baballonia is off by default. The launcher enables `--independent-eyes`
for estimated horizontal eye convergence; set `INDEPENDENT_EYES=0` to disable
that estimate. This PC option does not install or activate the headset's
independent-gaze patch. Connect WiVRn with eye and face tracking enabled and close other
camera viewers first. Press Ctrl+C to stop Visage and its relay/port forward.
ADB must be installed and the headset must already be rooted. The helper's
binaries, C sources, license, and optional Android NDK build script are in
[`headset/`](headset/README.md); no separate `qpro-ft` checkout is needed.
`HEADSET_STREAM_SCRIPT` can override the bundled `./headset-stream.sh` helper.

The model files are included in `models/qpro-companion-v3.0.3/` (about 31 MB total):

- `qpro-stereo-tongue-v15-gate.onnx`: tongue visibility and extension.
- `qpro-stereo-tongue-v15-direction.onnx`: tongue direction and shape.

Their upstream metadata and MIT license, conversion provenance with SHA-256
hashes, and reference-output verification results are included alongside them.
Normal startup needs no model download or Python installation. To reproduce the
ONNX exports, download the official
[QproCompanion.V3.0.3.zip](https://github.com/Fwooffy/Qpro-Enhanced-FT-Wireless/releases/download/v3.0.3/QproCompanion.V3.0.3.zip),
then run the converter with PyTorch and ONNX installed:

```bash
python3 contrib/setup_qpro_v15.py /path/to/QproCompanion.V3.0.3.zip
```

The converter verifies the pinned archive and member checksums, preserves the
release's model metadata/license, and records converted model hashes in
`provenance.json`. Live inference needs neither Python nor PyTorch. Both exported
models have been compared against OpenCV/PyTorch reference outputs on three
synthetic image patterns; this verifies conversion, not tracking accuracy for a
particular wearer. The upstream model remains experimental and has limited
independent-wearer validation. Extracted PyTorch checkpoints and training source
remain ignored by Git. Use `TONGUE_MODEL` and `TONGUE_DIRECTION_MODEL` to select
other compatible models.

The defaults are weighted visibility using the v15 model's camera weight 0.6,
a launcher threshold of 0.575 (overriding the model's 0.5), and motion smoothing 55.
With native tongue confidence zero, activation requires camera confidence of
about 0.958. A threshold of 0.6 reaches the fused score ceiling and can prevent
activation. Set `TONGUE_THRESHOLD=` to use the model's original threshold.
Detection confidence gates the tongue;
the separate extension prediction sets how far the tongue sticks out. Set
`TONGUE_ENABLED=0 ./start.sh` to use only WiVRn's ordinary tracking.

If `agreement` prevents tongue activation, its native tongue score may be too
low. To inspect the camera and native scores while smiling and sticking your
tongue out, run `TONGUE_DEBUG=1 ./start.sh`. The log shows
the active threshold and whether the visibility gate is open. Set
`TONGUE_THRESHOLD` to override the model's threshold with a value greater than 0
and at most 1; choose a value above smile scores and below tongue-out scores.
If those scores overlap, a threshold alone cannot distinguish the expressions.

### Control window

Builds with the `gui` feature open a window when started with `--gui`
(`visage --gui openxr ...`). `start.sh` passes it by default; set `GUI=0` to
run without a window, and builds without the feature fall back to the terminal
with a warning. Closing the window stops Visage.

- **Face**: switch `--ft-mode` live, see what VRChat reports for the avatar,
  per-eye gaze, and every unified and combined blendshape grouped by face
  region. Pin any unified shape and drag its bar to hold it at a fixed value
  while testing avatar blendshapes.
- **Mouth**: live camera, headset and fused tongue scores with a 12-second
  history plot and the activation threshold, the two mouth-camera images, and
  the stream/model status. Every tongue option from the command line can be
  changed while running: on/off (off passes the headset's own tongue value
  through), visibility mode, threshold, camera weight, release hysteresis and
  smoothing. Each of the 12 tongue shapes has a gain slider, and **Manual
  override** drives all of them from sliders and a drag pad to test an avatar
  without the camera. **Save tuning** writes `~/.config/visage/tongue.json`,
  which is loaded on every later start and takes precedence over the `TONGUE_*`
  options; use **Delete saved** to go back to them.
  When the camera stream is down, **Start headset cameras** runs
  `headset-stream.sh start` from inside the app (relay push, streamer
  injection, adb forward). Visage checks over adb every few seconds that the
  connected headset is a Quest Pro with root granted to the adb shell; the
  button, the launch-time autostart and the automatic retries only run once
  that check passes, and the tab says what is missing otherwise. With the
  window enabled, `start.sh` no longer stops when the headset is missing or not
  rooted at launch; it warns and leaves the relay to this button. The relay is
  stopped on exit, including Ctrl+C, `SIGTERM` and logout.
- **Preview**: a live 3D head driven by the same unified expressions that are
  sent to the game, so tracking and pinned shapes can be checked without
  VRChat or Resonite running. Drag to orbit, scroll to zoom, double-click to
  reset.
- **Log**: the last 1000 log lines with level and text filters. Long lines are
  truncated, so the log never grows without bound.

### Reliability

- If the headset disconnects or WiVRn restarts, the OpenXR session is torn down
  cleanly, the face falls back to neutral, and Visage reconnects every 5
  seconds without restarting.
- The OSC loop runs under a supervisor: an unexpected error is logged and the
  loop restarts with backoff instead of closing the app. A busy OSC port (for
  example a second Visage, oscavmgr or VRCFaceTracking) is reported and retried.
- Avatar parameter discovery runs off the main loop with short timeouts, so a
  slow or missing VRChat OSCQuery server never stalls tracking.

For smile false positives, stop Visage and run `./calibrate-tongue.sh` to collect
labeled camera samples. It starts and stops the relay automatically, uses only
Python's standard library, and needs no native tongue label stream. Keep the
headset in its usual position, follow each pose prompt, and press Enter while
holding the pose until the image is saved. Four rounds capture 28 images of
neutral, smiling, mouth-open, and tongue-out poses. The images and labels are saved
under `captures/qpro-calibration/` for inspection and personal model calibration;
the script does not train or replace a model. These captures are ignored by Git.

For the optional combination of WiVRn eyes/brows with Babble mouth data, run
`TONGUE_ENABLED=0 BABBLE_ENABLED=1 ./start.sh`, or use the direct command
`./target/release/visage openxr --babble --independent-eyes`. Configure Babble's
OSC output as `127.0.0.1:9400` with an empty prefix/location, and keep its camera
bridge running separately. Use `--babble 9500` or `BABBLE_PORT=9500` to change the
port. Only fresh Babble mouth values override WiVRn; after one second without an
update, each mouth value falls back to WiVRn. Eye data on the Babble port cannot
overwrite WiVRn gaze or blinks. Camera tongue and Babble mouth cannot be enabled
together. This combined mode requires both `openxr` and `babble` build features.

## Reading Babble and EyeTrackVR cameras directly

Visage can read Babble mouth cameras and EyeTrackVR eye cameras itself and run
their models in-process, so Baballonia, ProjectBabble or EyeTrackApp do not
need to run alongside it. Build with the `cameras` feature (`just build`
includes it):

```bash
cargo build --release --features cameras,gui
```

```bash
# Babble mouth camera on top of OpenXR eyes/brows (fresh camera data wins, stale data falls back to OpenXR after 1s)
visage openxr --babble-camera /dev/ttyUSB0 --babble-model ~/models/faceModel.onnx

# EyeTrackVR eye cameras over Wi-Fi with the Babble OSC receiver
visage babble --etvr-left http://etvr-left.local/ --etvr-right http://etvr-right.local/ \
    --etvr-model ~/models/NEXT_ETVR.onnx

# Everything at once
visage --gui openxr --babble-camera /dev/video2 --babble-model faceModel.onnx \
    --etvr-left /dev/ttyACM0 --etvr-right /dev/ttyACM1 --etvr-model NEXT_ETVR.onnx
```

| Flag | Meaning |
| --- | --- |
| `--babble-camera <SOURCE>` | Babble mouth camera |
| `--babble-model <ONNX>` | Babble face model (`faceModel.onnx` from Baballonia, `model.onnx` from ProjectBabble) |
| `--etvr-left <SOURCE>`, `--etvr-right <SOURCE>` | EyeTrackVR eye cameras; one eye is enough, the other is mirrored |
| `--etvr-model <ONNX>` | EyeTrackVR NEXT stereo model (`NEXT_ETVR*.onnx`, `NEXT_BSB*.onnx`, `NEXT_PSVR*.onnx`) |
| `--camera-threads <N>` | CPU threads per camera model (default 2) |

A `SOURCE` is one of:

- a serial port such as `/dev/ttyUSB0` or `/dev/ttyACM0` (ESP32 / OpenIris firmware at 3,000,000 baud; append `@115200` to use another rate). Your user needs to be in the serial group: `dialout` on Debian, Ubuntu and Fedora, `uucp` on Arch.
- an MJPEG URL such as `http://etvr-left.local/` or `http://192.168.1.60:80/`. A bare host name such as `babble.local` also works.
- a V4L2 webcam such as `/dev/video0` (MJPEG or YUYV). Your user needs to be in the `video` group.

Unplugging a camera, a missing device, a bad frame or a model that fails to
load never stops Visage: each camera reconnects with backoff, each model
reloads with backoff, and errors are logged once per state change. The status
bar shows `BABBLE-CAM` and `ETVR-CAM` in green while fresh camera data is
being merged and in red otherwise.

The **Cameras** tab in the `--gui` window shows each camera's connection,
frame rate and a live preview. Drag on the preview to set the region of
interest (double-click for the full frame), set rotation and flips, change the
source or model, run calibration and watch the live outputs. Mouth calibration
records the range of every shape while you make each mouth shape a few times;
eye calibration records gaze, brow and squint ranges while you look at every
edge of your view, and **Recenter** makes the current gaze the centre.

Settings live in `~/.config/visage/cameras.json`; command-line flags override
the sources and models saved there. Every field is optional:

```json
{
  "mouth": {
    "source": "/dev/ttyUSB0",
    "model": "/home/me/models/faceModel.onnx",
    "view": { "roi": { "x": 0.1, "y": 0.2, "width": 0.8, "height": 0.7 }, "rotation": 0.0, "flip_x": false, "flip_y": false },
    "filter": { "enabled": true, "min_cutoff": 0.5, "beta": 3.0, "d_cutoff": 1.0 },
    "calibration": { "lower": [0.0, "... 45 values"], "upper": [1.0, "... 45 values"] }
  },
  "eyes": {
    "left": { "source": "http://etvr-left.local/", "view": { "rotation": 0.0 } },
    "right": { "source": "http://etvr-right.local/", "view": { "flip_x": true } },
    "model": "/home/me/models/NEXT_ETVR.onnx",
    "filter": { "enabled": true, "min_cutoff": 0.5, "beta": 3.0, "d_cutoff": 1.0 },
    "settings": {
      "open_lid": 0.75,
      "wide_lid": 0.95,
      "limits": { "horizontal": 40.0, "up": 15.0, "down": 40.0 },
      "gaze": { "left_x": { "gain": 1.0, "offset": 0.0 }, "right_x": { "gain": 1.0, "offset": 0.0 }, "y": { "gain": 1.0, "offset": 0.0 } },
      "expressions": { "lower": [0, 0, 0, 0], "upper": [1, 1, 1, 1] }
    }
  }
}
```

**Model licences.** Visage does not ship any Babble or EyeTrackVR model. Use
the `.onnx` files from your own Baballonia, ProjectBabble or EyeTrackApp
install. Those models are distributed by their authors under the Babble
Software Distribution License 1.0 (Apache-2.0 with non-commercial and
copyleft terms), which is not the MIT licence that covers Visage; check that
licence before redistributing them. Visage's camera code is an independent
implementation written from the published wire formats and model interfaces
and contains no Babble or EyeTrackVR code.

## Using with VRChat

Once Visage is started, it will print further instructions to the terminal.

### VRC-Only: Native face tracking

VRChat avatars set up with the SDK's native Face Tracking ("Use All Tracking Keys") receive face data on `/tracking/face/v1`, synced by VRChat without using your parameter budget.

By default (`--ft-mode auto`) Visage sends native face tracking when VRChat reports the current avatar supports it, and the usual VRCFT-style avatar parameters otherwise. The status bar shows `NATIVE` or `LEGACY`.

To force a method, use `--ft-mode legacy`, `--ft-mode native` or `--ft-mode both`.

### VRC-Only: Autopilot

This activates when the avatar bool parameter `AutoPilot` is true. The bottom of the terminal will change from `AP-OFF` to `MANUAL`.

**Turn left-right**: Look at the left/right edge of your screen\
**Jump**: Look at the top edge of your screen\
**Toggle Mute**: Raise your eyebrows\
**Move forward**: Puff your cheeks\
**Move backwards**: Suck your cheeks

### VRC-Only: Gogo Loco integration

Auto loco switch:

- Switches walking animations off when in full-body mode.

Pose save:

- Saves the idle stand/crouch/prone pose between avatars.

Quick-ascend:

- While in Gogo flight mode, put both hands (controllers) above your head to ascend at a super-high speed.
- Requires `TRACK` ticker to be green.

### VRC-Only: VSync parameter

This allows Visage to best keep in sync with the avatar's animator. This is optional and does not require the use of a synced parameter.

- Create a VSync parameter in your VRCExpressionParameters (Bool, not synced, not saved)
- Create a VSync parameter in your Animator (Float)
- Create a new layer in your Animator, with two states set up like [this reference image](./contrib/VSync.webp). (The other state has its VRC Avatar Parameter Driver Value set to 1.)
- Alternatively, VRCFury users can drag and drop the [VSync prefab](./contrib/VSync.unitypackage) in their avatar hierarchy.

### VRC-Only External Storage

This allows you to save infrequently used sync parameters into Visage, so that they don't take up sync param space on your avatar.

Your corresponding animator state machine **must be Write Defaults OFF** for this to work.

Requires 4 parameters:

- `ExtIndex` key/name of the parameter to be saved
- `ExtValue` value of the parameter to be saved
- `IntIndex` key/name of the variable coming from visage
- `IntValue` value of the parameter coming from visage

To set a value:

- Set `ExtIndex` to an integer. This will be the key/name of your parameter.
- Set `ExtValue` to a float. This is the value saved. This float can be 0/1 when saving a bool, or a number higher than 1 when saving an int.

To read a value:

- Have `ExtIndex` on 0 (otherwise visage will be stuck waiting for your input).
- visage will iterate through all of your saved parameters and send them back to VRC (and other players) one at a time.
- In your avatar's FX animator, make a decision tree to handle the `IntValue` if `IntIndex` corresponds to a known value.

## Using with Resonite

Enable this mod: [Nytra/EyeTrackVRResonite](https://github.com/Nytra/EyeTrackVRResonite) (This is a fork that supports both Eye + Face)

With the mod enabled, simply start Visage.

A set of DynamicValueVariables will be created for you. Use them to drive your choice of blendshapes. (Network syncing is already handled for you).

If you are starting up Resonite after using VRC, you will need to restart Visage, or it will keep sending the set of parameters from your last VRC avatar!

## Building from Source

We recommend using the rust and cargo packages provided by the OS.


```bash
cargo build --release
```

If the build is failing, check you Rust version using `rustc --version`. If the reported version is below 1.89, try with Rustup instead.

Notes for ALVR: By default, Visage build for ALVR branch `v20`, which has the latest 20.x release.

If you need to use Visage with a different ALVR version, change the `branch` in `cargo.toml` and then run `cargo update` before building.

## Join the Linux VR Adventures community

- [Discord](https://discord.gg/gHwJ2vwSWV)
- [Matrix](https://matrix.to/#/#linux-vr-adventures:matrix.org)
- [Wiki](https://lvra.gitlab.io/)
