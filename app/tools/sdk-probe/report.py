#!/usr/bin/env python3
"""Match the USB traffic of a capture to the probe's steps, and write one report.

    report.py probe.log capture.pcapng [device-address] > report.txt

The capture is read directly (pcapng from usbmon, link type 220: the kernel's 64-byte usbmon header, then the
data), so the request fields and data bytes do not depend on which Wireshark dissectors are installed. Without
a device address, the device with the most 60-byte class requests (the OBSBOT extension unit) is used.

Packets are put under the step whose time window they fall in: from its BEGIN to just before the next step's
BEGIN. Identical packets within a step are listed once, with a count, so the SDK's status polling does not
bury the command itself. The camera's serial number is replaced by <serial>.
"""
import collections
import re
import struct
import sys

log_path, pcap_path = sys.argv[1], sys.argv[2]
address = int(sys.argv[3]) if len(sys.argv) > 3 else None
log = open(log_path, errors="replace").read()

# Keep the serial number out of the report.
serials = set(re.findall(r"dev sn = (\w+)", log)) | set(re.findall(r"add uvc device: (\w+)", log))
for serial in serials:
    log = log.replace(serial, "<serial>")
log = re.sub(r"device uuid is: \w+", "device uuid is: <uuid>", log)
log = re.sub(r"\b[0-9A-F]{48}\b", "<uuid>", log)
lines = log.splitlines()


def read_pcapng(path):
    """Yield (epoch seconds, usbmon header dict, data bytes) for every packet."""
    data = open(path, "rb").read()
    pos, endian = 0, "<"
    while pos + 12 <= len(data):
        block_type = struct.unpack_from(endian + "I", data, pos)[0]
        if block_type == 0x0A0D0D0A:  # section header: find the byte order
            magic = data[pos + 8:pos + 12]
            endian = "<" if magic == b"\x4d\x3c\x2b\x1a" else ">"
        length = struct.unpack_from(endian + "I", data, pos + 4)[0]
        if length < 12 or pos + length > len(data):
            break
        if block_type == 6:  # enhanced packet block
            caplen = struct.unpack_from(endian + "I", data, pos + 20)[0]
            packet = data[pos + 28:pos + 28 + caplen]
            if len(packet) >= 64:
                (urb_id, kind, xfer, ep, dev, bus, flag_setup, flag_data, ts_sec, ts_usec, status, length_, len_cap) = \
                    struct.unpack_from("<QBBBBHbbqii", packet, 0)[:11] + struct.unpack_from("<II", packet, 32)
                setup = packet[40:48]
                yield ts_sec + ts_usec / 1e6, {
                    "kind": chr(kind), "xfer": xfer, "ep": ep, "dev": dev, "status": status,
                    "length": length_, "setup": setup if flag_setup == 0 else None,
                }, packet[64:64 + len_cap]
        pos += length


packets = [p for p in read_pcapng(pcap_path) if p[1]["xfer"] == 2]  # control transfers
if address is None:
    counts = collections.Counter(h["dev"] for _, h, _ in packets if h["setup"] and h["setup"][0] == 0x21 and struct.unpack_from("<H", h["setup"], 6)[0] == 60)
    address = counts.most_common(1)[0][0] if counts else None
packets = [p for p in packets if p[1]["dev"] == address]
# A completion carries the data read (IN); its setup is on the submission with the same URB id, so pair them
# by order: remember the last setup per endpoint direction.
rows = []
last_setup = {}
for t, h, payload in packets:
    if h["setup"] is not None:
        last_setup[h["ep"]] = h["setup"]
    setup = h["setup"] or last_setup.get(h["ep"])
    rows.append((t, h, setup, payload))


def describe(h, setup, payload):
    if setup:
        bm, req, value, index, wlen = struct.unpack("<BBHHH", setup)
        request = "bmReq=0x%02x bReq=0x%02x wValue=0x%04x wIndex=0x%04x wLength=%d" % (bm, req, value, index, wlen)
    else:
        request = "(no setup)"
    kind = {"S": "submit", "C": "complete", "E": "error"}.get(h["kind"], h["kind"])
    return "%-8s %s status=%d data[%d]=%s" % (kind, request, h["status"], len(payload), payload.hex(" "))


steps, current, header = [], None, []
for line in lines:
    m = re.match(r"STEP (\S+) BEGIN ([\d.]+)", line)
    if m:
        current = [m.group(1), float(m.group(2)), None, None, []]
        steps.append(current)
        continue
    m = re.match(r"STEP (\S+) END ([\d.]+) ret=(-?\d+)", line)
    if m and current:
        current[2], current[3] = float(m.group(2)), int(m.group(3))
        continue
    if line.startswith("PROBE END"):
        current = None
        continue
    if current is not None and line.startswith("  "):
        current[4].append(line)
    elif current is None:
        header.append(line)
connected = next((float(l.split()[1]) for l in lines if l.startswith("CONNECTED ")), None)


def section(title, start, end, extra=()):
    print("=" * 100)
    print(title)
    for line in extra:
        print(line)
    picked = [r for r in rows if start <= r[0] < end]
    counts = collections.OrderedDict()
    for _, h, setup, payload in picked:
        key = describe(h, setup, payload)
        counts[key] = counts.get(key, 0) + 1
    print("  packets: %d (%d distinct)" % (len(picked), len(counts)))
    for key, n in counts.items():
        print("  %4dx %s" % (n, key))


final = []
if "PROBE END" in log:
    final = [l for l in log.split("PROBE END", 1)[1].splitlines()[1:] if l.strip()]
for line in header:
    print(line)
print("Device address used: %s; control packets: %d" % (address, len(rows)))
if connected and steps:
    section("BASELINE (SDK traffic after connecting, before the first step)", connected, steps[0][1] - 0.05)
for i, (name, begin, end, ret, detail) in enumerate(steps):
    stop = steps[i + 1][1] - 0.05 if i + 1 < len(steps) else (end or begin) + 2.0
    section("STEP %s  ret=%s  (%.3f s)" % (name, ret, (end or begin) - begin), begin - 0.02, stop, detail)
if final:
    print("=" * 100)
    for line in final:
        print(line)
