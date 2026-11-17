
### 1. Identify the device

Find the vendor and product ID (`VVVV:PPPP`), and the bus and device numbers:

```bash
lsusb
```

`Bus 001 Device 022: ID VVVV:PPPP …` means bus 1, device 22. The device number changes every time
the device is reconnected, so check it again before each recording.

List the device's HID interfaces and their report descriptors:

```bash
for d in /sys/class/hidraw/hidraw*; do echo "$d -> $(readlink -f $d/device)"; done
```

```bash
xxd /sys/class/hidraw/hidrawN/device/report_descriptor
```

Interface 0 is usually the input one (keys, buttons, movement). The settings are usually on another
interface with a vendor-defined usage page (`06 xx ff`) and **feature reports**. Note its report IDs
and lengths.

### 2. Get access

This rule lets the logged-in user open only the settings interface, with no world-writable
permissions. Put your IDs and interface number in:

```
# /etc/udev/rules.d/70-mydevice.rules
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="vvvv", ATTRS{idProduct}=="pppp", ENV{MYDEVICE}="1"
SUBSYSTEM=="hidraw", ENV{MYDEVICE}=="1", ATTRS{bInterfaceNumber}=="01", TAG+="uaccess"
```

Reload the rules, then unplug and replug the device:

```bash
sudo udevadm control --reload
```

```bash
sudo udevadm trigger --subsystem-match=hidraw
```

The same rule lets Wine reach the device in step 3.

### 3. Run the official software and record it