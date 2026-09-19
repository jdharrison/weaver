#!/bin/sh
# Launch bounded woven-lab windows against one explicitly selected Woven node.

set -eu

usage() {
    cat <<'EOF'
Usage: scripts/local/run-woven-labs.sh [local|managed-local|remote|cloud] [count] [rate_hz]

Builds woven-lab once, then starts bounded GUI clients or explicit managed soak workers.
Start the shared Woven node separately, for example:
  cd ../woven && cargo run -p woven-server

Configuration:
  WOVEN_LAB_TARGET           local (default), managed-local, remote, or cloud
  WOVEN_LAB_MANAGED_URL      Managed local QUIC URL (default: quic://127.0.0.1:18082)
  WOVEN_LAB_CLOUD_URL        Required managed cloud quic://host:port; no default
  WOVEN_LAB_NAMESPACE_ID     Required managed-local/cloud namespace from Host Connect
  WOVEN_LAB_SESSION_ID       Required managed-local/cloud session from Host Connect
  WOVEN_LAB_REMOTE_URL       Explicit verified static remote quic://host:port; no default
  WOVEN_LAB_CA_PEM_FILE      Required managed/remote CA bundle, maximum 1 MiB
  WOVEN_LAB_TOKEN_FILE       Required managed/remote token file, owner-only on Unix
  WOVEN_LAB_DURATION_SECONDS Required remote/cloud or soak active duration, integer 1..600
  WOVEN_LAB_SOAK             Set exactly 1 for managed headless soak; omit for GUI
  WOVEN_LAB_SOAK_STARTUP_SECONDS     Soak startup/admission timeout (default: 60, max: 900)
  WOVEN_LAB_SOAK_FINAL_DRAIN_SECONDS Soak final echo drain (default: 5, max: 30)
  WOVEN_LAB_URL              Local QUIC URL (default: quic://127.0.0.1:8081)
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
  WOVEN_LAB_DELAY_SECONDS    Delay between launches (default: 0.4, range: 0..10)

Managed-local and cloud use products explicitly created through Host. Remote/shared
traffic requires operator approval. This script never provisions a node or product;
missing managed/remote settings fail before building or launching. The Rust client
performs bounded content, TLS and owner-only token-permission validation after launch.
Rates must be >0 and <=120 Hz per client; at most 16 clients are launched.
Escape closes a GUI window; Ctrl-C stops all launched clients. WEAVER_HEADLESS
is rejected by this launcher: that separate smoke mode never publishes lab traffic.
Soak requires WOVEN_LAB_SOAK=1 and managed-local/cloud, and emits one JSON result
per worker.

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

if [ "${WEAVER_HEADLESS+x}" = x ]; then
    echo "error: WEAVER_HEADLESS is a separate connection smoke and is not supported by this launcher" >&2
    exit 2
fi
if [ "${WOVEN_LAB_SOAK+x}" = x ]; then
    if [ "$WOVEN_LAB_SOAK" != 1 ]; then
        echo "error: WOVEN_LAB_SOAK must be exactly 1 when set; omit it for GUI mode" >&2
        exit 2
    fi
    soak=1
else
    soak=0
fi

target="${WOVEN_LAB_TARGET-local}"
case "${1:-}" in
    local|managed-local|remote|cloud)
        target="$1"
        shift
        ;;
    ''|[0-9]*) ;;
    *)
        echo "error: target must be local, managed-local, remote or cloud (or omit it and supply a client count)" >&2
        exit 2
        ;;
esac
if [ "$soak" = 1 ]; then
    case "$target" in
        managed-local|cloud) ;;
        *)
            echo "error: WOVEN_LAB_SOAK=1 requires target managed-local or cloud" >&2
            exit 2
            ;;
    esac
    if [ -z "${WOVEN_LAB_DURATION_SECONDS:-}" ]; then
        echo "error: soak runs require WOVEN_LAB_DURATION_SECONDS (1..600)" >&2
        exit 2
    fi
fi

case "$target" in
    local)
        url="${WOVEN_LAB_URL:-quic://127.0.0.1:8081}"
        ;;
    managed-local|cloud)
        if [ "$target" = cloud ]; then
            url="${WOVEN_LAB_CLOUD_URL:-}"
            if [ -z "$url" ]; then
                echo "error: cloud target requires WOVEN_LAB_CLOUD_URL; no local URL fallback" >&2
                exit 2
            fi
            if [ -z "${WOVEN_LAB_DURATION_SECONDS:-}" ]; then
                echo "error: cloud runs require WOVEN_LAB_DURATION_SECONDS (1..600)" >&2
                exit 2
            fi
        else
            url="${WOVEN_LAB_MANAGED_URL:-quic://127.0.0.1:18082}"
        fi
        if [ -z "${WOVEN_LAB_NAMESPACE_ID:-}" ] || [ -z "${WOVEN_LAB_SESSION_ID:-}" ]; then
            echo "error: managed targets require WOVEN_LAB_NAMESPACE_ID and WOVEN_LAB_SESSION_ID" >&2
            exit 2
        fi
        case "$WOVEN_LAB_NAMESPACE_ID" in
            0|0[0-9]*|*[!0-9]*)
                echo "error: managed scope IDs must be canonical nonzero decimal identifiers" >&2
                exit 2
                ;;
        esac
        case "$WOVEN_LAB_SESSION_ID" in
            0|0[0-9]*|*[!0-9]*)
                echo "error: managed scope IDs must be canonical nonzero decimal identifiers" >&2
                exit 2
                ;;
        esac
        if [ ! -f "${WOVEN_LAB_CA_PEM_FILE:-}" ] || [ ! -r "${WOVEN_LAB_CA_PEM_FILE:-}" ] ||
           [ ! -f "${WOVEN_LAB_TOKEN_FILE:-}" ] || [ ! -r "${WOVEN_LAB_TOKEN_FILE:-}" ]; then
            echo "error: managed targets require readable WOVEN_LAB_CA_PEM_FILE and WOVEN_LAB_TOKEN_FILE regular files" >&2
            exit 2
        fi
        if [ "$target" = cloud ]; then
            export WOVEN_LAB_CLOUD_URL="$url"
        else
            export WOVEN_LAB_MANAGED_URL="$url"
        fi
        ;;
    remote)
        url="${WOVEN_LAB_REMOTE_URL:-}"
        if [ -z "$url" ]; then
            echo "error: remote target requires WOVEN_LAB_REMOTE_URL; no local URL fallback" >&2
            exit 2
        fi
        if [ -z "${WOVEN_LAB_DURATION_SECONDS:-}" ]; then
            echo "error: remote runs require WOVEN_LAB_DURATION_SECONDS (1..600)" >&2
            exit 2
        fi
        if [ ! -f "${WOVEN_LAB_CA_PEM_FILE:-}" ] || [ ! -r "${WOVEN_LAB_CA_PEM_FILE:-}" ] ||
           [ ! -f "${WOVEN_LAB_TOKEN_FILE:-}" ] || [ ! -r "${WOVEN_LAB_TOKEN_FILE:-}" ]; then
            echo "error: remote runs require readable WOVEN_LAB_CA_PEM_FILE and WOVEN_LAB_TOKEN_FILE regular files" >&2
            exit 2
        fi
        export WOVEN_LAB_REMOTE_URL="$url"
        ;;
    *)
        echo "error: WOVEN_LAB_TARGET must be local, managed-local, remote or cloud" >&2
        exit 2
        ;;
esac
if [ "$#" -gt 2 ]; then
    usage >&2
    exit 2
fi

count="${1:-${WOVEN_LAB_COUNT:-4}}"
rate_override="${2:-}"
case "$count" in
    ''|*[!0-9]*)
        echo "error: client count must be a positive integer" >&2
        exit 2
        ;;
esac
if ! awk -v count="$count" 'BEGIN { exit !(count >= 1 && count <= 16) }'; then
    echo "error: client count must be between 1 and 16" >&2
    exit 2
fi

prefix="${WOVEN_LAB_NAME_PREFIX:-lab}"
names="${WOVEN_LAB_NAMES:-}"
rates="${rate_override:-${WOVEN_LAB_RATES:-${WOVEN_LAB_RATE_HZ:-10}}}"
speeds="${WOVEN_LAB_SPEEDS:-${WOVEN_LAB_ANGULAR_SPEED:-1.0}}"
delay="${WOVEN_LAB_DELAY_SECONDS:-0.4}"

# Validate caps before any build or process creation. Do not print URL/file values.
if [ "${WOVEN_LAB_DURATION_SECONDS+x}" = x ] && ! printf '%s\n' "$WOVEN_LAB_DURATION_SECONDS" |
    awk 'NR != 1 || $0 !~ /^[0-9]+$/ || $0 < 1 || $0 > 600 { bad=1 } END { exit bad }'; then
    echo "error: WOVEN_LAB_DURATION_SECONDS must be an integer in 1..600" >&2
    exit 2
fi
if [ "${WOVEN_LAB_SOAK_STARTUP_SECONDS+x}" = x ] && ! printf '%s\n' "$WOVEN_LAB_SOAK_STARTUP_SECONDS" |
    awk 'NR != 1 || $0 !~ /^[0-9]+$/ || $0 < 1 || $0 > 900 { bad=1 } END { exit bad }'; then
    echo "error: WOVEN_LAB_SOAK_STARTUP_SECONDS must be an integer in 1..900" >&2
    exit 2
fi
if [ "${WOVEN_LAB_SOAK_FINAL_DRAIN_SECONDS+x}" = x ] && ! printf '%s\n' "$WOVEN_LAB_SOAK_FINAL_DRAIN_SECONDS" |
    awk 'NR != 1 || $0 !~ /^[0-9]+$/ || $0 < 1 || $0 > 30 { bad=1 } END { exit bad }'; then
    echo "error: WOVEN_LAB_SOAK_FINAL_DRAIN_SECONDS must be an integer in 1..30" >&2
    exit 2
fi
if ! printf '%s\n' "$rates" | awk -F, '
    { for (i=1; i<=NF; i++) if ($i !~ /^[0-9]+([.][0-9]+)?$/ || $i <= 0 || $i > 120) bad=1 }
    END { exit bad || NR != 1 }'; then
    echo "error: every publish rate must be >0 and <=120 Hz" >&2
    exit 2
fi
if ! printf '%s\n' "$delay" | awk '
    $0 !~ /^[0-9]+([.][0-9]+)?$/ || $0 < 0 || $0 > 10 { bad=1 }
    END { exit bad || NR != 1 }'; then
    echo "error: launch delay must be in 0..10 seconds" >&2
    exit 2
fi
case "$url" in
    quic://*) ;;
    *) echo "error: endpoint must be an explicit QUIC URL" >&2; exit 2 ;;
esac
case "$url" in
    *'@'*|*'?'*|*'#'*|*'%'*) echo "error: endpoint must not contain credentials, query or fragment" >&2; exit 2 ;;
esac

cargo build -p woven-lab
binary="target/debug/woven-lab"

if [ ! -x "$binary" ]; then
    echo "error: woven-lab binary was not produced at $binary" >&2
    exit 1
fi

pids=""
trap 'if [ -n "$pids" ]; then kill $pids 2>/dev/null || true; fi' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
index=1
while [ "$index" -le "$count" ]; do
    if [ -n "$names" ]; then
        client="$(list_value "$names" "$index")"
    else
        client="$prefix-$index"
    fi
    rate="$(list_value "$rates" "$index")"
    speed="$(list_value "$speeds" "$index")"

    if [ "$soak" = 1 ]; then
        launch_mode=managed-soak
    else
        launch_mode=gui
    fi
    echo "launching mode=$launch_mode target=$target client=$client rate_hz=$rate steps_per_second=${WOVEN_LAB_STEPS_PER_SECOND:-auto} angular_speed=$speed duration_seconds=${WOVEN_LAB_DURATION_SECONDS:-unlimited-local}"
    WOVEN_LAB_TARGET="$target" \
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

status=0
for pid in $pids; do
    wait "$pid" || status=1
done
pids=""
exit "$status"
