
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