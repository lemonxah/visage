#!/usr/bin/env python3
"""Extract and export the official Qpro Companion v3.0.3 developer model.

Usage: python3 contrib/setup_qpro_v15.py /path/to/QproCompanion.V3.0.3.zip
Requires PyTorch and ONNX only during setup. Live inference uses Rust/ONNX Runtime.
"""

import argparse
import ast
import hashlib
import json
from pathlib import Path
import zipfile


RELEASE_URL = "https://github.com/Fwooffy/Qpro-Enhanced-FT-Wireless/releases/download/v3.0.3/QproCompanion.V3.0.3.zip"
ARCHIVE_SHA256 = "2c9004bdd8ecb29d5795e5cdb91b71dd525242a4b295d257a47c77d3ee7cec0a"
PREFIX = "QproFaceTracking.V3.0.3/"
CHECKPOINTS = {
    "gate": "dbe22ab74943ce9b134bb17a4dbacdbc4c781d9943005cfa3548fc958441015c",
    "direction": "1900e8761c9ceaf89069121af1016ba24c33849836a5b7ee94b4dfd9fc7db396",
}
SOURCE_SHA256 = "0e853cdb80f3a82efa3720f92568f56108299633e4f8131bc4fe1ddbea124797"
TARGETS = [
    "visibility", "extension", "horizontal", "vertical", "curl_up",
    "bend_down", "roll", "flat", "squish", "twist",
]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_architecture(source):
    import torch
    from torch import nn
    from torch.nn import functional as functional

    # Import only the three reviewed network classes, without pulling in the
    # Windows training UI, capture modules, GPU setup or unrelated dependencies.
    tree = ast.parse(source)
    names = {"ResidualBlock", "SpatialTongueEncoder", "SpatialStereoTongueModel"}
    classes = [node for node in tree.body if isinstance(node, ast.ClassDef) and node.name in names]
    if {node.name for node in classes} != names:
        raise ValueError("The release is missing the expected model architecture")
    namespace = {
        "torch": torch, "nn": nn, "F": functional,
        "SIGNED_TARGETS": {"horizontal", "vertical", "twist"},
    }
    exec(compile(ast.Module(body=classes, type_ignores=[]), "qpro_v3.0.3_architecture", "exec"), namespace)
    return namespace["SpatialStereoTongueModel"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument(
        "--output", type=Path,
        default=Path(__file__).resolve().parents[1] / "models/qpro-companion-v3.0.3",
    )
    args = parser.parse_args()
    if sha256(args.archive) != ARCHIVE_SHA256:
        raise ValueError("Release archive does not match the official GitHub asset SHA-256")

    import onnx
    import torch

    torch.set_num_threads(2)
    args.output.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.archive) as archive:
        checksums = {}
        for line in archive.read(PREFIX + "SHA256SUMS.txt").decode("utf-8-sig").splitlines():
            fields = line.split(maxsplit=1)
            if len(fields) == 2:
                checksums[fields[1].replace("\\", "/")] = fields[0]

        def verified(relative):
            data = archive.read(PREFIX + relative)
            digest = hashlib.sha256(data).hexdigest()
            if checksums.get(relative) != digest:
                raise ValueError(f"Release member checksum mismatch: {relative}")
            return data

        source = verified("QproRuntime/train_tongue_model.py")
        if hashlib.sha256(source).hexdigest() != SOURCE_SHA256:
            raise ValueError("Unexpected model architecture source")
        model_type = load_architecture(source)
        (args.output / "train_tongue_model.py").write_bytes(source)
        metadata = verified("QproRuntime/models/qpro-stereo-tongue-v15.metadata.json")
        (args.output / "qpro-stereo-tongue-v15.metadata.json").write_bytes(metadata)
        (args.output / "LICENSE").write_bytes(verified("Docs/LICENSE"))

        models = []
        for role, expected_digest in CHECKPOINTS.items():
            name = f"qpro-stereo-tongue-v15-{role}"
            checkpoint = args.output / f"{name}.pt"
            data = verified(f"QproRuntime/models/{checkpoint.name}")
            if hashlib.sha256(data).hexdigest() != expected_digest:
                raise ValueError(f"Unexpected {role} model")
            checkpoint.write_bytes(data)
            state = torch.load(checkpoint, map_location="cpu", weights_only=True)
            if state["architecture"] != "spatial-stereo-resnet-v2" or state["targetNames"] != TARGETS:
                raise ValueError(f"Unsupported {role} model output schema")
            preprocessing = state.get("inputPreprocessing", "raw-v1")
            if preprocessing != "raw-v1":
                raise ValueError(f"Unsupported preprocessing: {preprocessing}")
            size = int(state["imageSize"])
            if size != {"gate": 224, "direction": 192}[role]:
                raise ValueError(f"Unexpected {role} image size {size}")

            model = model_type(TARGETS).eval()
            model.load_state_dict(state["modelState"], strict=True)
            sample = torch.rand(1, 2, size, size, generator=torch.Generator().manual_seed(0))
            output = args.output / f"{name}.onnx"
            temporary = output.with_suffix(".onnx.tmp")
            torch.onnx.export(
                model, (sample,), str(temporary), input_names=["cameras"],
                output_names=["values"], opset_version=17, dynamo=False,
            )
            proto = onnx.load(str(temporary))
            fields = {
                "qproTongueOnnxFormat": "1", "sourceSha256": expected_digest,
                "architecture": state["architecture"], "targetNames": json.dumps(TARGETS),
                "imageSize": str(size), "inputPreprocessing": preprocessing,
                "visibilityGate": json.dumps(state.get("visibilityGate", {})),
                "modelVersion": "15", "sourceRelease": "v3.0.3",
            }
            for key, value in fields.items():
                entry = proto.metadata_props.add()
                entry.key, entry.value = key, value
            onnx.checker.check_model(proto)
            onnx.save(proto, str(temporary))
            temporary.replace(output)
            models.append({
                "role": role, "checkpoint": checkpoint.name, "checkpointSha256": expected_digest,
                "onnx": output.name, "onnxSha256": sha256(output), "imageSize": size,
                "inputPreprocessing": preprocessing, "visibilityGate": state.get("visibilityGate", {}),
            })
            print(f"Exported {output}: {size}px, {preprocessing}", flush=True)

    provenance = {
        "release": "v3.0.3", "modelVersion": 15, "downloadUrl": RELEASE_URL,
        "archiveSha256": ARCHIVE_SHA256, "architectureSourceSha256": SOURCE_SHA256,
        "torchVersion": torch.__version__, "onnxVersion": onnx.__version__, "models": models,
    }
    (args.output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print("Export complete. Compare Rust inference with PyTorch before enabling these models.")


if __name__ == "__main__":
    main()
