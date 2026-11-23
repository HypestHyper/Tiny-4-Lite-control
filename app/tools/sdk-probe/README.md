# SDK probe

Records which USB commands OBSBOT's SDK sends for each of its functions, so they can be implemented in this
app from our own captures (the evidence rules in `Assistant/AGENTS.md` still apply: a captured command is
evidence, the SDK's header is only a map).

Nothing of OBSBOT's is in this folder: `probe.cpp`, `report.py` and `run.sh` are ours, and the build points at
a local copy of the SDK (`libdev_v2.1.0_8`, unzipped outside the repository). Do not add the SDK's library or
headers to the repository.

```bash
bash run.sh                 # reads, and changes that are put back
bash run.sh --with-moves    # also a 10° pan and back
```

It needs cmake, a C++ compiler, tshark, python3 and lsusb, and sudo for the USB capture. It stops this app's
background service during the run and starts it again afterwards; close the app's windows first. The output
folder is `~/obsbot-sdk-probe-<date>/` with `report.txt` (steps with their packets), `probe.log` and the raw
capture.

What it does to the camera is listed at the top of `probe.cpp`: reads; AI auto zoom on then off; pan reverse
on then off; each limited-zone tracking limit set and reset to default; limited-zone tracking flipped and
back. Never: factory reset, adding or deleting presets, the start-up position, the camera's sleep timer,
voice control or firmware.
