# Quest Pro camera helper

`../headset-stream.sh` deploys the bundled Android ARM64 binaries in `bin/`
over ADB. It starts the relay on headset loopback port 27273 and injects the
streamer into the sensor provider on a rooted Quest Pro. `../start.sh` and
`../calibrate-tongue.sh` use this helper automatically.

The three C sources are copied unchanged from
[Qpro-Enhanced-FT](https://github.com/n0tmast3r/Qpro-Enhanced-FT/tree/a95ec1306b66b4045dd0a7eef0ac62443fcfbf87)
at commit `a95ec1306b66b4045dd0a7eef0ac62443fcfbf87`.
Their MIT license is preserved in `LICENSE`. The prebuilt binaries are the
same files used by the previous local `qpro-ft/headset` installation;
`SHA256SUMS` records their hashes. They are included so normal startup requires
neither an Android NDK installation nor a separate camera-helper checkout.
The bundled files identify their build toolchain as NDK r30-beta2; rebuilding
with r29 produces different binary hashes.

To rebuild from this repository with the Android NDK (tested with r29):

```bash
ANDROID_NDK_HOME=/path/to/android-ndk ./headset/build.sh
```

This uses the upstream build flags for Android API 28 and 16 KiB page alignment,
with `STREAM_PORT=27273`. Set `OUT_DIR` to build elsewhere for comparison without
replacing the bundled binaries. To verify the original bundled files, run
`(cd headset && sha256sum -c SHA256SUMS)`.

Run `./headset-stream.sh start`, `stop`, or `logs` from the project root for
manual control. `PORT` selects the PC port (default 27273), `MODE` selects
`mouth` (default), `face`, `eyes`, or `all`, and `MAX_FPS` sets the frame cap
(default 24; 0 means uncapped). `ADB` can select another ADB executable and
`ANDROID_SERIAL` can select a headset. The streamer remains injected after
`stop`, but capture stops with the relay; reboot the headset to unload it.

Run `python3 contrib/test_headset_launcher.py` from the project root to verify
the bundled binary/model hashes and exercise startup, port forwarding, and
cleanup using a simulated ADB connection. These checks do not access a headset.
