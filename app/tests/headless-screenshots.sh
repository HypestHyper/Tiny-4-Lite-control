#!/usr/bin/env bash
set -euo pipefail

APP="\${1:-app/target/release/tiny4lite-control}"
OUT="\${2:-app/artifacts/headless-ui}"
mkdir -p "$OUT"
APP="$(realpath "$APP")"
OUT="$(realpath "$OUT")"

capture_one() {
    local state="$1"
    local page="$2"
    local name="$3"
    local with_scroll="\${4:-0}"
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
        export NO_AT_BRIDGE=1
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
            import -window "$window" "$OUT/\${name%-top}-bottom.png"
        fi
    )
    rm -rf "$config" "$cache" "$data"
}

capture_one auto controls controls-auto-top 1
capture_one manual controls controls-manual 0
capture_one auto capture capture-page 0
capture_one auto diagnostics diagnostics-page 0

for image in "$OUT"/*.png; do
    test -s "$image"
    identify "$image"
done

echo "Headless GTK screenshots written to $OUT"
