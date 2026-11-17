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