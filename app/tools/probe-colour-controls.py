#!/usr/bin/env python3
"""Measure what Hue, Red balance, Blue balance and Colour temperature do to the picture.

    python3 tools/probe-colour-controls.py [/dev/videoN]

Point the camera at something still, with neutral colours (a white wall or a grey card is best), and keep
the lighting steady. Close any app using the camera first.

For each setting it tries a few values, waits for the picture to settle, grabs one frame with ffmpeg and
prints the average red, green and blue (0-255) and their ratios to green. It also reads red and blue balance
back after each colour temperature, to see whether the camera derives them from it.

It only changes picture-colour settings (never the gimbal), turns Auto white balance off for the test, and
puts every value back as it was at the end, even if it is interrupted.
Needs v4l2-ctl and ffmpeg.
"""
import glob
import subprocess
import sys
import time

CONTROLS = ["white_balance_automatic", "white_balance_temperature", "red_balance", "blue_balance", "hue"]


def find_camera():
    for path in sorted(glob.glob("/sys/class/video4linux/video*/name")):
        if "Tiny" in open(path).read() or "OBSBOT" in open(path).read():
            node = "/dev/" + path.split("/")[-2]
            info = subprocess.run(["v4l2-ctl", "-d", node, "--list-ctrls"], capture_output=True, text=True).stdout
            if "white_balance" in info:
                return node
    return None


def get(dev, name):
    out = subprocess.run(["v4l2-ctl", "-d", dev, f"--get-ctrl={name}"], capture_output=True, text=True).stdout
    return int(out.split(":")[1]) if ":" in out else None


def set_(dev, name, value):
    subprocess.run(["v4l2-ctl", "-d", dev, f"--set-ctrl={name}={value}"], capture_output=True)


def mean_rgb(dev):
    # Skip a few frames so the new setting has taken effect, then average one small frame.
    raw = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "v4l2", "-i", dev,
                          "-vf", "select=gte(n\\,8),scale=64:36", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                         capture_output=True).stdout
    if len(raw) < 64 * 36 * 3:
        return None
    n = len(raw) // 3
    r = sum(raw[0::3]) / n
    g = sum(raw[1::3]) / n
    b = sum(raw[2::3]) / n
    return r, g, b


def show(label, rgb, extra=""):
    if rgb is None:
        print(f"  {label:>14}: no frame")
        return
    r, g, b = rgb
    print(f"  {label:>14}: R {r:6.1f}  G {g:6.1f}  B {b:6.1f}   R/G {r / max(g, 1):.2f}  B/G {b / max(g, 1):.2f}{extra}")


def main():
    dev = sys.argv[1] if len(sys.argv) > 1 else find_camera()
    if not dev:
        sys.exit("No Tiny 2 Lite found; give its node, e.g. /dev/video0")
    saved = {name: get(dev, name) for name in CONTROLS}
    print(f"Camera {dev}. Values before the test: {saved}\n")
    try:
        set_(dev, "white_balance_automatic", 0)
        time.sleep(1)

        print("Colour temperature (and the red/blue balance the camera reports after it):")
        for kelvin in (2000, 3000, 4000, 5000, 6500, 8000, 10000):
            set_(dev, "white_balance_temperature", kelvin)
            time.sleep(1.5)
            show(f"{kelvin} K", mean_rgb(dev), f"   red_balance {get(dev, 'red_balance')}  blue_balance {get(dev, 'blue_balance')}")
        set_(dev, "white_balance_temperature", saved["white_balance_temperature"] or 5000)
        time.sleep(1)

        for name in ("red_balance", "blue_balance"):
            print(f"\n{name} (the other one, and temperature, as reported after each):")
            for value in (0, 512, 1024, 1536, 2048):
                set_(dev, name, value)
                time.sleep(1.5)
                other = "blue_balance" if name == "red_balance" else "red_balance"
                show(str(value), mean_rgb(dev), f"   {other} {get(dev, other)}  temperature {get(dev, 'white_balance_temperature')}")
            set_(dev, name, saved[name] if saved[name] is not None else 1024)
            time.sleep(1)

        print("\nhue:")
        for value in (0, 25, 50, 75, 100):
            set_(dev, "hue", value)
            time.sleep(1.5)
            show(str(value), mean_rgb(dev))
    finally:
        # Everything back as it was: manual values first, then auto white balance last.
        for name in ("hue", "white_balance_temperature", "red_balance", "blue_balance", "white_balance_automatic"):
            if saved.get(name) is not None:
                set_(dev, name, saved[name])
        print(f"\nRestored: {saved}")


if __name__ == "__main__":
    main()
