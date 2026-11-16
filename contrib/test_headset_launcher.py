#!/usr/bin/env python3
"""Exercise the bundled launchers without connecting to a headset.

Run with: python3 contrib/test_headset_launcher.py
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
MODEL_DIR = Path("models/qpro-companion-v3.0.3")
RELAY = "/data/local/tmp/questpro-camera-relay-v8"
INJECTOR = "/data/local/tmp/questpro-camera-injector"


class BundledAssetsTests(unittest.TestCase):
    def test_model_hashes_match_provenance(self):
        directory = ROOT / MODEL_DIR
        provenance = json.loads((directory / "provenance.json").read_text())
        self.assertEqual({model["role"] for model in provenance["models"]}, {"gate", "direction"})
        for model in provenance["models"]:
            with self.subTest(role=model["role"]):
                digest = hashlib.sha256((directory / model["onnx"]).read_bytes()).hexdigest()
                self.assertEqual(digest, model["onnxSha256"])

    def test_headset_binaries_match_manifest(self):
        directory = ROOT / "headset"
        for line in (directory / "SHA256SUMS").read_text().splitlines():
            expected, name = line.split()
            with self.subTest(binary=name):
                self.assertEqual(hashlib.sha256((directory / name).read_bytes()).hexdigest(), expected)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="oscavmgr-launcher-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        # A standalone checkout with spaces in its path and no sibling qpro-ft.
        self.project = self.directory / "project with spaces"
        self.project.mkdir()
        for script in ("start.sh", "calibrate-tongue.sh", "headset-stream.sh"):
            shutil.copy2(ROOT / script, self.project / script)
        shutil.copytree(ROOT / "headset/bin", self.project / "headset/bin")
        models = self.project / MODEL_DIR
        models.mkdir(parents=True)
        for role in ("gate", "direction"):
            # Inference is mocked; real model integrity is checked separately.
            (models / f"qpro-stereo-tongue-v15-{role}.onnx").touch()
        self.log = self.directory / "calls.jsonl"
        mock_bin = self.directory / "mock-bin"
        mock_bin.mkdir()
        self.write_executable(mock_bin / "adb", '''
import json, os, sys
args = sys.argv[1:]
with open(os.environ["QA_LOG"], "a") as log:
    log.write(json.dumps(["adb", *args]) + "\\n")
command = " ".join(args)
if args == ["get-state"]:
    print("device")
elif args == ["get-serialno"]:
    print("TEST_HEADSET")
elif "resolve-activity" in args:
    print("org.meumeu.wivrn/.MainActivity")
elif command == 'shell su -c "id -u"':
    print("0")
elif command == "shell su -c id":
    print("uid=0(root)")
elif command == "shell su -c cat /data/local/tmp/questpro-relay-v8.log":
    if not os.environ.get("QA_RELAY_FAIL"):
        print("RELAY_LISTENING")
elif command.startswith("shell su -c /data/local/tmp/questpro-camera-injector "):
    sys.exit(int(os.environ.get("QA_INJECT_EXIT", "0")))
elif args and args[0] == "push":
    assert os.path.isfile(args[1]), args[1]
''')
        self.write_executable(self.project / "target/release/oscavmgr", '''
import json, os, sys
if "--help" in sys.argv:
    print("--tongue-model --babble")
else:
    with open(os.environ["QA_LOG"], "a") as log:
        log.write(json.dumps(["oscavmgr", *sys.argv[1:]]) + "\\n")
    sys.exit(int(os.environ.get("QA_OSC_EXIT", "0")))
''')
        self.write_executable(mock_bin / "sleep", "")
        self.write_executable(mock_bin / "python3", '''
import json, os, sys
with open(os.environ["QA_LOG"], "a") as log:
    log.write(json.dumps(["capture", *sys.argv[1:]]) + "\\n")
sys.exit(int(os.environ.get("QA_CAPTURE_EXIT", "0")))
''')
        self.env = {
            "PATH": str(mock_bin) + os.pathsep + os.environ["PATH"],
            "HOME": str(self.directory),
            "QA_LOG": str(self.log),
        }

    @staticmethod
    def write_executable(path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"#!{sys.executable}\n{body}")
        path.chmod(0o755)

    def run_script(self, script, *args, **environment):
        result = subprocess.run(
            ["bash", str(self.project / script), *args],
            cwd=self.directory, env={**self.env, **environment},
            capture_output=True, text=True, timeout=10,
        )
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def assert_stopped(self, calls, port="27273"):
        self.assertIn(["adb", "shell", "su", "-c", f"{RELAY} --stop"], calls)
        self.assertEqual(calls[-1], ["adb", "forward", "--remove", f"tcp:{port}"])

    def test_default_start_uses_bundled_assets_and_cleans_up(self):
        result, calls = self.run_script("start.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        pushes = [call for call in calls if call[:2] == ["adb", "push"]]
        self.assertEqual(len(pushes), 3)
        self.assertTrue(all(Path(call[2]).parent == self.project / "headset/bin" for call in pushes))
        osc = next(call for call in calls if call[0] == "oscavmgr")
        for option, role in (("--tongue-model", "gate"), ("--tongue-direction-model", "direction")):
            self.assertEqual(Path(osc[osc.index(option) + 1]), MODEL_DIR / f"qpro-stereo-tongue-v15-{role}.onnx")
        self.assert_stopped(calls)

    def test_custom_pc_port_still_forwards_to_bundled_headset_port(self):
        result, calls = self.run_script("start.sh", TONGUE_PORT="28123")
        self.assertEqual(result.returncode, 0, result.stderr)
        forwards = [call for call in calls if call[:2] == ["adb", "forward"] and "--remove" not in call]
        self.assertTrue(forwards)
        self.assertTrue(all(call[2:] == ["tcp:28123", "tcp:27273"] for call in forwards))
        self.assert_stopped(calls, "28123")

    def test_osc_failure_is_preserved_and_relay_stops(self):
        result, calls = self.run_script("start.sh", QA_OSC_EXIT="7")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assert_stopped(calls)

    def test_standalone_injection_failure_stops_relay(self):
        result, calls = self.run_script("headset-stream.sh", "start", QA_INJECT_EXIT="8")
        self.assertEqual(result.returncode, 8, result.stderr)
        self.assert_stopped(calls)

    def test_relay_start_failure_stops_before_injection(self):
        result, calls = self.run_script("headset-stream.sh", "start", QA_RELAY_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Relay did not start", result.stderr)
        self.assertFalse(any(call[:4] == ["adb", "shell", "su", "-c"] and call[4].startswith(INJECTOR) for call in calls))
        self.assert_stopped(calls)

    def test_disabled_camera_and_wivrn_launch_need_no_adb(self):
        result, calls = self.run_script("start.sh", TONGUE_ENABLED="0", LAUNCH_WIVRN="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0], "oscavmgr")
        self.assertNotIn("--tongue-model", calls[0])

    def test_invalid_settings_fail_before_contacting_headset(self):
        for setting in ({"PORT": "65536"}, {"PORT": "0"}, {"PORT": "bad"},
                        {"MODE": "unknown"}, {"MAX_FPS": "121"}):
            with self.subTest(setting=setting):
                result, calls = self.run_script("headset-stream.sh", "start", **setting)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])

    def test_calibration_uses_bundled_helper_and_cleans_up_on_failure(self):
        result, calls = self.run_script("calibrate-tongue.sh", TONGUE_PORT="28123", QA_CAPTURE_EXIT="9")
        self.assertEqual(result.returncode, 9, result.stderr)
        capture = next(call for call in calls if call[0] == "capture")
        self.assertEqual(capture[1:4], ["contrib/qpro_tongue_capture.py", "--port", "28123"])
        self.assertTrue(any("--mode mouth --max-fps 12" in " ".join(call) for call in calls))
        self.assert_stopped(calls, "28123")


if __name__ == "__main__":
    unittest.main()
