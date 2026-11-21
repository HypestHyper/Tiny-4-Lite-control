# Troubleshooting notes

Things still to check on a real camera, and what the app does until then.

## To test: does the camera wake by itself when a program opens it?

The background service never sends the wake command. The idea is that the Tiny 2 Lite wakes by itself when
an application starts streaming from it, so a wake from the service would be a second, needless command.
This is not confirmed.

How to check, with the service running and the idle sleep timer short (1 minute):

1. Let the camera go to sleep through the timer (watch `tiny4lite-control service status`).
2. Open the camera in another program (a browser's camera test page, OBS).
3. Does the picture appear without pressing Wake in the app?

If it does not wake by itself, add a single wake on the first use after a sleep that the service sent, and
note the result here.

## To test: sleep on suspend

Turn on *Sleep when the computer suspends*, suspend and resume, and check that the camera went to sleep
(its light and gimbal) and that suspend was not held up for long. `systemd-inhibit --list` shows the
service's delay lock while it is waiting. Check on an elogind system too (Void, Artix).

## Changing resolution or frame rate while another program streams

The camera refuses a format change while a stream is open (VIDIOC_S_FMT returns EBUSY), and forcing one
breaks the other program's picture. So the window names the programs that hold the camera open and
saves the choice as pending; the service applies it when nothing has the camera open. A pending choice is
dropped after one failed try.

## To come back to: the missing-tools section

The package names and the way missing tools are handled have only been written down, not checked on each
distribution. To go through:

* The exact package that provides `dbus-monitor` on Debian and Ubuntu (`dbus` or `dbus-bin`), openSUSE
  (`dbus-1` or `dbus-1-tools`), Alpine, Void and Gentoo.
* What the window and the setup terminal say on a system with no package manager they know (NixOS), and
  whether to offer `nix profile install` there.
* A system with `dbus-monitor` but neither `systemd-inhibit` nor `elogind-inhibit`: suspend is skipped with a
  message in the service log, and the card only lists the tool as missing. Decide if the card should say
  that suspend will not work there.
* Whether a terminal that is found but cannot run commands (a few minimal ones) needs a fallback.
* How the systemd and non-systemd routes are described to the user in the window, not only in the docs:
  see `docs/service-and-distros.md`.
