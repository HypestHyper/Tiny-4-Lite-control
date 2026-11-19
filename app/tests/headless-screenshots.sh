#!/usr/bin/env bash
set -euo pipefail

run_capture_one() {
    local state="$1" page="$2" name="$3" with_scroll="$4" width="$5" height="$6"
    local config cache data window=""
    config="$(mktemp -d)"
    cache="$(mktemp -d)"
    data="$(mktemp -d)"

    (
        export TINY4LITE_UI_TEST=1
        export TINY4LITE_UI_TEST_STATE="$state"
        export TINY4LITE_UI_TEST_PAGE="$page"
        export TINY4LITE_UI_TEST_WIDTH="$width"
        export TINY4LITE_UI_TEST_HEIGHT="$height"
        export GDK_BACKEND=x11 GSK_RENDERER=cairo NO_AT_BRIDGE=1 GTK_A11Y=none
        export XDG_CONFIG_HOME="$config" XDG_CACHE_HOME="$cache" XDG_DATA_HOME="$data"

        "$APP" >"$OUT/$name.log" 2>&1 &
        local app_pid=$!
        cleanup() {
            kill "$app_pid" 2>/dev/null || true
            wait "$app_pid" 2>/dev/null || true
        }
        trap cleanup EXIT

        for _ in $(seq 1 80); do
            window="$(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null | head -n1 || true)"
            [[ -n "$window" ]] && break
            sleep 0.25
        done
        if [[ -z "$window" ]]; then
            cat "$OUT/$name.log"
            echo "GTK window did not appear for screenshot: $name" >&2
            exit 1
        fi

        sleep 1
        import -window "$window" "$OUT/$name.png"
        if [[ "$with_scroll" == "1" ]]; then
            # Scroll over the empty left gutter, not over a control.
            xdotool mousemove --window "$window" 30 930
            xdotool click --repeat 9 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-mid.png"
            xdotool click --repeat 6 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-focus.png"
            xdotool click --repeat 6 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-bottom.png"
            xdotool click --repeat 18 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-camera.png"
            xdotool click --repeat 12 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-features.png"
            xdotool click --repeat 18 --delay 70 5
            sleep 0.3
            import -window "$window" "$OUT/${name%-top}-tail.png"
        fi
    )
    rm -rf "$config" "$cache" "$data"
}

run_wheel_test() {
    local config cache data window=""
    config="$(mktemp -d)"
    cache="$(mktemp -d)"
    data="$(mktemp -d)"
    (
        export TINY4LITE_UI_TEST=1
        export TINY4LITE_UI_TEST_STATE=auto
        export TINY4LITE_UI_TEST_PAGE=controls
        export GDK_BACKEND=x11 GSK_RENDERER=cairo NO_AT_BRIDGE=1 GTK_A11Y=none
        export XDG_CONFIG_HOME="$config" XDG_CACHE_HOME="$cache" XDG_DATA_HOME="$data"
        "$APP" >"$OUT/wheel-lock.log" 2>&1 &
        local app_pid=$!
        trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
        for _ in $(seq 1 80); do
            window="$(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null | head -n1 || true)"
            [[ -n "$window" ]] && break
            sleep 0.25
        done
        [[ -n "$window" ]] || { cat "$OUT/wheel-lock.log"; exit 1; }
        sleep 1
        xdotool mousemove 1590 1180
        import -window "$window" "$OUT/wheel-lock-before.png"

        # Sensitivity slider: wheel input must neither change its value nor
        # scroll the containing page.
        xdotool mousemove --window "$window" 700 390
        xdotool click 5
        sleep 0.3
        xdotool mousemove 1590 1180
        import -window "$window" "$OUT/wheel-lock-after-slider.png"
        compare -metric AE -crop 1400x1040+0+0 "$OUT/wheel-lock-before.png" "$OUT/wheel-lock-after-slider.png" null: >/dev/null 2>"$OUT/wheel-slider-diff.txt"

        # Theme dropdown: ignore focus background, compare page pixels and the
        # selected label glyphs independently.
        xdotool mousemove --window "$window" 1220 36
        xdotool click 5
        sleep 0.3
        xdotool mousemove 1590 1180
        import -window "$window" "$OUT/wheel-lock-after-dropdown.png"
        compare -metric AE -crop 1400x960+0+80 "$OUT/wheel-lock-before.png" "$OUT/wheel-lock-after-dropdown.png" null: >/dev/null 2>"$OUT/wheel-dropdown-page-diff.txt"
        convert "$OUT/wheel-lock-before.png" -crop 50x24+1185+24 -colorspace Gray -threshold 50% +repage "$OUT/wheel-theme-before.png"
        convert "$OUT/wheel-lock-after-dropdown.png" -crop 50x24+1185+24 -colorspace Gray -threshold 50% +repage "$OUT/wheel-theme-after.png"
        compare -metric AE "$OUT/wheel-theme-before.png" "$OUT/wheel-theme-after.png" null: >/dev/null 2>"$OUT/wheel-dropdown-value-diff.txt"
    )
    rm -rf "$config" "$cache" "$data"
}

MODE="${1:-}"
if [[ "$MODE" == "--capture-one" ]]; then
    shift
    APP="$(realpath "$1")"
    OUT="$(realpath "$2")"
    mkdir -p "$OUT"
    run_capture_one "$3" "$4" "$5" "$6" "$7" "$8"
    exit 0
elif [[ "$MODE" == "--wheel-test" ]]; then
    shift
    APP="$(realpath "$1")"
    OUT="$(realpath "$2")"
    mkdir -p "$OUT"
    run_wheel_test
    exit 0
fi

APP="${1:-app/target/release/tiny4lite-control}"
OUT="${2:-app/artifacts/headless-ui}"
mkdir -p "$OUT"
APP="$(realpath "$APP")"
OUT="$(realpath "$OUT")"
SCRIPT="$(realpath "$0")"

capture_one() {
    xvfb-run -a -s "-screen 0 1600x1200x24" bash "$SCRIPT" --capture-one "$APP" "$OUT" "$@"
}

capture_one auto controls controls-auto-top 1 1440 1040
capture_one manual controls controls-manual-top 1 1440 1040
capture_one auto controls controls-default-size-top 0 900 680
capture_one auto capture capture-page 0 1440 1040
capture_one auto diagnostics diagnostics-page 0 1440 1040
xvfb-run -a -s "-screen 0 1600x1200x24" bash "$SCRIPT" --wheel-test "$APP" "$OUT"

for image in "$OUT"/*.png; do
    test -s "$image"
    identify "$image"
done

echo "Headless GTK screenshots written to $OUT"
