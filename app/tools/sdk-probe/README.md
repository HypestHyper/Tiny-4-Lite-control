# SDK probe

Records which USB commands OBSBOT's SDK sends for each of its functions, so they can be implemented in this
app from our own captures (the evidence rules in `Assistant/AGENTS.md` still apply: a captured command is
evidence, the SDK's header is only a map).

Nothing of OBSBOT's is in this folder: `probe.cpp`, `report.py` and `run.sh` are ours, and the build points at
a local copy of the SDK (`libdev_v2.1.0_8`, unzipped outside the repository). Do not add the SDK's library or
headers to the repository.

```bash
bash run.sh                       # reads, and changes that are written back exactly
bash run.sh --toggles             # also on/off settings whose state cannot be read (final states listed)
bash run.sh --toggles --moves     # also pans, zoom, presets, AI modes, sleep and wake (each undone)
```

It needs cmake, a C++ compiler, tshark, python3 and lsusb, and sudo for the USB capture. It stops this app's
background service during the run and starts it again afterwards; close the app's windows first. The output
folder is `~/obsbot-sdk-probe-<date>/` with `report.txt` (steps with their packets), `probe.log` and the raw
capture.

What it does to the camera is listed at the top of `probe.cpp`. It never factory-resets, adds, deletes, renames
or updates presets, sets or resets the start-up position, touches background images, the boot AI mode,
privacy mode or firmware.
