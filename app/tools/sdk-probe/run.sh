#!/usr/bin/env bash
# Run the OBSBOT SDK probe while recording the camera's USB traffic, and write one report.
#
#   bash run.sh [--hidden | --round4 [--sleep-images]] [--toggles] [--moves] [path/to/libdev_v2.1.0_8]
#
# It builds the probe against your local copy of the SDK, stops this app's background service for the
# duration (it would add its own traffic), records USB with tshark (asks for your password once), runs the
# probe, writes report.txt, and starts the service again. Close the app's windows (the preview) first.
# What the probe does to the camera: see probe.cpp. It never resets, deletes presets, or updates firmware.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
flags=()
sdk="$HOME/obsbot-sdk/libdev_v2.1.0_8"
for arg in "$@"; do
    case "$arg" in
        --toggles|--moves|--with-moves|--hidden|--round4|--round5|--sleep-images) flags+=("$arg") ;;
        *) sdk="$arg" ;;
    esac
done
out="$HOME/obsbot-sdk-probe-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"

need() { command -v "$1" >/dev/null || { echo "Missing: $1 ($2)"; exit 1; }; }
need cmake "install cmake"; need c++ "install a C++ compiler (gcc)"; need tshark "install wireshark-cli"
need python3 "install python"; need lsusb "install usbutils"; need sudo "needed for USB capture"
need nm "install binutils"
[ -d "$sdk/include/dev" ] || { echo "SDK not found at $sdk (pass its path)"; exit 1; }

echo "== Building the probe"
cmake -S "$here" -B "$out/build" -DOBSBOT_SDK="$sdk" -DCMAKE_BUILD_TYPE=Release >"$out/build.log" 2>&1 \
    && cmake --build "$out/build" >>"$out/build.log" 2>&1 || { echo "Build failed; see $out/build.log"; exit 1; }

line="$(lsusb -d 3564: | head -n1)"
[ -n "$line" ] || { echo "No OBSBOT camera on USB (lsusb -d 3564:)"; exit 1; }
bus=$((10#$(echo "$line" | awk '{print $2}')))
addr=$((10#$(echo "$line" | awk '{print $4}' | tr -d ':')))
echo "== Camera on bus $bus, device $addr"

service_was_active=0
if systemctl --user is-active --quiet tiny4lite-control 2>/dev/null; then
    service_was_active=1
    echo "== Stopping the background service for the test"
    systemctl --user stop tiny4lite-control
fi
restore() {
    sudo pkill -INT -f "tshark -i usbmon$bus" 2>/dev/null || true
    if [ "$service_was_active" = 1 ]; then systemctl --user start tiny4lite-control || true; echo "== Background service started again"; fi
}
trap restore EXIT

echo "== Starting the USB capture (password for sudo)"
sudo -v
sudo modprobe usbmon
sudo tshark -i "usbmon$bus" -q -w - >"$out/capture.pcapng" 2>"$out/tshark.log" &
sleep 3

echo "== Running the probe (a few minutes; leave the camera alone)"
"$out/build/sdk-probe" "${flags[@]}" | tee "$out/probe.log" || true
sleep 2
sudo pkill -INT -f "tshark -i usbmon$bus" || true
sleep 2

echo "== Writing the report"
sudo chown "$(id -u):$(id -g)" "$out/capture.pcapng" 2>/dev/null || true
python3 "$here/report.py" "$out/probe.log" "$out/capture.pcapng" "$addr" >"$out/report.txt"
echo
echo "Done. Report: $out/report.txt  ($(wc -l <"$out/report.txt") lines)"
echo "Raw capture: $out/capture.pcapng"
