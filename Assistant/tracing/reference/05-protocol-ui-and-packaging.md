
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