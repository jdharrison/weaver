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

`woven-lab` is a local multi-client replication visualizer. Start a single
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
properties are intentionally local development controls, not a load-test
runner.

To launch a bounded sequence of variants quickly, after starting the Woven
node run:

```bash
scripts/local/run-woven-labs.sh 5 30
```

The optional second argument is a shared publish rate in Hz. The example above
builds once and starts five GUI clients at `30 Hz` with a `0.4` second interval. Use comma-separated variants for repeatable visual comparisons:

```bash
WOVEN_LAB_NAMES=alpha,bravo,charlie \
WOVEN_LAB_RATES=5,10,30 \
WOVEN_LAB_SPEEDS=0.5,1.0,3.0 \
WOVEN_LAB_DELAY_SECONDS=0.25 \
  scripts/local/run-woven-labs.sh 3
```

Run `scripts/local/run-woven-labs.sh --help` for all controls. The script does
not start, configure, or stop the Woven node; it only launches local lab
windows against the explicit loopback URL.

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
