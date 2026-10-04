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
│   ├── weaver-core/             # Runtime lifecycle, IDs, commands, snapshots
│   ├── weaver-app/              # Existing native runner and world composition
│   ├── weaver-app-core/         # Platform-neutral lab contract, assets, and input
│   ├── weaver-platform-desktop/ # Native winit/WGPU shell
│   ├── weaver-platform-web/     # Browser WASM/winit/WGPU shell
│   ├── weaver-render/           # Renderer-neutral vocabulary
│   ├── weaver-render-wgpu/      # WGPU graphics backend
│   ├── weaver-woven/            # Local Woven node and protocol adapter
│   └── weaver-worldline/        # Simulation time/frame adapter
├── examples/
│   ├── first-person-lab/ # Shared multiplayer room with desktop/web shells
│   ├── render-lab/       # Shared renderer feature lab with desktop/web shells
│   ├── space-lab/        # Shared solar-system lab with desktop/web shells
│   └── woven-lab/        # Deprecated native protocol/load diagnostic
├── prototypes/
│   └── first-person-web/ # Non-Weaver JavaScript/WebGL2 behavior mock
└── xtask/                # Repository-local lab build/run orchestration
```

## Current network status

`weaver-woven` uses the sibling `../woven` checkout's `woven-server`,
`woven-client`, and `woven-protocol` crates. `EmbeddedLocalNode` starts
Woven's development composition on ephemeral loopback ports and connects via
its public `WVN1` QUIC client path. The server assigns the connection's entity
when the client subscribes. `Loopback` connects to an explicitly configured
local Woven node, allowing multiple Weaver processes to share a development
session. `RemoteQuic` is an explicit verified-TLS path using the sibling client's
`ClientTlsConfig::from_ca_pem` and `Client::connect_with_tls`. `ManagedQuic` uses
Host-provisioned scope IDs, verified TLS, Bearer authentication, bounded admission,
and the managed Lite channel contract; it never sends legacy `JoinSession`.
Both verified modes require an explicit QUIC URL and bounded CA PEM/token files and
never fall back to development TLS or credentials. `weaver-app-core` exposes a bounded
realtime event/command seam. Desktop labs may drive it with `WovenRealtimeDriver` over
native QUIC. First-Person Lab's browser shell uses the sibling
`@signalweave/woven-client` over WebTransport and forwards opaque application payloads
to the same shared Rust scene; it does not compile native QUIC or a Woven server into
WASM. First-Person poses are fixed 25-byte little-endian binary payloads on channel
4 (`UnreliableSequenced`/`Ephemeral`) over actual datagrams. Every pose uses Woven's
atomic positioned-state API in an explicitly preconfigured Cartesian3D spatial subspace;
there is no reliable, unpositioned, or broadcast fallback. Clients cannot create ad-hoc
spaces, and server-defined bounds are authoritative. Chat and display-name profiles
remain reliable ephemeral JSON on channel 1 in the same spatial space. Both native and
browser realtime seams expose positioned unreliable bytes alongside existing string/JSON
paths. Managed Lite nodes/descriptors must permit channels 1/4 and negotiate positioned
state. Credentials are explicit and held in memory.

`woven-lab` is deprecated as an interactive example but retained for native protocol,
soak, and managed-admission diagnostics. Its remote/cloud GUI selection requires a
1–600 second wall-clock duration; the launcher caps workers at 16 and rates at 120 Hz
per client. `WOVEN_LAB_SOAK=1` remains a distinct managed-local/cloud-only headless
runner. See README for invocation and current limits; no cloud/shared-node validation
is implied.

Do not make unsupported modes appear to work, and do not couple Weaver to
`woven-core` in-process. A real network integration uses Woven's
`woven-protocol` and supported QUIC/WebTransport client path. Woven owns
session provisioning, identity/ownership, routing, channel definitions,
delivery/persistence policy, and bounded queues; Weaver owns the application
and presentation behavior around that connection.

## Graphics and UI direction

Read `docs/UI-ARCHITECTURE.md` and
`docs/adr/009-unified-rendering-and-scriptable-ui.md` before changing graphics or
component/UI contracts. The direction is accepted; implementation is deferred in
`docs/UI-IMPLEMENTATION-PLAN.md`, not implied shipped by these documents.

Initial built-in modes are dynamic/depth-tested 3D, flat X/Y fixed-orthographic
2D, and a screen-space UI overlay above scene content with its own stacking and
clipping. Share graphics infrastructure without forcing scenes into a widget
model. UI composition is scriptable; the language/runtime/framework is not yet
selected. Execution authority, GPU ownership, and bounded scheduling remain in
Rust/application services.

Support future presentation and input adapters through explicit contracts, but
do not bundle curved/diegetic surfaces, VR/OpenXR, or mobile integrations as part
of the initial work. Preserve platform text/accessibility requirements and
explicit WebGPU/WebGL2 capabilities; do not infer performance or sandboxing from
GPU acceleration or an embedded interpreter.

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

Cross-platform labs use the repository task runner:

```sh
cargo xtask check first-person --platform all
cargo xtask build render --platform web
cargo xtask run space --platform desktop
```

First-Person Lab uses the real Woven TypeScript WebTransport client in browsers. Other browser labs remain offline until they explicitly adopt the same realtime seam; browser targets must never compile or embed the native QUIC/server composition.

For wire/client changes, also consult and run the focused validation prescribed
by `../woven/AGENTS.md`.
