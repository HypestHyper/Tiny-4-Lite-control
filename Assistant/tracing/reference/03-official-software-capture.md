
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