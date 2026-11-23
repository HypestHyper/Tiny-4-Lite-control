#!/usr/bin/env python3
"""Match the USB traffic of a capture to the probe's steps, and write one report.

    report.py probe.log packets.tsv > report.txt

packets.tsv is tshark's field output (see run.sh). Packets are put under the step whose time window they fall
in: from its BEGIN to just before the next step's BEGIN. Identical packets within a step are listed once,
with a count, so the SDK's own status polling (about ten reads a second) does not bury the command itself.
"""
import collections
import re
import sys

log_path, tsv_path = sys.argv[1], sys.argv[2]
log = open(log_path).read().splitlines()

steps = []  # (name, begin, end, ret, detail lines)
current = None
header = []
for line in log:
    m = re.match(r"STEP (\S+) BEGIN ([\d.]+)", line)
    if m:
        current = [m.group(1), float(m.group(2)), None, None, []]
        steps.append(current)
        continue
    m = re.match(r"STEP (\S+) END ([\d.]+) ret=(-?\d+)", line)
    if m and current:
        current[2], current[3] = float(m.group(2)), int(m.group(3))
        continue
    if current is not None and line.startswith("  "):
        current[4].append(line)
    elif current is None:
        header.append(line)

connected = next((float(l.split()[1]) for l in log if l.startswith("CONNECTED ")), None)

rows = []
with open(tsv_path) as f:
    names = f.readline().rstrip("\n").split("\t")
    for line in f:
        values = line.rstrip("\n").split("\t")
        row = dict(zip(names, values + [""] * (len(names) - len(values))))
        try:
            row["t"] = float(row.get("frame.time_epoch", "0") or 0)
        except ValueError:
            continue
        rows.append(row)

def describe(row):
    direction = {"'S'": "submit", "'C'": "complete", "83": "submit", "67": "complete"}.get(row.get("usb.urb_type", ""), row.get("usb.urb_type", ""))
    data = row.get("usb.capdata", "") or row.get("usb.control.Response", "")
    return "%-8s ep=%-4s bmReq=%-4s bReq=%-4s wValue=%-6s wIndex=%-6s len=%-4s status=%-3s data=%s" % (
        direction, row.get("usb.endpoint_address", ""), row.get("usb.bmRequestType", ""), row.get("usb.setup.bRequest", ""),
        row.get("usb.setup.wValue", ""), row.get("usb.setup.wIndex", ""), row.get("usb.data_len", ""),
        row.get("usb.urb_status", ""), data.replace(":", ""))

def section(title, start, end, extra=()):
    print("=" * 100)
    print(title)
    for line in extra:
        print(line)
    picked = [r for r in rows if start <= r["t"] < end]
    counts = collections.OrderedDict()
    for r in picked:
        key = describe(r)
        counts[key] = counts.get(key, 0) + 1
    print("  packets: %d (%d distinct)" % (len(picked), len(counts)))
    for key, n in counts.items():
        print("  %4dx %s" % (n, key))

for line in header:
    print(line)
if connected and steps:
    section("BASELINE (SDK traffic after connecting, before the first step)", connected, steps[0][1] - 0.05)
for i, (name, begin, end, ret, detail) in enumerate(steps):
    stop = steps[i + 1][1] - 0.05 if i + 1 < len(steps) else (end or begin) + 2.0
    section("STEP %s  ret=%s  (%.3f s)" % (name, ret, (end or begin) - begin), begin - 0.02, stop, detail)
