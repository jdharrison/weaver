# Weaver

A next-generation, local-first Rust engine for interactive simulations, games, and XR—powered by Signalweave for distributed worlds and intelligence.

## Bootstrap milestone

This repository contains the initial vertical slice of Weaver: a runnable runtime that composes Worldline-backed simulation time, embedded Signalweave connectivity, and a WGPU renderer through a renderer-neutral snapshot seam.

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
│   ├── weaver-signalweave/ # Embedded Signalweave adapter
│   └── weaver-worldline/   # Worldline-backed time and frame adapter
├── examples/render-lab/    # Integrated rendering example
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

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
cargo build --workspace --all-targets
```

## Dependencies

Key pinned source dependencies:

- `signalweave-core` from `https://github.com/jdharrison/signalweave.git` at revision `a2fc2d3bab7884969d22e14e039070fcfd93e2bb`
- `simengine` (Worldline) from `https://github.com/jdharrison/worldline.git` at revision `21ace68928345f8581f660bffd0e50f181348199`

See `Cargo.lock` and individual `Cargo.toml` files for full dependency versions.

## License

Apache-2.0
