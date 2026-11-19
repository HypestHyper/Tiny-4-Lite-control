#!/usr/bin/env bash
set -euo pipefail

APP="${1:-app/target/release/tiny4lite-control}"
OUT="${2:-app/artifacts/headless-ui}"
mkdir -p "$OUT"
APP="$(realpath "$APP")"
OUT="$(realpath "$OUT")"

capture_one() {
    local state="$1"
    local page="$2"
    local name="$3"
    local with_scroll="${4:-0}"
    local config cache data
    config="$(mktemp -d)"
    cache="$(mktemp -d)"
    data="$(mktemp -d)"

    (
        export TINY4LITE_UI_TEST=1
        export TINY4LITE_UI_TEST_STATE="$state"
        export TINY4LITE_UI_TEST_PAGE="$page"
        export GDK_BACKEND=x11
        export GSK_RENDERER=cairo
        export NO_AT_BRIDGE=1 GTK_A11Y=none
        export XDG_CONFIG_HOME="$config"
        export XDG_CACHE_HOME="$cache"
        export XDG_DATA_HOME="$data"

        "$APP" >"$OUT/$name.log" 2>&1 &
        local app_pid=$!
        cleanup() {
            kill "$app_pid" 2>/dev/null || true
            wait "$app_pid" 2>/dev/null || true
        }
        trap cleanup EXIT

        local window=""
        for _ in $(seq 1 80); do
            window="$(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null | head -n1 || true)"
            if [[ -n "$window" ]]; then break; fi
            sleep 0.25
        done
        if [[ -z "$window" ]]; then
            cat "$OUT/$name.log"
            echo "GTK window did not appear for screenshot: $name" >&2
            exit 1
        fi

        sleep 1.5
        import -window "$window" "$OUT/$name.png"
        if [[ "$with_scroll" == "1" ]]; then
            # Scroll the page over its empty right margin, not over a control.
            xdotool mousemove --window "$window" 1370 930
            xdotool click --repeat 9 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-mid.png"
            xdotool click --repeat 6 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-focus.png"
            xdotool click --repeat 6 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-bottom.png"
            xdotool click --repeat 18 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-camera.png"
            xdotool click --repeat 12 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-features.png"
            xdotool click --repeat 18 --delay 70 5
            sleep 0.5
            import -window "$window" "$OUT/${name%-top}-tail.png"
        fi
    )
    rm -rf "$config" "$cache" "$data"
}

capture_one auto controls controls-auto-top 1
capture_one manual controls controls-manual-top 1
capture_one auto capture capture-page 0
capture_one auto diagnostics diagnostics-page 0

# Prove that a wheel event over a slider or dropdown does not alter it or
# scroll the containing page. The same pointer position is cleared before
# each screenshot so cursor rendering cannot create a false image difference.
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
    app_pid=$!
    trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
    window=""
    for _ in $(seq 1 80); do
        window="$(xdotool search --onlyvisible --name "Tiny 2 Lite Control" 2>/dev/null | head -n1 || true)"
        [[ -n "$window" ]] && break
        sleep 0.25
    done
    [[ -n "$window" ]] || { cat "$OUT/wheel-lock.log"; exit 1; }
    sleep 1.5
    xdotool mousemove 1590 1180
    import -window "$window" "$OUT/wheel-lock-before.png"

    # Sensitivity slider and theme dropdown are fixed in the header/first
    # section, making this check independent of the document's scroll height.
    xdotool mousemove --window "$window" 700 390
    xdotool click 5
    sleep 0.3
    xdotool mousemove 1590 1180
    import -window "$window" "$OUT/wheel-lock-after-slider.png"
    compare -metric AE -crop 1400x1040+0+0 "$OUT/wheel-lock-before.png" "$OUT/wheel-lock-after-slider.png" null: >/dev/null 2>"$OUT/wheel-slider-diff.txt"

    xdotool mousemove --window "$window" 1220 36
    xdotool click 5
    sleep 0.3
    xdotool mousemove 1590 1180
    import -window "$window" "$OUT/wheel-lock-after-dropdown.png"
    compare -metric AE -crop 1400x1040+0+0 "$OUT/wheel-lock-before.png" "$OUT/wheel-lock-after-dropdown.png" null: >/dev/null 2>"$OUT/wheel-dropdown-diff.txt"
)
rm -rf "$config" "$cache" "$data"

for image in "$OUT"/*.png; do
    test -s "$image"
    identify "$image"
done

echo "Headless GTK screenshots written to $OUT"
