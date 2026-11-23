#!/usr/bin/env python3
"""Collect labeled mouth-camera images for visibility-model diagnosis.

Uses only the Python standard library. Labels come from the pose the user
confirms, independently of the headset's native tongue score.
"""

import argparse
import json
from pathlib import Path
import socket
import struct
import threading
import time
import zlib


HEADER = struct.Struct("<8sIIQQIIIIIIQ")
WIDTH, HEIGHT = 800, 400
POSES = (
    ("neutral", 0, "Relax your mouth; tongue fully hidden."),
    ("small-smile", 0, "Make the small smile that falsely triggers tongue-out; tongue hidden."),
    ("teeth-smile", 0, "Smile with teeth showing; tongue stays behind the teeth."),
    ("open-mouth", 0, "Open your mouth; keep the tongue behind your teeth."),
    ("tongue-tip", 1, "Show the tip of your tongue beyond your lips."),
    ("tongue-out", 1, "Stick your tongue straight out comfortably."),
    ("smile-tongue-out", 1, "Smile with your tongue sticking out beyond your lips."),
)


def read_exact(connection, size):
    output = bytearray()
    while len(output) < size:
        part = connection.recv(size - len(output))
        if not part:
            raise ConnectionError("Camera stream disconnected")
        output.extend(part)
    return bytes(output)


def read_frame(connection):
    fields = HEADER.unpack(read_exact(connection, HEADER.size))
    magic, version, size, sequence, timestamp, width, height, stride, fmt, length, mask, _ = fields
    if (magic, version, size) != (b"QPLIVE3\0", 3, HEADER.size):
        raise ValueError("Unexpected camera stream header")
    layout = width, height, stride, fmt, length, mask
    if layout != (WIDTH, HEIGHT, WIDTH, 1, WIDTH * HEIGHT, 0x0C):
        raise ValueError("Expected mouth mode: cameras 2 and 3, 800x400 grayscale")
    return sequence, timestamp, read_exact(connection, length)


def png(pixels):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    scanlines = b"".join(b"\0" + pixels[y * WIDTH:(y + 1) * WIDTH] for y in range(HEIGHT))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 0, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(scanlines))
        + chunk(b"IEND", b"")
    )


class Receiver:
    def __init__(self, connection):
        self.connection = connection
        self.condition = threading.Condition()
        self.latest = None
        self.error = None
        self.thread = threading.Thread(target=self.receive, daemon=True)
        self.thread.start()

    def receive(self):
        try:
            while True:
                frame = read_frame(self.connection)
                with self.condition:
                    self.latest = (time.monotonic(), frame)
                    self.condition.notify_all()
        except (OSError, ValueError) as error:
            with self.condition:
                self.error = error
                self.condition.notify_all()

    def after(self, deadline):
        with self.condition:
            def ready():
                return self.error is not None or (
                    self.latest is not None and self.latest[0] > deadline
                )

            received = self.condition.wait_for(ready, timeout=5)
            if self.error is not None:
                raise self.error
            if not received:
                raise TimeoutError("No fresh mouth-camera frame; keep face tracking active")
            return self.latest[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=27273)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repetitions", type=int, default=4)
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("--repetitions must be positive")
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "version": 1,
        "purpose": "tongue-visibility-calibration",
        "cameraOrder": [2, 3],
        "complete": False,
        "samples": [],
    }
    journal = args.output / "samples.json"

    def save():
        temporary = journal.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(manifest, indent=2) + "\n")
        temporary.replace(journal)

    save()
    print("Keep the headset in its usual position and face tracking enabled.")
    print("Each Enter confirms the described pose. Hold still until 'Saved'. Type q to stop.")
    try:
        with socket.create_connection(("127.0.0.1", args.port), timeout=5) as connection:
            receiver = Receiver(connection)
            receiver.after(time.monotonic())
            for repetition in range(args.repetitions):
                for name, visible, instruction in POSES:
                    answer = input(
                        f"\nRound {repetition + 1}/{args.repetitions}: {instruction}"
                        "\nHold the pose, then press Enter: "
                    )
                    if answer.strip().lower() == "q":
                        raise KeyboardInterrupt
                    time.sleep(1)
                    sequence, timestamp, pixels = receiver.after(time.monotonic())
                    filename = f"{len(manifest['samples']):03d}-{name}.png"
                    (args.output / filename).write_bytes(png(pixels))
                    manifest["samples"].append({
                        "image": filename,
                        "pose": name,
                        "visibility": visible,
                        "repetition": repetition,
                        "sequence": sequence,
                        "timestampNs": timestamp,
                    })
                    save()
                    print(f"Saved {filename}. Relax before the next pose.")
            manifest["complete"] = True
            save()
    except (KeyboardInterrupt, EOFError):
        print("\nStopped; captured samples are preserved.")
        return 130
    except (OSError, ValueError) as error:
        print(f"Capture failed: {error}. Captured samples are preserved.")
        return 1
    print(f"\nSaved {len(manifest['samples'])} labeled images to {args.output.resolve()}")
    print("This records calibration samples; it does not change or train the model.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
