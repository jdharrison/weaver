# Weaver — Agent Reference

Read `../AGENTS.md` first for ecosystem boundaries, then use this document for
Weaver-specific work.

## What Weaver is

Weaver is a native, local-first Rust/WGPU runtime for high-interaction games,
simulations, XR, and applications. It owns simulation lifecycle, application
state, rendering, and operator interaction. It is intended to be responsive,
but no performance or safety-critical certification claim may be inferred from
that goal.

Weaver may eventually visualize or participate in flight/aeronautical and
autonomous-control domains, but it is **not currently a flight-control or
safety-critical system**. Do not add language or behavior that represents it
as certified, fail-operational, or suitable to control real vehicles.

## Workspace map

```text
weaver/
├── crates/
│   ├── weaver-core/        # Runtime lifecycle, IDs, commands, snapshots
│   ├── weaver-app/         # Native application runner and world composition
│   ├── weaver-render/      # Renderer-neutral vocabulary
│   ├── weaver-render-wgpu/ # WGPU graphics backend
│   ├── weaver-woven/       # Local Woven node and protocol adapter
│   └── weaver-worldline/   # Simulation time/frame adapter
└── examples/
    ├── render-lab/         # Integrated rendering example
    └── space-lab/
```

## Current network status

`weaver-woven` uses the sibling `../woven` checkout's `woven-server`,
`woven-client`, and `woven-protocol` crates. `EmbeddedLocalNode` starts
Woven's development composition on ephemeral loopback ports and connects via
its public `WVN1` QUIC client path. The server assigns the connection's entity
when the client subscribes. `Loopback` connects to an explicitly configured
local Woven node, allowing multiple Weaver processes to share a development
session. `RemoteQuic` is an explicit verified-TLS path using the sibling client's
`ClientTlsConfig::from_ca_pem` and `Client::connect_with_tls`. It requires an
explicit QUIC URL, bounded CA PEM/token files, and never falls back to development
TLS or credentials. `woven-lab` remote/cloud selection additionally requires a
1–300 second wall-clock duration; the launcher caps workers at 16 and rates at
120 Hz per client. Remote network operations respect the lab deadline and a
10-second per-operation timeout. See README for invocation and current limits;
no cloud/shared-node validation is implied.

Do not make unsupported modes appear to work, and do not couple Weaver to
`woven-core` in-process. A real network integration uses Woven's
`woven-protocol` and supported QUIC/WebTransport client path. Woven owns
session provisioning, identity/ownership, routing, channel definitions,
delivery/persistence policy, and bounded queues; Weaver owns the application
and presentation behavior around that connection.

## First milestone: Woven load-test client and UI

Build a Weaver-facing tool that drives **real, protocol-valid Woven client
traffic**. It is distinct from `woven/crates/woven-loadtest`, which is an
in-process routing benchmark rather than a remote or multi-instance load test.

Keep three layers separate:

1. **Script/configuration model** — versioned, serializable, validated and
   reproducible. It specifies target endpoint(s)/transport/credentials,
   namespace/session/space/channel, worker count, ramp-up, run duration,
   concurrency cap, cadence in Hz, payload generator/size, push/publish
   behavior, and applicable interest/routing setup.
2. **Bounded load engine** — owns connections, scheduling, traffic generation,
   cancellation, timeout/error handling, and measurement. It must not depend
   on the render frame loop for correctness or timing.
3. **UI** — edits/selects scripts; starts, stops, and observes runs; and shows
   per-target plus aggregate results. It does not implement the traffic loop.

For multiple target instances, use explicit target records and isolated worker
groups. They are independent Woven routing domains unless Woven implements a
real bridge/federation feature; never imply that a session spans instances.

### Test correctness and measurement

- A test script must honor the server's `ChannelDefinition`: the client cannot
  override delivery class, persistence class, payload limit, or coalescing-key
  requirements. Treat server rejections as results, not silent retries.
- Record the resolved script/configuration, random seed where used, tool and
  protocol versions, and timestamps with every result.
- Report attempted, sent, received, rejected, dropped/coalesced when
  observable, errors, disconnects, throughput, and latency distribution.
  Clearly label client-observed metrics; do not claim server-only state without
  an explicit server observation API.
- Bound every run: worker count, connection concurrency, message rate, payload
  size, duration, pending work, samples, and result retention. Provide an
  explicit stop/cancellation path and ensure cleanup disconnects clients.

## Safety and approval gates

Default to local/development Woven nodes and non-destructive channels. Running
against a shared, hosted, or production target requires explicit user approval
and explicit caps for rate, duration, payload, and concurrency. It may incur
cost or disrupt users. Never commit credentials, put tokens in scripts checked
into source control, or log credentials/payloads that may contain sensitive
data.

No cloud, IAM, DNS, secret, or production-deployment mutations without
explicit user approval.

## Validation

Run relevant checks after changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
cargo build --workspace --all-targets
```

The current headless renderer smoke test is:

```sh
WEAVER_HEADLESS=1 cargo run --example render-lab
```

For wire/client changes, also consult and run the focused validation prescribed
by `../woven/AGENTS.md`.
