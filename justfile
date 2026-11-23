set shell := ["bash", "-euo", "pipefail", "-c"]

prefix := env("PREFIX", home_directory() / ".local")
features := "qpro-tongue,gui,cameras"
model := "qpro-companion-v3.0.3"
threshold := "0.575"

bindir := prefix / "bin"
datadir := prefix / "share" / "visage"
appdir := prefix / "share" / "applications"
icondir := prefix / "share" / "icons" / "hicolor"
sizes := "16 32 48 64 128 256 512"

default:
    @just --list

build:
    cargo build --release --features {{ features }}

run *args: build
    ./target/release/visage --gui openxr --independent-eyes --launch-wivrn \
        --tongue-model "models/{{ model }}/qpro-stereo-tongue-v15-gate.onnx" \
        --tongue-direction-model "models/{{ model }}/qpro-stereo-tongue-v15-direction.onnx" \
        --tongue-threshold {{ threshold }} \
        --tongue-relay-script ./headset-stream.sh --tongue-relay-autostart {{ args }}

install: build
    install -Dm755 target/release/visage "{{ bindir }}/visage"
    install -Dm755 headset-stream.sh "{{ datadir }}/headset-stream.sh"
    install -Dm755 -t "{{ datadir }}/headset/bin" headset/bin/*
    install -Dm644 -t "{{ datadir }}/headset" headset/LICENSE headset/SHA256SUMS
    install -Dm644 -t "{{ datadir }}/models/{{ model }}" models/{{ model }}/*.onnx models/{{ model }}/LICENSE models/{{ model }}/provenance.json
    for size in {{ sizes }}; do \
        install -Dm644 "assets/icons/visage-$size.png" "{{ icondir }}/${size}x${size}/apps/visage.png"; \
    done
    mkdir -p "{{ appdir }}"
    sed -e 's|@BIN@|{{ bindir }}/visage|g' \
        -e 's|@DATA@|{{ datadir }}|g' \
        -e 's|@MODEL@|{{ model }}|g' \
        -e 's|@THRESHOLD@|{{ threshold }}|g' \
        assets/visage.desktop.in > "{{ appdir }}/visage.desktop"
    chmod 644 "{{ appdir }}/visage.desktop"
    -update-desktop-database -q "{{ appdir }}" 2>/dev/null
    -gtk-update-icon-cache -q -t -f "{{ icondir }}" 2>/dev/null
    @echo "Installed Visage to {{ prefix }}; launch it from your application menu."

uninstall:
    rm -f "{{ bindir }}/visage" "{{ appdir }}/visage.desktop"
    rm -rf "{{ datadir }}"
    for size in {{ sizes }}; do rm -f "{{ icondir }}/${size}x${size}/apps/visage.png"; done
    -update-desktop-database -q "{{ appdir }}" 2>/dev/null
    -gtk-update-icon-cache -q -t -f "{{ icondir }}" 2>/dev/null
