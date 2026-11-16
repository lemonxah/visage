# OSC Avatar Manager

This is a personal project that includes a face tracking relay (VRCFT replacement), among other things.

Supported:

- Quest Pro (eye + face)
- Pico 4 Pro/Enterprise (eye + face)
- HTC XR Elite (eye only)
- Project Babble
- EyeTrackVR

## Installation

OscAvMgr advertises itself to VRChat through OSCQuery. **VrcAdvert is no longer needed**!

**AUR**: `oscavmgr-git`

**Cargo (Rust 1.86+)**:
```bash
cargo install --git https://github.com/galister/oscavmgr.git
```

**Homebrew**:
```bash
brew tap matrixfurry.com/atomicxr https://tangled.sh/@matrixfurry.com/homebrew-atomicxr
brew install oscavmgr
```

Or grab the portable binary from releases!

## Starting

```bash
# If using WiVRn
oscavmgr openxr

## If using ALVR
oscavmgr alvr

## If using Project Babble and/or EyeTrackVR
oscavmgr babble
```

For a rooted Quest Pro with the camera streamer in `../qpro-ft` and the v15
pretrained models installed locally, run:

```bash
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
camera viewers first. Press Ctrl+C to stop OscAvMgr and its relay/port forward.
The binary must be built with `cargo build --release --features qpro-tongue`.

The model files are in `models/qpro-companion-v3.0.3/` and are ignored by Git. To
reproduce the installation, download the official
[QproCompanion.V3.0.3.zip](https://github.com/Fwooffy/Qpro-Enhanced-FT-Wireless/releases/download/v3.0.3/QproCompanion.V3.0.3.zip),
then run this one-time conversion with PyTorch and ONNX installed:

```bash
python3 contrib/setup_qpro_v15.py /path/to/QproCompanion.V3.0.3.zip
```

The converter verifies the pinned archive and member checksums, preserves the
release's model metadata/license, and records converted model hashes in
`provenance.json`. Live inference needs neither Python nor PyTorch. Both exported
models have been compared against OpenCV/PyTorch reference outputs on three
synthetic image patterns; this verifies conversion, not tracking accuracy for a
particular wearer. The upstream model remains experimental and has limited
independent-wearer validation. The original v8 files in `../qpro-ft/models` remain
available through `TONGUE_MODEL` and `TONGUE_DIRECTION_MODEL` overrides.

The defaults are weighted visibility using the v15 model's camera weight 0.6,
a launcher threshold of 0.573 (overriding the model's 0.5), and motion smoothing 55.
With native tongue confidence zero, activation requires camera confidence of
about 0.955. A threshold of 0.6 reaches the fused score ceiling and can prevent
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

For smile false positives, stop OscAvMgr and run `./calibrate-tongue.sh` to collect
labeled camera samples. It starts and stops the relay automatically, uses only
Python's standard library, and needs no native tongue label stream. Keep the
headset in its usual position, follow each pose prompt, and press Enter while
holding the pose until the image is saved. Four rounds capture 28 images of
neutral, smiling, mouth-open, and tongue-out poses. The images and labels are saved
under `captures/qpro-calibration/` for inspection and personal model calibration;
the script does not train or replace a model. These captures are ignored by Git.

For the optional combination of WiVRn eyes/brows with Babble mouth data, run
`TONGUE_ENABLED=0 BABBLE_ENABLED=1 ./start.sh`, or use the direct command
`./target/release/oscavmgr openxr --babble --independent-eyes`. Configure Babble's
OSC output as `127.0.0.1:9400` with an empty prefix/location, and keep its camera
bridge running separately. Use `--babble 9500` or `BABBLE_PORT=9500` to change the
port. Only fresh Babble mouth values override WiVRn; after one second without an
update, each mouth value falls back to WiVRn. Eye data on the Babble port cannot
overwrite WiVRn gaze or blinks. Camera tongue and Babble mouth cannot be enabled
together. This combined mode requires both `openxr` and `babble` build features.

## Using with VRChat

Once OscAvMgr is started, it will print further instructions to the terminal.

### VRC-Only: Native face tracking

VRChat avatars set up with the SDK's native Face Tracking ("Use All Tracking Keys") receive face data on `/tracking/face/v1`, synced by VRChat without using your parameter budget.

By default (`--ft-mode auto`) OscAvMgr sends native face tracking when VRChat reports the current avatar supports it, and the usual VRCFT-style avatar parameters otherwise. The status bar shows `NATIVE` or `LEGACY`.

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

This allows OscAvMgr to best keep in sync with the avatar's animator. This is optional and does not require the use of a synced parameter.

- Create a VSync parameter in your VRCExpressionParameters (Bool, not synced, not saved)
- Create a VSync parameter in your Animator (Float)
- Create a new layer in your Animator, with two states set up like [this reference image](./contrib/VSync.webp). (The other state has its VRC Avatar Parameter Driver Value set to 1.)
- Alternatively, VRCFury users can drag and drop the [VSync prefab](./contrib/VSync.unitypackage) in their avatar hierarchy.

### VRC-Only External Storage

This allows you to save infrequently used sync parameters into OscAvMgr, so that they don't take up sync param space on your avatar.

Your corresponding animator state machine **must be Write Defaults OFF** for this to work.

Requires 4 parameters:

- `ExtIndex` key/name of the parameter to be saved
- `ExtValue` value of the parameter to be saved
- `IntIndex` key/name of the variable coming from oscavmgr
- `IntValue` value of the parameter coming from oscavmgr

To set a value:

- Set `ExtIndex` to an integer. This will be the key/name of your parameter.
- Set `ExtValue` to a float. This is the value saved. This float can be 0/1 when saving a bool, or a number higher than 1 when saving an int.

To read a value:

- Have `ExtIndex` on 0 (otherwise oscavmgr will be stuck waiting for your input).
- oscavmgr will iterate through all of your saved parameters and send them back to VRC (and other players) one at a time.
- In your avatar's FX animator, make a decision tree to handle the `IntValue` if `IntIndex` corresponds to a known value.

## Using with Resonite

Enable this mod: [Nytra/EyeTrackVRResonite](https://github.com/Nytra/EyeTrackVRResonite) (This is a fork that supports both Eye + Face)

With the mod enabled, simply start OscAvMgr.

A set of DynamicValueVariables will be created for you. Use them to drive your choice of blendshapes. (Network syncing is already handled for you).

If you are starting up Resonite after using VRC, you will need to restart OscAvMgr, or it will keep sending the set of parameters from your last VRC avatar!

## Building from Source

We recommend using the rust and cargo packages provided by the OS.


```bash
cargo build --release
```

If the build is failing, check you Rust version using `rustc --version`. If the reported version is below 1.86, try with Rustup instead.

Notes for ALVR: By default, OscAvMgr build for ALVR branch `v20`, which has the latest 20.x release.

If you need to use OscAvMgr with a different ALVR version, change the `branch` in `cargo.toml` and then run `cargo update` before building.

## Join the Linux VR Adventures community

- [Discord](https://discord.gg/gHwJ2vwSWV)
- [Matrix](https://matrix.to/#/#linux-vr-adventures:matrix.org)
- [Wiki](https://lvra.gitlab.io/)
