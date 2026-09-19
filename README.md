# Weaver

A next-generation, local-first Rust engine for interactive simulations, games, and XR—powered by Woven for distributed worlds and intelligence.

## Bootstrap milestone

This repository contains the initial vertical slice of Weaver: a runnable runtime that composes Worldline-backed simulation time, a local Woven node reached through the `WVN1` protocol, and a WGPU renderer through a renderer-neutral snapshot seam.

## Workspace layout

```text
weaver/
├── Cargo.toml
├── rust-toolchain.toml
├── crates/
│   ├── weaver-core/        # Runtime lifecycle, ids, commands, revisions, snapshot interfaces
│   ├── weaver-app/         # Native application runner and world composition
│   ├── weaver-render/      # Renderer-neutral vocabulary
│   ├── weaver-render-wgpu/ # WGPU graphics backend
│   ├── weaver-woven/       # Local Woven node and protocol adapter
│   └── weaver-worldline/   # Worldline-backed time and frame adapter
├── examples/render-lab/    # Integrated rendering example
├── examples/woven-lab/     # Local multi-client Woven replication visualizer
├── assets/                 # Repository-owned test assets
├── shaders/                # WGSL shaders
├── tests/                  # Additional integration tests
└── docs/adr/               # Architecture Decision Records
```

## Running

Headless smoke test:

```bash
WEAVER_HEADLESS=1 cargo run --example render-lab
```

Render Lab (requires a display and GPU):

```bash
cargo run --example render-lab
```

Controls:

- `Space` — pause/resume simulation
- `1`, `2`, `3` — set time multiplier to 0.5x, 1.0x, 2.0x
- `F1` — toggle coordinate frame visualization
- `F2` — toggle trajectory history visualization
- `Escape` — quit

## Woven Lab

`woven-lab` is a multi-client replication visualizer with development, Host-managed
local, and opt-in verified remote native QUIC paths. All Woven crates resolve from
the sibling `../woven` checkout. For development mode, start a single
local Woven node from the sibling checkout:

```bash
cd ../woven
cargo run -p woven-server
```

Then, in one or more Weaver terminals, launch clients against that loopback
node. Each window publishes its own authoritative cube rotation and shows the
server-assigned client entities in a grid:

```bash
WOVEN_LAB_URL=quic://127.0.0.1:8081 \
WOVEN_LAB_CLIENT=alpha WOVEN_LAB_RATE_HZ=10 WOVEN_LAB_ANGULAR_SPEED=1.0 \
  cargo run -p woven-lab
```

Launch additional windows with different `WOVEN_LAB_CLIENT`, rate, or angular
speed values. `WOVEN_LAB_RATE_HZ` accepts `0 < Hz <= 120` and configures the
lab's simulation step rate to at least the requested publish rate (with a
60 Hz minimum). Override it explicitly with `WOVEN_LAB_STEPS_PER_SECOND`
(`requested publish Hz <= steps/s <= 240`) when testing engine scheduling.
`WOVEN_LAB_ANGULAR_SPEED` accepts `0 <= radians/second <= 20`. Presentation
uses vsync by default; set `WOVEN_LAB_PRESENT_MODE=no-vsync` to request an
uncapped surface when testing a high-refresh display. Inactive cells remain
gray; active entities receive a stable pseudo-random color. These environment
properties configure a visual replication lab, not an independent load-test
runner. Publishing is tied to application updates; requested Hz is not a
throughput guarantee.

To launch a bounded sequence of variants quickly, after starting the Woven
node run:

```bash
scripts/local/run-woven-labs.sh 5 30
```

The arguments are `[local|managed-local|remote|cloud] [count] [rate_hz]`; the target defaults to
`WOVEN_LAB_TARGET` or `local` when unset. An explicit first target argument
wins over the environment. The legacy `5 30` form above still builds once and
starts five GUI clients at `30 Hz` with a `0.4` second interval; explicit
`scripts/local/run-woven-labs.sh local 5 30` does the same.
Use comma-separated variants for repeatable visual comparisons:

```bash
WOVEN_LAB_NAMES=alpha,bravo,charlie \
WOVEN_LAB_RATES=5,10,30 \
WOVEN_LAB_SPEEDS=0.5,1.0,3.0 \
WOVEN_LAB_DELAY_SECONDS=0.25 \
  scripts/local/run-woven-labs.sh 3
```

Run `scripts/local/run-woven-labs.sh --help` for all controls. The script does
not start, configure, or stop the Woven node; it only launches lab windows.

Both the launcher and direct binary accept
`WOVEN_LAB_TARGET=local|managed-local|remote|cloud`. `cloud` uses the managed
Bearer-admission path against an explicit Host-provisioned endpoint; `remote` uses
static development credentials. Neither target provisions resources. Direct local
runs still require `WOVEN_LAB_URL`; the launcher supplies `quic://127.0.0.1:8081` by default.
Local mode retains the loopback-only development client settings; embedded mode
remains unchanged.

### Managed local Host → Woven → Weaver QA

Start the complete disposable stack from `woven-host`:

```bash
npm run dev:local
```

Open `http://127.0.0.1:5173`, create a Lite product with enough allocated CCU for
the number of lab windows, and use **Connect** to reveal its native QUIC endpoint,
namespace ID, session ID, client token, and TLS CA certificate. Save the token to
an owner-only temporary file and the CA certificate to a separate temporary file;
never put the token in a URL, command argument, repository file, or terminal output:

```bash
install -m 600 /dev/null /tmp/woven-lab-token
install -m 600 /dev/null /tmp/woven-lab-ca.pem
# Edit those two files locally using the values revealed by Host.
```

Then run from `weaver`, replacing only the two ID placeholders:

```bash
WOVEN_LAB_NAMESPACE_ID='<namespace-id>' \
WOVEN_LAB_SESSION_ID='<session-id>' \
WOVEN_LAB_CA_PEM_FILE=/tmp/woven-lab-ca.pem \
WOVEN_LAB_TOKEN_FILE=/tmp/woven-lab-token \
  scripts/local/run-woven-labs.sh managed-local 2 10
```

The managed-local launcher defaults to `quic://127.0.0.1:18082`, the native QUIC
listener created by `dev:local`. Override it only with an explicit
`WOVEN_LAB_MANAGED_URL`. Each client performs verified TLS, Bearer authentication,
bounded managed admission, space subscription, server-assigned entity creation,
and bidirectional publishing on Lite channel 1 (`ReliableOrdered`/`Ephemeral`). It
never uses legacy `JoinSession`, the development token, or channel 2. A product's
allocated CCU controls immediate admission; excess windows wait in Woven's bounded
queue until capacity is available or the 60-second default admission deadline
expires. The launcher owns Ctrl+C and terminates queued or running child processes,
which releases their Woven connections without installing a process-global signal
handler in the adapter. Set `WOVEN_LAB_DURATION_SECONDS=1..600` for an overall
bounded run.

This validates the native Rust client path against the same local managed node that
Host provisioned. It does not validate the browser TypeScript/WebTransport client;
that remains a separate website/browser QA pass. Remove the temporary credential
files after QA and stop `dev:local` with Ctrl+C.

### Managed cloud QA (explicit operator approval required)

**Do not run against a shared, hosted, or production node without approval for
that endpoint, credential, rate, worker count and duration.** Traffic may incur
cost or affect other users. The launcher does not provision or mutate cloud
resources. Save the Host-provided CA certificate and scoped client token in the
protected temporary files described above, then run a bounded managed smoke:

```bash
WOVEN_LAB_CLOUD_URL='quic://approved-woven-host.example.invalid:8081' \
WOVEN_LAB_NAMESPACE_ID='<namespace-id>' \
WOVEN_LAB_SESSION_ID='<session-id>' \
WOVEN_LAB_CA_PEM_FILE=/tmp/woven-lab-ca.pem \
WOVEN_LAB_TOKEN_FILE=/tmp/woven-lab-token \
WOVEN_LAB_DURATION_SECONDS=30 \
  scripts/local/run-woven-labs.sh cloud 2 10
```

The cloud target requires every value above, uses verified TLS and Bearer
admission, and exercises the managed Lite channel contract. It never falls back
to local or static remote settings.

### Static secure remote runs (explicit operator approval required)

From the Weaver checkout, after an operator has supplied the existing server,
its CA certificate bundle and its scoped static credential file:

```bash
# PLACEHOLDERS: replace the .invalid hostname, port and both /absolute/path paths.
# The hostname/IP must match a SAN in the server certificate.
WOVEN_LAB_REMOTE_URL='quic://approved-woven-host.example.invalid:8081' \
WOVEN_LAB_CA_PEM_FILE='/absolute/path/to/approved-ca.pem' \
WOVEN_LAB_TOKEN_FILE='/absolute/path/to/approved-token-file' \
WOVEN_LAB_DURATION_SECONDS=30 \
  scripts/local/run-woven-labs.sh remote 2 10
```

Equivalent single-window invocation (same placeholders):

```bash
WOVEN_LAB_TARGET=remote \
WOVEN_LAB_REMOTE_URL='quic://approved-woven-host.example.invalid:8081' \
WOVEN_LAB_CA_PEM_FILE='/absolute/path/to/approved-ca.pem' \
WOVEN_LAB_TOKEN_FILE='/absolute/path/to/approved-token-file' \
WOVEN_LAB_DURATION_SECONDS=30 WOVEN_LAB_RATE_HZ=10 \
  cargo run -p woven-lab
```

- Remote selection requires `WOVEN_LAB_REMOTE_URL` with an explicit port; it has
  **no default address and never uses `WOVEN_LAB_URL` as a fallback**. Cloud
  selection independently requires `WOVEN_LAB_CLOUD_URL`. URLs must not contain
  credentials, paths, queries or fragments.
- `WOVEN_LAB_CA_PEM_FILE` must be a nonempty regular file, at most **1 MiB**,
  containing a valid CA PEM bundle. Only those roots are trusted. The sibling
  client's standard chain, validity and URL hostname/IP verification is used;
  there is no TLS bypass, insecure retry or development credential fallback.
- `WOVEN_LAB_TOKEN_FILE` must be a nonempty regular UTF-8 file, at most **4098
  bytes** including optional trailing CR/LF. After trimming trailing CR/LF, the
  credential must contain **32–4096 non-whitespace ASCII bytes**, matching Woven's
  static remote server. Unix group/other permissions must be denied (e.g. an
  operator-provided mode `0600` file). On other platforms protect it with ACLs.
  Tokens never belong in URLs, CLI arguments or checked-in scripts. Config Debug
  omits URLs/paths and redacts tokens; token bytes remain plaintext in memory
  without zeroization, as in the sibling client.
- `WOVEN_LAB_DURATION_SECONDS` is **required for remote and cloud runs**, integer
  **1–600**; optional locally with the same bounds. Validation occurs before connection
  and the launcher rejects missing/invalid caps before building or launching.
  Each client's monotonic deadline starts after config validation, before its
  world/connection is created; it is independent of simulation pause/time.
  Remote startup (DNS/TLS/auth/join/subscribe) and each network operation are
  bounded by the remaining duration and a 10-second timeout. Receive draining
  is capped at 128 envelopes per remote update. Expiry stops the application
  and drops/closes its client; timed-out writes close rather than reuse a
  potentially partial stream. Cancellation is cooperative, not a hard realtime
  process-kill guarantee: file I/O, renderer/driver work or OS stalls may delay
  exit. Escape stops one window, launcher Ctrl-C stops all its children.
- The launcher creates **1–16 workers**, each with **0 < rate <= 120 Hz**, and
  launch delay **0–10 seconds** (default 0.4). These are per-client caps, not an
  aggregate server allowance: 16 × 120 requests 1920 publishes/second. Choose
  lower approved totals for a shared node. Staggering means total launcher
  wall time can exceed a single client's duration (plus build/startup time).
- The fixed server contract is **namespace/session/space 1, epoch 1, channel 2,
  LatestValue/Stateful**, server-assigned entity/coalescing key, 64 KiB channel
  payload ceiling. Remote client names are limited to 1–64 UTF-8 bytes. The lab
  sends its small fixed cube-state schema, not arbitrary load scripts. This targets Woven's initial static remote composition, not
  production tenant auth, remote WebTransport or management HTTP. Server limits
  and rejections remain authoritative; no policy overrides or silent retries.
- `WEAVER_HEADLESS=1` still performs **at most 60 simulation steps and no lab
  publishing**, often exiting before the duration. It can check connection/
  subscription startup, but is **not a headless load test**. GUI runs require a
  display/GPU; startup failures return a failing exit status. Metrics are
  client observations/echo confirmations, not proof of
  delivery to every peer or server-side drop/coalescing statistics.

The implementation resolves the Rust client, protocol, and server crates directly
from the sibling Woven checkout. No successful cloud/shared-node run is claimed;
that remains an explicitly approved operator follow-up.

Focused checks (launcher tests use fake build/client commands, no connections):

```bash
python3 scripts/local/test-run-woven-labs.py
cargo test -p woven-lab
```

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
cargo build --workspace --all-targets
```

## Dependencies

Key pinned source dependencies:

- `woven-client`, `woven-protocol`, and `woven-server` from the sibling `../woven` checkout (its checked-out `main` revision is the integration source of truth)
- `simengine` (Worldline) from `https://github.com/jdharrison/worldline.git` at revision `21ace68928345f8581f660bffd0e50f181348199`

See `Cargo.lock` and individual `Cargo.toml` files for full dependency versions.

## License

Apache-2.0
