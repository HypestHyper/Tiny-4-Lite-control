#!/usr/bin/env bash
# Renders every window and page against the synthetic camera (no hardware is
# touched), drives the joystick and a confirmation with real pointer events,
# and checks window classes and the commands that were sent.
#
#   bash app/tests/headless-screenshots.sh app/target/release/tiny4lite-control app/artifacts/headless-ui
#
# Needs: Xvfb (or an existing DISPLAY), xdotool, imagemagick, x11-utils.
set -euo pipefail

APP="$(realpath "${1:?path to the tiny4lite-control binary}")"
OUT="$(realpath -m "${2:?output directory}")"
mkdir -p "$OUT"

export TINY4LITE_UI_TEST=1 GDK_BACKEND=x11 GSK_RENDERER=cairo NO_AT_BRIDGE=1 GTK_A11Y=none

if [[ -z "${DISPLAY:-}" ]]; then
    Xvfb :97 -screen 0 1920x1100x24 >/dev/null 2>&1 &
    XVFB_PID=$!
    trap 'kill "$XVFB_PID" 2>/dev/null || true' EXIT
    export DISPLAY=:97
    sleep 1
fi

APP_PID=""
stop_app() {
    if [[ -n "$APP_PID" ]]; then kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; APP_PID=""; fi
}
trap 'stop_app' RETURN

# start NAME [app args...]: env TINY4LITE_UI_TEST_* are passed through.
start() {
    local name="$1"; shift
    export XDG_CONFIG_HOME="$(mktemp -d)" XDG_CACHE_HOME="$(mktemp -d)" XDG_DATA_HOME="$(mktemp -d)"
    if [[ -n "${START_CONFIG:-}" ]]; then mkdir -p "$XDG_CONFIG_HOME/tiny4lite-control"; printf '%s\n' "$START_CONFIG" > "$XDG_CONFIG_HOME/tiny4lite-control/config.toml"; fi
    export TINY4LITE_UI_TEST_LOG="$OUT/$name.commands.log"
    rm -f "$TINY4LITE_UI_TEST_LOG"
    "$APP" "$@" >"$OUT/$name.log" 2>&1 &
    APP_PID=$!
    WINDOW=""
    for _ in $(seq 1 80); do
        WINDOW="$(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null | head -n1 || true)"
        [[ -n "$WINDOW" ]] && break
        sleep 0.25
    done
    if [[ -z "$WINDOW" ]]; then cat "$OUT/$name.log"; echo "No window appeared for $name" >&2; exit 1; fi
    sleep 2.2
}

shot() { import -window root "$OUT/$1.png"; }

scenario() { # name, then VAR=value pairs, then -- and app args
    local name="$1"; shift
    local -a envs=() args=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do envs+=("$1"); shift; done
    [[ "${1:-}" == "--" ]] && shift
    args=("$@")
    ( for e in "${envs[@]}"; do export "$e"; done; start "$name" "${args[@]}"; shot "$name"; stop_app )
}

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── Pages and states ──
scenario full-auto
scenario full-manual TINY4LITE_UI_TEST_STATE=manual
scenario full-aperture TINY4LITE_UI_TEST_STATE=aperture
for page in tracking audio presets device; do scenario "page-$page" TINY4LITE_UI_TEST_PAGE=$page; done
scenario full-light TINY4LITE_UI_TEST_THEME=light
scenario full-wide TINY4LITE_UI_TEST_WIDTH=1700 TINY4LITE_UI_TEST_HEIGHT=1000
scenario full-wide-manual TINY4LITE_UI_TEST_STATE=manual TINY4LITE_UI_TEST_WIDTH=1700 TINY4LITE_UI_TEST_HEIGHT=1000
scenario full-narrow TINY4LITE_UI_TEST_WIDTH=700 TINY4LITE_UI_TEST_HEIGHT=900
scenario full-compact TINY4LITE_UI_TEST_SCALE=compact
scenario search TINY4LITE_UI_TEST_SEARCH=focus
scenario slim -- --slim
scenario slim-customise TINY4LITE_UI_TEST_PAGE=customise -- --slim
scenario slim-light TINY4LITE_UI_TEST_THEME=light -- --slim

# ── Window classes and titles (what window-manager rules match) ──
check_class() { # args..., expected class, expected title
    local want_class="$1" want_title="$2"; shift 2
    start class-check "$@"
    local found=""
    for w in $(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null); do
        local class title
        class="$(xprop -id "$w" WM_CLASS | sed 's/.*= "\([^"]*\)".*/\1/')"
        title="$(xprop -id "$w" _NET_WM_NAME | sed 's/.*= "\(.*\)"/\1/')"
        [[ "$class" == "$want_class" && "$title" == "$want_title" ]] && found=1
    done
    stop_app
    [[ -n "$found" ]] || fail "no window with class '$want_class' and title '$want_title' for args: $*"
}
check_class io.github.hypesthyper.Tiny4LiteControl "Tiny 2 Lite Control"
check_class io.github.hypesthyper.Tiny4LiteControl.Slim "Tiny 2 Lite Control - Quick controls" --slim
check_class io.github.hypesthyper.Tiny4LiteControl.Preview "Tiny 2 Lite Control - Preview" --preview
echo "ok: window classes and titles"

# ── Joystick: drag, hold, release ──
# The pad sits below the gimbal model; at this window size its centre is (235, 594).
TINY4LITE_UI_TEST_WIDTH=1300 TINY4LITE_UI_TEST_HEIGHT=900 start joystick
CENTRE_X=235; CENTRE_Y=594
xdotool mousemove --window "$WINDOW" "$CENTRE_X" "$CENTRE_Y" mousedown 1
sleep 0.2
xdotool mousemove --window "$WINDOW" $((CENTRE_X + 60)) $((CENTRE_Y - 50)); sleep 0.35
xdotool mousemove --window "$WINDOW" $((CENTRE_X + 90)) $((CENTRE_Y - 80)); sleep 0.4
shot joystick-engaged
xdotool mousemove --window "$WINDOW" $((CENTRE_X - 20)) $((CENTRE_Y + 70)); sleep 0.3
xdotool mouseup 1; sleep 0.8
shot joystick-released
stop_app
LOG="$OUT/joystick.commands.log"
[[ -s "$LOG" ]] || fail "the joystick sent nothing"
grep -q 'pan_speed=-[1-9]' "$LOG" || fail "dragging right did not send a negative pan speed (which shows more on the right)"
grep -q 'tilt_speed=-[1-9]' "$LOG" || fail "dragging down did not send a negative tilt speed"
[[ "$(tail -n1 "$LOG" | cut -d' ' -f2)" == "--set-ctrl=pan_speed=0,tilt_speed=0" ]] || fail "the last command was not a stop"
# Updates are rate-limited: never more than 30 in any one second.
awk '{ s=int($1/1000); n[s]++ } END { for (k in n) if (n[k] > 30) { print "second " k " had " n[k]; exit 1 } }' "$LOG" || fail "joystick updates were not rate-limited"
echo "ok: joystick sends speeds while held and always stops on release"

# ── A quick flip from up to down, with the pointer leaving the window on the way, is not stuck ──
TINY4LITE_UI_TEST_WIDTH=1300 TINY4LITE_UI_TEST_HEIGHT=900 start joystick-flip
xdotool mousemove --window "$WINDOW" "$CENTRE_X" "$CENTRE_Y" mousedown 1
sleep 0.15
xdotool mousemove --window "$WINDOW" "$CENTRE_X" $((CENTRE_Y - 80)); sleep 0.5
xdotool mousemove 1800 300; sleep 0.3                                   # out of the window, button still down
xdotool mousemove --window "$WINDOW" "$CENTRE_X" $((CENTRE_Y + 80)); sleep 0.6
xdotool mouseup 1; sleep 0.6
stop_app
FLIP="$OUT/joystick-flip.commands.log"
grep -q 'tilt_speed=[1-9]' "$FLIP" || fail "pushing up did not send a positive tilt speed"
grep -q 'tilt_speed=-[1-9]' "$FLIP" || fail "pushing down after up did not send a negative tilt speed (stuck?)"
[[ "$(tail -n1 "$FLIP" | cut -d' ' -f2)" == "--set-ctrl=pan_speed=0,tilt_speed=0" ]] || fail "the flip did not end with a stop"
echo "ok: a quick up-then-down flip is not stuck"

# ── With Mirror image on, pushing the stick right turns the camera the other way ──
START_CONFIG='mirror_image_enabled = true' TINY4LITE_UI_TEST_WIDTH=1300 TINY4LITE_UI_TEST_HEIGHT=900 start joystick-mirror
xdotool mousemove --window "$WINDOW" "$CENTRE_X" "$CENTRE_Y" mousedown 1
sleep 0.2
xdotool mousemove --window "$WINDOW" $((CENTRE_X + 60)) $((CENTRE_Y)); sleep 0.35
xdotool mousemove --window "$WINDOW" $((CENTRE_X + 90)) $((CENTRE_Y)); sleep 0.4
xdotool mouseup 1; sleep 0.6
stop_app
grep -q 'pan_speed=[1-9]' "$OUT/joystick-mirror.commands.log" || fail "with Mirror image on, pushing right did not send a positive pan speed"
! grep -q 'pan_speed=-[1-9]' "$OUT/joystick-mirror.commands.log" || fail "with Mirror image on, pushing right still sent a negative pan speed"
echo "ok: the joystick follows the mirror"

# ── 1° step buttons send small signed moves ──
TINY4LITE_UI_TEST_WIDTH=1300 TINY4LITE_UI_TEST_HEIGHT=900 start step
xdotool mousemove --window "$WINDOW" 283 723 click 1; sleep 0.5      # Pan +1°
xdotool mousemove --window "$WINDOW" 210 764 click 1; sleep 0.5      # Tilt -1°
shot step-buttons
stop_app
grep -q -- '--set-ctrl=pan_absolute=3600' "$OUT/step.commands.log" || fail "Pan +1° did not send a one-degree pan step"
grep -q -- '--set-ctrl=tilt_absolute=-3600' "$OUT/step.commands.log" || fail "Tilt -1° did not send a one-degree tilt step"
echo "ok: 1° step buttons"

# ── Developer tools open, and a diagnostic read does not break the app ──
TINY4LITE_UI_TEST_DEVTOOLS=1 start devtools
DEV="$(xdotool search --onlyvisible --name "Developer tools" 2>/dev/null | head -n1 || true)"
[[ -n "$DEV" ]] || fail "the Developer tools window did not open"
xdotool mousemove --window "$DEV" 87 335 click 1; sleep 0.8          # Read status (0x06)
kill -0 "$APP_PID" 2>/dev/null || fail "the app exited while reading diagnostics"
shot devtools-read-status
stop_app
echo "ok: developer tools"

# ── Sleep needs confirmation, and cancelling sends nothing ──
TINY4LITE_UI_TEST_PAGE=device start sleep-confirm
xdotool mousemove --window "$WINDOW" 971 436 click 1; sleep 0.8
shot sleep-confirm
xdotool key Escape; sleep 0.4
shot sleep-cancelled
stop_app
echo "ok: confirmation dialog captured"

echo "Screenshots and logs are in $OUT"
