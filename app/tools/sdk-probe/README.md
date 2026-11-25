# SDK probe

Records which USB commands OBSBOT's SDK sends for each of its functions, so they can be implemented in this
app from our own captures (a captured command is
evidence, the SDK's header is only a map).

Nothing of OBSBOT's is in this folder: `probe.cpp`, `report.py` and `run.sh` are ours, and the build points at
a local copy of the SDK (`libdev_v2.1.0_8`, unzipped outside the repository). Do not add the SDK's library or
headers to the repository.

```bash
bash run.sh                       # reads, and changes that are written back exactly
bash run.sh --toggles             # also on/off settings whose state cannot be read (final states listed)
bash run.sh --toggles --moves     # also pans, zoom, presets, AI modes, sleep and wake (each undone)
bash run.sh --hidden              # only the library's functions the headers leave out (see inventory.py)
bash run.sh --hidden --toggles --moves
```

`--hidden` calls functions the SDK library exports but its headers do not declare (audio volume, noise
reduction, gamma, night mode, status light, auto-sleep time, hand-tracking area, the zone-tracking return
position and others). It finds each by its exact signature with `nm` and calls only those that take plain
numbers; anything taking an undocumented structure, or whose argument order is unknown, is skipped.
`python3 inventory.py` lists everything in the SDK, by device.

```bash
bash run.sh --round4 --moves      # return position written and read back, hand side, gesture bytes in the AI status
bash run.sh --round4 --sleep-images   # also downloads the three sleep picture thumbnails (run on its own)
bash run.sh --round5              # zone tracking switches with the AI status after each; start-up and saved positions written back as read
bash run.sh --round5 --toggles    # also deletes the last saved position and adds it back as it was
```

`report.txt` ends with a SUMMARY: each step's commands decoded (write/read, module, command, payload) with
the camera's replies, which is the part to compare.

It needs cmake, a C++ compiler, tshark, python3 and lsusb, and sudo for the USB capture. It stops this app's
background service during the run and starts it again afterwards; close the app's windows first. The output
folder is `~/obsbot-sdk-probe-<date>/` with `report.txt` (steps with their packets), `probe.log` and the raw
capture.

What it does to the camera is listed at the top of `probe.cpp`. It never factory-resets, adds, deletes, renames
or updates presets, sets or resets the start-up position, touches background images, the boot AI mode,
privacy mode or firmware.
