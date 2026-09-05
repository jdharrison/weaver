#!/bin/sh
# Launch multiple local woven-lab windows against one already-running Woven node.

set -eu

usage() {
    cat <<'EOF'
Usage: scripts/local/run-woven-labs.sh [count] [rate_hz]

Builds woven-lab once, then starts a bounded sequence of local GUI clients.
Start the shared Woven node separately, for example:
  cd ../woven && cargo run -p woven-server

Configuration:
  WOVEN_LAB_URL              Shared QUIC URL (default: quic://127.0.0.1:8081)
  [count]                    Client count (default: 4, max: 16)
  [rate_hz]                  Shared rate override for every client
  WOVEN_LAB_COUNT            Client count when [count] is omitted (default: 4, max: 16)
  WOVEN_LAB_NAME_PREFIX      Generated client name prefix (default: lab)
  WOVEN_LAB_NAMES            Optional comma-separated client names
  WOVEN_LAB_RATES            Optional comma-separated publish rates in Hz
  WOVEN_LAB_SPEEDS           Optional comma-separated angular speeds in radians/second
  WOVEN_LAB_RATE_HZ          Fallback rate when WOVEN_LAB_RATES is unset (default: 10)
  WOVEN_LAB_STEPS_PER_SECOND Simulation rate (default: max(60, requested publish Hz); max: 240)
  WOVEN_LAB_PRESENT_MODE     `vsync` (default) or `no-vsync` for an uncapped surface
  WOVEN_LAB_ANGULAR_SPEED    Fallback speed when WOVEN_LAB_SPEEDS is unset (default: 1.0)
  WOVEN_LAB_DELAY_SECONDS    Delay between launches (default: 0.4)

When a comma-separated rates or speeds list is shorter than the client count,
its final value is reused for the remaining clients.
EOF
}

list_value() {
    printf '%s\n' "$1" | awk -F, -v position="$2" '{ if (position <= NF) print $position; else print $NF }'
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
esac

count="${1:-${WOVEN_LAB_COUNT:-4}}"
rate_override="${2:-}"
case "$count" in
    ''|*[!0-9]*)
        echo "error: client count must be a positive integer" >&2
        exit 2
        ;;
esac
if [ "$count" -eq 0 ] || [ "$count" -gt 16 ]; then
    echo "error: client count must be between 1 and 16" >&2
    exit 2
fi

url="${WOVEN_LAB_URL:-quic://127.0.0.1:8081}"
prefix="${WOVEN_LAB_NAME_PREFIX:-lab}"
names="${WOVEN_LAB_NAMES:-}"
rates="${rate_override:-${WOVEN_LAB_RATES:-${WOVEN_LAB_RATE_HZ:-10}}}"
speeds="${WOVEN_LAB_SPEEDS:-${WOVEN_LAB_ANGULAR_SPEED:-1.0}}"
delay="${WOVEN_LAB_DELAY_SECONDS:-0.4}"

cargo build -p woven-lab
binary="target/debug/woven-lab"

if [ ! -x "$binary" ]; then
    echo "error: woven-lab binary was not produced at $binary" >&2
    exit 1
fi

pids=""
index=1
while [ "$index" -le "$count" ]; do
    if [ -n "$names" ]; then
        client="$(list_value "$names" "$index")"
    else
        client="$prefix-$index"
    fi
    rate="$(list_value "$rates" "$index")"
    speed="$(list_value "$speeds" "$index")"

    echo "launching client=$client rate_hz=$rate steps_per_second=${WOVEN_LAB_STEPS_PER_SECOND:-auto} angular_speed=$speed url=$url"
    WOVEN_LAB_URL="$url" \
        WOVEN_LAB_CLIENT="$client" \
        WOVEN_LAB_RATE_HZ="$rate" \
        WOVEN_LAB_ANGULAR_SPEED="$speed" \
        "$binary" &
    pids="$pids $!"

    if [ "$index" -lt "$count" ]; then
        sleep "$delay"
    fi
    index=$((index + 1))
done

trap 'kill $pids 2>/dev/null || true' INT TERM
wait $pids
