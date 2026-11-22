# The service on different systems

The aim is one program that works on any Linux desktop. This page says how the background service is
installed, started, stopped and signalled on each kind of system, and what is not covered.

## Installing and starting

`tiny4lite-control service setup` (or the **Install…** button on the Device tab, which opens a terminal for it)
decides by what the system has:

| The system has | What is installed | How start, stop and remove work |
|---|---|---|
| A systemd user manager (`systemctl --user` answers): Fedora, Arch, Debian, Ubuntu, openSUSE, NixOS… | A user unit, `~/.config/systemd/user/tiny4lite-control.service`, enabled and started | `systemctl --user start` / `stop`; remove disables it, deletes the unit and reloads |
| No systemd user manager (Void, Alpine, Artix and Gentoo with runit, OpenRC, s6 or dinit; a container) | A desktop autostart entry, `~/.config/autostart/tiny4lite-control-service.desktop` | Start runs the service as a detached program that writes its log to `$XDG_RUNTIME_DIR/tiny4lite-control/service.log`; stop sends it SIGTERM using the process id in its status file there; remove stops it and deletes the entry |

Neither route needs a password. A password is asked for only to install missing programs, and only in the
terminal that setup opens.

The autostart entry is read by desktop environments (GNOME, KDE, Xfce, Cinnamon, MATE, LXQt). A bare window
manager does not read it, so for those `tiny4lite-control service install TARGET` prints the one line to add to
its own configuration, and never edits it: `hyprland` (Lua, 0.55 and later), `hyprland-legacy`, `niri`, `sway`,
`i3`, `river`, `mango`, `labwc`, `openbox`, `bspwm`, `shell` (any script, `.xinitrc`, `.xprofile`), and for
the init systems `openrc`, `runit`, `s6`, `dinit`, `cron`. A `udev` rule that starts the systemd unit when the
camera is plugged in is printed on request too. Run `service targets` for the list.

Only one service runs at a time, whichever way it was started: a lock file in the runtime directory decides,
and a lock left by a process that is gone is taken over. A second copy exits with code 6, and the systemd
unit has `RestartPreventExitStatus=6`, so systemd does not keep restarting it while another copy runs.
Installing the systemd unit first stops a copy that was started another way, and **Stop** stops a copy
whichever way it was started (systemd only stops what it started itself, so a hand-started one gets
SIGTERM too).

## Stopping, shutdown and logout

The service stops on SIGINT, SIGTERM and SIGHUP, which is what every init system, a logout and
`systemctl stop` do to a user program. If *Sleep when the computer shuts down* is on, it sends the sleep
command on its way out (a few tries, then it exits). The systemd unit has `TimeoutStopSec=15`.

## Suspend

*Sleep when the computer suspends* holds a logind **delay inhibitor lock** and watches the
`PrepareForSleep` signal on the system bus. This works wherever `org.freedesktop.login1` exists: systemd's
logind, and elogind on Void, Alpine, Artix and Gentoo. It uses two tools and no libraries:

* `dbus-monitor` (the dbus tools: `dbus`, `dbus-bin`, `dbus-tools` or `dbus-1`, by distribution)
* `systemd-inhibit`, or `elogind-inhibit` on elogind systems

The inhibitor runs `cat` on a pipe from the service. Closing the pipe ends it, so the lock goes away when
the service lets go of it, and also when the service ends in any way, even if it is killed: no helper is
left behind. If `dbus-monitor` ends (the system bus restarted), the service starts the watcher again, at
most once a minute.

## Requirements and the package managers

`tiny4lite-control requirements` lists what is installed, and `--install` installs what is missing:

| Needed | For | Package |
|---|---|---|
| `v4l2-ctl` | everything (required) | `v4l-utils` (`media-tv/v4l-utils` on Gentoo) |
| `dbus-monitor` | suspend (optional) | `dbus`, `dbus-tools` (Fedora), `dbus-1` (openSUSE), `sys-apps/dbus` (Gentoo) |
| `systemd-inhibit` or `elogind-inhibit` | suspend (optional) | part of the init system, not installed on its own |

Package managers known: apt, dnf, pacman, zypper, apk, xbps and emerge. The command is run with `sudo`,
`doas` or `pkexec` in front, whichever is found, and not at all when already root.

## Not covered

* Systems with neither logind nor elogind get no suspend handling.
* NixOS and other systems where programs are not installed by a package manager command: the install step
  prints what is missing and does not try.
* A window manager that does not read autostart entries needs its one line added by hand (see above).
* Running the AppImage from a root context is not tried; the unit and the entry point at the AppImage file
  itself, not at its temporary mount.
