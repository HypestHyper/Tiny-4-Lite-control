# Building a Linux app for a device that only has Windows software

A guide for an AI assistant helping with this. It describes how a Linux settings app was built for a
USB gaming keyboard whose maker only offers Windows software. The same method works for many other
USB devices.

## Does it apply to my device?

It usually works when:

- the device plugs in by USB (or has a wired mode)
- the maker's software changes settings that are **stored on the device**, such as lighting, key
  maps, macros, DPI or profiles
- the device talks over standard USB HID. Most keyboards, mice, headsets and RGB controllers do.

It's harder or won't work when:

- the device only works wirelessly, or the settings only exist inside the maker's software
- the software uses its own kernel driver instead of HID
- the traffic is encrypted or signed, or the change needs a firmware update

## Ground rules

These keep the device safe. Agree them before starting.

1. **Never send a guessed command.** Only send bytes the official software was recorded sending,
   changing only fields whose meaning has been proven.
2. **Read before you write.** Find the read command first. A read can't change anything, and it
   shows the device's current state.
3. **One change per recording.** Change one setting in the official software, record it, compare.
4. **Leave alone what you don't understand.** If part of a write isn't decoded, send it exactly as
   recorded.
5. **Don't touch the typing or input interface.** Use only the device's configuration interface,
   and never read or log keystrokes or other input.
6. **Ask the person before adding any new write.** Show them the evidence first.
7. **No root app.** Give the logged-in user access to that one interface with a narrow udev rule
   instead.

## The process

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

**Install the tools.** On Arch:

```bash
sudo pacman -S --needed wine wireshark-qt
```

On Debian and Ubuntu the packages are `wine` and `wireshark`. On Fedora they're `wine` and
`wireshark`.

**Run the software.** Plain Wine works:

```bash
wine ~/Downloads/INSTALLER.exe
```

Bottles works too. If Bottles is a Flatpak, give it access to devices, then restart it:

```bash
flatpak override --user --device=all com.usebottles.bottles
```

If the software's windows or pop-ups ignore clicks, turn on Wine's virtual desktop. In Bottles it's
under Settings → Display → Virtual Desktop; otherwise use `winecfg` → Graphics → "Emulate a
virtual desktop". Before recording, check the device actually responds to the software.

**Allow USB recording** until the next reboot:

```bash
sudo modprobe usbmon
```

```bash
sudo setfacl -m u:$USER:r /dev/usbmon*
```

**Record one change.** Close your own app first, so only the official software talks to the device.
`usbmonN` is the bus number from `lsusb`; for bus 1 use `usbmon1`:

```bash
sudo dumpcap -i usbmon1 -w /tmp/change.pcapng
```

Make **one** change in the software, press Apply, wait a few seconds, then press Ctrl+C. Write down
exactly what you changed, from what to what.

**Keep only the settings traffic.** Settings travel as control transfers (`transfer_type 0x02`), and
keystrokes don't, so this also removes anything typed. Use the device number from `lsusb`:

```bash
sudo tshark -r /tmp/change.pcapng -Y "usb.device_address == 22 && usb.transfer_type == 0x02" -w ~/Downloads/change.pcapng
```

```bash
sudo chown $USER ~/Downloads/change.pcapng
```

```bash
ls -lh ~/Downloads/change.pcapng
```

A filtered recording is usually a few KB. If it's tiny, the device number was wrong, or the software
didn't send anything.

**Recordings that worked well:**

- **Start-up:** open the software with nothing changed. This shows the read commands and the full
  blocks it reads.
- **One setting through several values,** for example brightness 0, 1, 2, 3, 4, with a pause
  between each, so you can find the scale.
- **Colours:** set white, then red, green, blue, then white again. Each channel shows up in turn.
- **Key remaps:** remap one key, apply, then reset to default and apply.
- **Macros:** record a short, known macro, assign it to a key, then apply.
- **Profiles:** switch profile and apply.

If a recording comes out empty, change the setting away and back, assign it again, or press Apply
again. Some software only sends then.

### 4. Decode

List the recorded transfers with their timing:

```bash
tshark -r change.pcapng -T fields -e frame.time_relative -e usb.setup.bmRequestType -e usb.setup.bRequest -e usb.setup.wValue -e usb.setup.wLength -e usb.data_fragment
```

- `bRequest` 0x09 is SET_REPORT (a write) and 0x01 is GET_REPORT (a read). The high byte of `wValue`
  is the report type (3 = feature) and the low byte is the report ID.
- Look for a pattern such as a short command sent as a feature report, followed by a longer data
  block that is read or written.
- Compare the block before and after each single change, and map each setting to its bytes. A short
  Python script that prints the differing offsets between two blocks makes this quick.
- Note the gaps between transfers (`frame.time_relative`) and whether anything is written twice.
  Copy both.
- The official software's files, such as config `.ini` files and key tables, often explain an
  indexing scheme. Look through its install folder inside the Wine prefix.
- Some things can't be read back from the device, such as colours. The app has to keep its own copy.

**Check your reading on the real device.** Once you know the read command, have the app send only
that command and show the reply. Have the person change one setting in the official software, close
it, read again, and compare. This tests decoding without writing anything.

### 5. Build the app's protocol layer

- **Allowlist:** a function per approved write that checks the buffer looks exactly like the
  recorded one apart from the decoded fields. Refuse anything else.
- **Read, modify, write:** read the current block, change only the known fields, write it, read it
  back and check the change stuck.
- **Copy the official software's behaviour:** the same order, the same gaps between writes, and
  repeated writes if it does them. Keeping the device handle open between operations, as the
  official software does, can matter.
- **Make a backup** of the original settings before the first write.
- **Test with a fake device** that records writes. Tests should prove the bytes match the recordings.

### 6. Build the user interface

- Keep the UI separate from the bytes: the UI calls functions such as "set effect", never raw buffers.
- Do device I/O on a worker thread, and show "busy" states and plain error messages.
- Useful extras:
  - an undo that restores what was there before
  - profiles stored by the app
  - a command-line option to apply a profile, for desktop key bindings
  - a warning when the device isn't connected
  - an "Allow access" helper

### 7. Package it

An AppImage built on an older distribution, with its own Python and GTK bundled, runs on most current
distributions. Test it in CI on several distributions with a headless smoke test.

## Pitfalls we hit

- The recording filter used the old device number after a reconnect, so recordings came out empty.
  Check `lsusb` before every recording.
- The official software's windows ignored clicks under Wine until the virtual desktop was turned on.
- A setting needed a second table written alongside it. Without it, the change looked half-applied.
- A special key, such as Fn, switches the device's own layers. Never remap it.
- Some visual glitches, like LEDs flickering on save, also happen with the official software's
  exact bytes. Compare against the official software before trying to "fix" them.
- Macros needed balanced key presses and releases, or keys got stuck.

## How to work with the AI

- Give it the vendor and product ID, the descriptor, and a list of what the official software can do.
- Let it ask for one recording at a time, and describe exactly what you changed in each one.
- Expect it to say "I need a recording of X" instead of guessing. That's the safe behaviour.
- Review each new write before it's enabled.
