
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
