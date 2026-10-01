# UI Implementation Plan

**Status: future implementation plan; no phase is complete.**

This document turns the accepted architectural direction into bounded, testable
work. It is not a statement that a retained UI framework, scripting runtime,
unified compositor, shader-surface API, or spatial UI has shipped. Every checklist
item below is intentionally unchecked. Code observations describe the working
checkout inspected on 2026-09-30; revalidate them before implementation as the
platform and labs evolve.

Companion documents, authored independently:

- [UI architecture](UI-ARCHITECTURE.md).
- [ADR 009: unified rendering and scriptable UI](adr/009-unified-rendering-and-scriptable-ui.md).

These links are coordination points, not a dependency on their contents being
available. This plan does not assert their publication or decision status. Resolve
any later disagreement explicitly rather than silently changing their scope.
Also retain the boundaries in [the ecosystem guidance](../../AGENTS.md),
[Weaver guidance](../AGENTS.md),
[ADR 006](adr/006-fixed-simulation-independent-rendering.md), and
[ADR 008](adr/008-deferred-features.md). Advancing UI scripting is a future
milestone beyond ADR 008's bootstrap deferral, not permission to scaffold every
other deferred subsystem.

## 1. Scope and architectural commitments

### Accepted direction, not implemented capability

1. **Share the low-level rendering contract.** Renderer-neutral vocabulary,
   WGPU-owned GPU resources, and a compositor should support scenes and UI without
   forcing a universal entity/component tree. A world may keep its simulation
   entities; retained UI may keep its own component/layout tree. They meet at
   validated render descriptions and resource handles, not shared domain rules.
2. **Keep three presentation semantics distinct.**

   | Domain | Coordinates and camera | Ordering and depth | Authority |
   | --- | --- | --- | --- |
   | 3D scene | World coordinates; dynamic camera | Explicit scene policy; depth testing for appropriate world geometry | Rust application/simulation |
   | 2D scene | Flat X/Y scene; fixed orthographic camera independent of the 3D camera | Explicit stable layer order; no depth by default | Rust application/graphics state |
   | Screen-space UI | Surface-local logical coordinates mapped to pixels | Drawn last above scenes; its own stacking and clipping; no scene depth | Retained UI semantics, validated by Rust |

   The compositor must also define the order of multiple scene submissions and
   their viewports. Flat graphics do not automatically acquire focus, layout,
   editing, accessibility, or component semantics. UI is not simply the 2D scene.
3. **Rust owns execution and authoritative state.** GPU submission, resource
   lifetime, application/simulation state, and load-engine scheduling remain in
   Rust. Scripts compose retained components and request allowed changes through
   bounded, batched, validated interfaces. They do not receive a WGPU device or
   unchecked mutable world access.
4. **Script authoring is above components.** TS/JSX is the preferred candidate,
   not a selected interpreter, compiler, framework, or React reconciler. A
   Material-like system is a useful model for tokens, component states, and
   interaction, not a requirement to use Material, MUI, React DOM, or one theme.
5. **GPU UI is portable through the existing shells.** The native winit/WGPU
   shell and browser canvas WGPU path are the initial targets. WebGPU is the
   preferred browser backend, with a deliberately limited WebGL2 fallback. DOM
   integration may supply semantic, text-input, or accessibility services; it is
   not the visual authority for shader-rich UI. Tauri, Electron, and similar
   wrappers are neither selected nor required. Mobile portability is an intent,
   not shipped support or a mobile SDK implementation milestone.
6. **Shader-rich content is first-class but bounded.** Custom WGSL materials and
   shader surfaces need explicit contracts, capability checks, diagnostics, and
   resource budgets. Effects and compute are optional by capability. WebGL2 does
   not guarantee compute. Shader validation is neither a GPU cost proof nor a
   sandbox for hostile content.
7. **Spatial presentation is an extension boundary only.** Preserve seams for
   content GPU texture ownership, presentation geometry/material, hit-to-UV
   mapping, input providers/multiple pointers, and independent surface poses and
   views. Do not implement built-in plane, cylinder, curved/spatial UI, VR,
   OpenXR, or mobile SDK adapters. A small test-only fake adapter will prove the
   contracts without constructing real presentation geometry.
8. **Do not alter ecosystem authority.** Woven integration remains through its
   public protocol/network clients; no `woven-core` coupling or wire changes.
   Woven Host remains a separate control plane. Reuse Host tokens/patterns only
   after an evidence-based audit; do not import Firebase, identity, billing, or
   entitlements into this UI foundation.

### Initial deliverable and exclusions

Build an incremental, offline showcase in the existing Render Lab: a depth-tested
3D scene with a dynamic camera, a separate fixed-orthographic 2D scene, overlay UI,
a script-authored button and editable text, a large virtualized list, a custom
shader visualization, and a selectively cached panel with animations. These are
staged outputs, not prerequisites for phase 1.

Excluded from initial deliverables: curved/VR demos, real spatial presentation
geometry, OpenXR or mobile SDK implementations, a full browser DOM replacement,
a general-purpose editor, arbitrary untrusted plugin execution, automatic
federation, and a new Woven load engine or network protocol. A UI may later edit
and observe a load configuration, but traffic scheduling belongs to its bounded
Rust engine, never to animation callbacks or render-frame cadence.

## 2. Grounded starting point and gaps

Paths in this section are relative to the Weaver repository root. These are
observations, not requests to rewrite unrelated work.

| Existing evidence | Consequence for implementation |
| --- | --- |
| [`crates/weaver-render/src/snapshot.rs`](../crates/weaver-render/src/snapshot.rs) provides one `SceneSnapshot`, one camera, separate `text` and `ui` lists, and only limited validation. [`camera.rs`](../crates/weaver-render/src/camera.rs) already supports perspective and orthographic projections. | Reuse the vocabulary, but explicitly model scene domains, pass/view ownership, ordering, and UI output. Orthographic projection alone is not an independent 2D scene/compositor. |
| [`crates/weaver-render-wgpu/src/render.rs`](../crates/weaver-render-wgpu/src/render.rs) clears/renders world content with a depth attachment, then renders UI/text without depth. UI extraction uses `filter_map` for top-level `UiElement::Rect`; images, UI text, and clipped children are not extracted. All prepared `snapshot.text` is drawn after rectangles. | Preserve the useful overlay pass. Add an ordered paint stream and recursive clipping rather than assuming the advertised enum is implemented. Interleaved text/images/panels must obey UI stacking, not a global text-last rule. |
| [`crates/weaver-render/src/ui.rs`](../crates/weaver-render/src/ui.rs) declares rectangles, images, text, clips, layers, anchors, and an `interactive` flag. | This is paint vocabulary, not a retained component/layout/focus/input system. Define those semantics above rendering; implement declared paint behavior deliberately. |
| [`crates/weaver-render-wgpu/src/ui.rs`](../crates/weaver-render-wgpu/src/ui.rs) uses `MAX_RECTS = 250`, truncates with `min`/`take`, uploads the selected rectangle data each nonempty render, and creates two bind groups each time. [`pipeline.rs`](../crates/weaver-render-wgpu/src/pipeline.rs) also fixes the WGSL array at 250; the shader does not use `layer` for ordering. | Replace silent truncation with correct bounded multi-batch rendering or explicit rejection. Synchronize CPU/WGSL binding sizes, reuse stable bindings, and define ordering on the CPU. Do not simply enlarge the uniform array past backend limits. |
| [`crates/weaver-render-wgpu/src/text.rs`](../crates/weaver-render-wgpu/src/text.rs) retains glyphon/cosmic-text buffers by run index and text/font size, clears them on resize, prepares text each frame with full-viewport bounds, and trims the atlas after presentation. `measure_text` builds a temporary font system/layout. | Preserve existing retention. Add stable identities and complete layout/style keys, per-element clip bounds, shared measurement caching, and bounded cache health telemetry. Do not claim all text is currently reshaped every frame or that retention already implements editing. |
| [`crates/weaver-render-wgpu/src/resource.rs`](../crates/weaver-render-wgpu/src/resource.rs) uploads RGBA textures with `TEXTURE_BINDING | COPY_DST`. | Uploaded textures are sampling/upload resources, not general render targets, storage textures, or readback sources. Introduce explicit usage/lifetime descriptors before offscreen rendering, compute, or copies. |
| `render.rs` currently batches sprites with the first registered texture. | Correct handle-based texture resolution is a prerequisite when UI images and 2D content share the resource/composition path; multi-texture tests must not accidentally pass with one texture. |
| [`crates/weaver-app-core/src/lib.rs`](../crates/weaver-app-core/src/lib.rs) supplies `WeaverApp`, assets, camera controllers, and movement/look/actions in `InputFrame`. It currently imports WGPU-backend `Vertex`. | Extend existing app/input seams carefully. The contract is platform-neutral in purpose, not fully backend-independent today. Do not demand a wholesale asset refactor to begin UI work. |
| [`crates/weaver-platform-desktop/src/lib.rs`](../crates/weaver-platform-desktop/src/lib.rs) and [`crates/weaver-platform-web/src/lib.rs`](../crates/weaver-platform-web/src/lib.rs) translate camera input and call `app.update` from redraw. | General UI pointer/focus/text/IME routing and UI-versus-camera consumption are gaps. Frame-driven demo updates do not prove an authoritative backend is isolated from script stalls. |
| [`crates/weaver-render-wgpu/src/context.rs`](../crates/weaver-render-wgpu/src/context.rs) probes browser WebGPU with a timeout, exposes adapter/device objects, clamps surface size, and has a separate headless GPU context. [`Cargo.toml`](../crates/weaver-render-wgpu/Cargo.toml) enables `webgl` on WASM. Windowed device creation requests adapter limits, whereas headless creation uses configured limits. | Build capability profiles from the actual device/backend, not desktop assumptions or the config field alone. Audit limit negotiation in scope. Headless GPU setup is not yet a general offscreen `WgpuRenderer` API. |
| [`crates/weaver-core/src/commands.rs`](../crates/weaver-core/src/commands.rs) and [`events.rs`](../crates/weaver-core/src/events.rs) wrap `Box<dyn Any + Send + Sync>`. [`revision.rs`](../crates/weaver-core/src/revision.rs) uses `u64`. | Internal commands/events are not a serializable scripting ABI. Map a versioned DTO boundary to typed Rust commands/events; preserve exact `u64` values as canonical decimal strings across languages. |
| [`crates/weaver-app/src/headless.rs`](../crates/weaver-app/src/headless.rs) provides a simulation runner with bounded-step and shutdown seams. [`examples/woven-lab/src/main.rs`](../examples/woven-lab/src/main.rs) has headless/soak dispatch. | Preserve backend paths without a script/UI dependency. Prove scheduling isolation with offline fakes; these existing entry points are not permission to run live network/soak tests. |
| [`examples/render-lab/src/lib.rs`](../examples/render-lab/src/lib.rs), its native entry point, and browser assets use the shared app/shell contracts. [`xtask/src/main.rs`](../xtask/src/main.rs) implements desktop/web checks and static web packaging. | Extend the existing lab and workflows incrementally rather than inventing a committed UI crate/application layout or a new desktop wrapper. |

No scripting engine or retained semantic UI implementation was established by
this inspection. No Host token mapping was audited, and no existing token reuse
is assumed. The companion documents were not used as implementation evidence.

## 3. Work boundaries and contract-first coordination

Prefer modules in the existing crates. The ownership below is a planning split,
not a commitment to new crate names or file scaffolding. Extract a crate only
when a concrete implementation demonstrates a dependency/reuse need and records
that decision. Preserve existing platform work, APIs, and lab behavior through
small compatible migrations where possible.

| Work lane | Likely existing change area | Owns; must not own |
| --- | --- | --- |
| Render contract | `crates/weaver-render/src/{snapshot,camera,ui,id}.rs` and adjacent modules | Render descriptions, view/pass/order/resource identifiers, capability requirements; not retained widgets, interpreter state, or simulation rules. |
| GPU execution | `crates/weaver-render-wgpu/src/{context,render,resource,pipeline,ui,text,sprite}.rs` | Compositor, batching, caches, textures, materials, effects, upload/readback execution; not script authority. |
| Retained UI semantics | Modules within `crates/weaver-app-core` initially | Component identity, layout, styles, input/focus, bindings, accessibility descriptions; lowers to the render contract without exposing WGPU to scripts. |
| Script/DTO adapter | Distinct modules within `crates/weaver-app-core` initially; typed mapping at `weaver-core`/`weaver-app` boundaries only as needed | Authoring/runtime adapter, DTO validation, batching and subscriptions; no traffic loop or mandatory interpreter dependency in headless execution. |
| Platform services | `crates/weaver-platform-desktop/src/lib.rs`, `crates/weaver-platform-web/src/lib.rs` | Lifecycle, input normalization, text/IME/clipboard/accessibility services, worker/process integration where proven feasible; not layout policy or a second visual renderer. |
| Showcase and evidence | `examples/render-lab`, existing lab tests/assets, `xtask` only for justified validation improvements | Offline fixtures, staged demonstrations, benchmark reports; no Host/cloud/shared-target changes. |

The lanes are intentionally disjoint. Within a lane, sequence edits to shared
files; the phase list is not a request for parallel edits to `render.rs` or the
platform shells. After contract review, render-contract and GPU work can proceed
alongside a retained-UI prototype using a fake paint sink. Runtime feasibility
spikes can use a fake retained host. Integrate only after their respective gates.

### Contracts to agree before broad implementation

- **Presentation:** domain, viewport, camera ownership, clear/load behavior,
  depth policy, stable order, logical/physical scale, color space and alpha
  convention. Independent scene views must not overwrite each other's camera
  state. UI clipping/stacking must match hit testing.
- **Paint:** ordered rectangles/images/text/material surfaces; stable identities,
  clip stack, dirty revision/ranges, resource references, and diagnostic errors.
  Batching may merge compatible adjacent operations; it may not reorder
  transparent content across stacking/clip/material boundaries.
- **Resource:** generational or otherwise stale-safe handles, ownership/borrowing,
  usage flags, dimensions/format/sample count, budgets, release/resize/device-loss
  behavior. Sharing means sampling a GPU resource where valid, not copying it by
  default. Resource revisions must invalidate dependent bindings/caches.
- **Retained semantics:** component keys/lifetime, local state versus authoritative
  bindings, layout/paint invalidation, focus, capture, scroll, text editing, and
  accessibility descriptions. Design-system choice does not change GPU ABI.
- **Cross-language DTO, if used:** schema version, tagged requests/events,
  correlation and source revision, allowed property/action types, errors,
  subscription lifetime, and negotiated limits. `u64` IDs/revisions/counters use
  canonical exact decimal strings, not JS `Number`; validate range and reject
  lossy values. Keep other typed identifiers explicitly typed; do not assume all
  internal IDs have the same representation. Internal `Any` wrappers may remain.
- **Backpressure:** bound batch operations and bytes, tree depth/node count,
  pending requests, subscriptions, event retention, script work, and resources.
  Validate before applying a batch and define atomicity. Coalesce replaceable
  telemetry/state by key only; do not silently discard edits or critical actions.
  Report admission failures and stale revisions. Backend stop/cancellation has a
  Rust-owned priority path independent of script responsiveness.

The UI may animate visual uniforms at presentation cadence. It must not publish
network traffic at that cadence. Any load configuration crossing this boundary
must retain server channel-policy constraints and validation in the authoritative
engine. This plan does not introduce those network DTOs or change Woven bindings.

## 4. Dependency-ordered milestones

All phases are **deferred/unimplemented**. Advance a phase only with its evidence
and stop/go review. Reviews may revise scope; they must not mark unsupported
capabilities as successful fallbacks.

### Phase 0 — Inventory, reproducible baseline, and contract review

**Dependencies:** none. **Primary lanes:** evidence and contract review.

- [ ] Recheck the working-tree inventory and preserve all unrelated work; record
  the source revision plus local-change provenance used for results.
- [ ] Establish bounded offline fixtures for static UI, mixed scene/UI ordering,
  changing text, scrolling data, animations, shader compilation, and synthetic
  input/backend events. Record fixture sizes, seed, duration, warm-up, and limits.
- [ ] Measure the existing Render Lab and rectangle/text paths before replacing
  them; collect baseline results using the protocol in section 6.
- [ ] Draft the contracts in section 3 and capability profiles for desktop,
  browser WebGPU, and browser WebGL2. Audit actual WGPU limits and existing
  uniform-array sizes; a configured limit is not evidence of device behavior.
- [ ] Identify which app work is presentation-local and which authoritative work
  needs independent scheduling; preserve headless behavior and cancellation.

**Acceptance evidence:** a reproducible offline baseline with hardware/backend
metadata; a contract review resolving order/depth/scale/resource ownership; a
list of supported, unsupported, and not-yet-measured capabilities. Compile checks
are recorded separately from GPU runtime tests. Missing hardware/tooling is
reported as blocked, not passed.

**Stop/go:** do not promise frame rate, latency, or a script engine. Stop any
proposal that unifies world/UI authority, assumes compute on WebGL2, or needs
cloud/network traffic to establish the UI baseline.

### Phase 1 — Shared scene contract and minimal compositor

**Dependencies:** phase 0 contracts. **Primary lanes:** render contract and GPU.

- [ ] Add the smallest explicit domain/view/pass contract needed for a dynamic
  3D scene, fixed-orthographic flat X/Y 2D scene, and screen overlay. Prefer an
  adapter from current `SceneSnapshot` to avoid an all-at-once lab migration.
- [ ] Define deterministic scene submission, depth/load/clear behavior, stable
  layer tie-breaking, and viewport/camera independence. Keep screen UI last and
  free of scene depth regardless of scene camera motion or depth values.
- [ ] Introduce resource descriptors/capability requirements and lifetime rules
  sufficient for future materials and offscreen targets. Do not build an effects
  graph or allocate a texture for every panel yet.
- [ ] Extend Render Lab with a simple 2D scene and basic Rust-authored overlay
  beside the existing 3D content. Reuse shared desktop/web app implementations.

**Acceptance tests:** depth-tested overlapping 3D objects change occlusion with
camera motion; 2D objects retain fixed orthographic projection and explicit
layer order, without a depth attachment by default; the overlay remains above
both. Resizing/viewports do not couple the two cameras. Trace/pass tests prove
UI has no scene depth. Existing first-person/render/space checks still compile
for desktop and WASM; runtime results are separately collected where available.

**Stop/go:** reject a universal semantic/entity tree or a compositor that relies
on UI Z coordinates to compete with scene geometry. Keep the phase deliverable
to this small scene/overlay slice; scripting and the full showcase are not gates.

### Phase 2 — Correct ordered UI paint and reusable GPU batches

**Dependencies:** phase 1. **Primary lanes:** GPU and render contract.

- [ ] Lower rectangles, images, UI text, and nested rectangular clips into one
  ordered paint stream. Define anchor resolution, stable layers/stacking, clip
  intersection, and per-run text bounds. Preserve diagnostic text via an explicit
  overlay adapter rather than silently giving all text highest priority.
- [ ] Resolve textures by handle; prove distinct images/sprites use distinct
  resources. Batch compatible adjacent operations while respecting alpha, clip,
  texture, and material order. Specify color/alpha behavior with pixel fixtures.
- [ ] Replace the silent 250-rectangle ceiling with bounded multi-batch rendering
  appropriate to device limits, or an explicit visible admission error at a
  separately documented total limit. Never silently truncate admitted content.
- [ ] Retain buffer capacity and bind groups until their resources change. For
  multiple batches, ensure distinct valid data ranges/buffers/offsets: repeated
  writes to the same bound range before submission must not make every draw see
  the last batch. Use portable uniform/vertex approaches where storage buffers
  are unavailable.
- [ ] Preserve glyphon/cosmic-text retention; add stable paint/text identities,
  complete shaping/layout cache keys, cached measurement, clip-aware preparation,
  and bounded atlas/cache eviction diagnostics. Track dirty paint/upload ranges
  from the start without claiming a universal zero-work frame.

**Acceptance tests:** boundary fixtures at 249, 250, 251, and several batches show
all admitted items exactly once and distinct last-batch data. Mixed text/image/
rectangle overlap and nested/empty clips match golden results within documented
backend/font tolerances. Stable content reuses bind groups and text layout caches;
resource replacement/resize invalidates the appropriate caches. Invalid/stale
handles and excess admission produce errors, not missing content. Test baseline
paint on both WebGPU and WebGL2; do not substitute desktop success for fallback
coverage.

**Stop/go:** ordering correctness comes before draw-count reduction. Reject a
larger hardcoded uniform array as the sole fix, or optimization that moves all
text/images after panels. If glyphon interleaving requires multiple preparations,
measure its behavior and atlas lifetimes before selecting a batching strategy.

### Phase 3 — Retained components, layout, input, and modular design systems

**Dependencies:** phase 2 paint contract; fake-sink development may start earlier.
**Primary lanes:** retained semantics and platform services.

- [ ] Implement a limited retained tree with stable keys and deterministic
  create/update/remove behavior. Start with panel, row/column/stack layout,
  label/image, button, editable text, and scroll container. Do not attempt full
  CSS or a universal world/UI component model.
- [ ] Define constraints, padding, alignment, overflow, logical pixels/scale,
  font metrics, and dirty layout-versus-paint-versus-binding propagation. Keep
  measurement shared with text shaping. Make unsupported layout features explicit.
- [ ] Separate semantic components, styling/tokens, and GPU paint. Provide a
  small default theme and a second contrasting token set to prove replaceability.
  Document button/input states and keyboard interaction; do not couple components
  to Material/MUI/React DOM. Audit Host patterns only if reuse is proposed, with
  specific evidence and no Host control-plane dependencies.
- [ ] Normalize pointer IDs, positions/buttons/scroll, keyboard/text/composition,
  focus and pointer capture in the existing shells. Retained hit testing uses
  the same stacking/clip geometry as paint; consumed input must not also move the
  camera or trigger global lab shortcuts while editing.
- [ ] Implement bounded editable text, caret/selection, committed text versus IME
  preedit, deletion/navigation and clipboard requests through platform services.
  Handle Unicode graphemes/shaping without byte-index assumptions.
- [ ] Define semantic/accessibility descriptions and platform-provider capability
  reporting. Evaluate optional browser DOM text/semantic assistance where needed;
  keep component state, focus, and canvas rendering authoritative. Do not imply
  native screen-reader support or accessibility compliance merely from a schema.

**Acceptance tests:** keyed updates preserve intended local state and remove
resources/subscriptions; unchanged subtrees avoid unrelated layout work; token
changes repaint the intended components without rebuilding world state. Keyboard
activation/focus order, clipping-aware hits, capture release on removal/focus
loss, and editing-versus-camera arbitration are deterministic. Native/browser
text fixtures cover Unicode selection and IME preedit/commit/cancel; clipboard
failure is surfaced. Semantic-provider tests confirm role/value/focus consistency
and explicitly report unavailable platform services. Add Rust-authored button
and editable-text controls to the lab before any script dependency.

**Stop/go:** review layout-library versus focused in-house layout with measured
fit and dependency impact. Block a scripting host that would bypass component
validation. If text/IME or accessibility integration is incomplete, narrow the
supported interaction contract visibly; do not call the result production-ready.

### Phase 4 — Scripting feasibility gate, then a bounded authoring bridge

**Dependencies:** phases 0/3 contracts. Candidate spikes may use a fake retained
host while phases 1–3 proceed; production integration waits for phase 3.
**Primary lane:** script/DTO adapter, with platform isolation support.

- [ ] Run small, equivalent offline spikes for the candidates in section 5:
  create/update/remove a component tree, a button callback, text binding, a large
  logical collection, errors, and an intentionally stalled script. Do not select
  packages or a committed crate layout before this gate.
- [ ] Compare native and WASM/browser feasibility, including build size/startup,
  bounded runtime interruption, memory/GC, FFI serialization, debug/source mapping,
  offline bundling, and maintenance. TS needs compilation/type checking into JS;
  JSX needs a transform and host semantics, not a GPU renderer by itself.
- [ ] Record a decision before adding a runtime/framework dependency. Prefer
  TS/JSX only if the evidence supports the required isolation and portable host.
  A thin JSX factory over retained components does not require React.
- [ ] Implement the versioned DTO boundary where languages cross: schemas and
  conformance fixtures, exact decimal-string `u64`, validation, bounded atomic
  mutation batches, command acknowledgements, stale-revision behavior, bounded
  subscriptions, and diagnostics. Map to typed Rust commands/events internally.
- [ ] Isolate script evaluation from authoritative Rust scheduling. Prove the
  selected engine's interrupt/termination mechanism on each target; cooperative
  callbacks alone do not bound an infinite loop. Use a worker/process/isolate
  arrangement only after demonstrating its actual platform feasibility.
- [ ] Keep Rust simulation/load workers and stop/cancellation independent of UI
  progress. Bound ingress/egress; preserve a last-good UI or render an error state
  on timeout. A script must not hold backend locks while executing callbacks.
- [ ] Add a script-authored button plus editable text to the existing offline
  showcase, reusing phase 3 components. A callback requests a Rust action or local
  UI update; it does not implement simulation, load scheduling, or networking.

**Acceptance tests:** supported DTO versions round-trip; unknown versions/tags,
invalid properties, oversized batches/deep trees, stale handles/revisions, and
subscription floods are rejected without partial corruption. Values beyond JS's
safe integer range and `u64::MAX` round-trip exactly as strings; malformed and
out-of-range strings fail. Script exception/infinite-loop/restart fixtures are
bounded by the declared mechanism and cleanup releases resources. While the UI
script is stalled, a synthetic Rust backend continues scheduled work and obeys
its independent stop path; headless execution never imports/initializes the
interpreter. Test this separately on native and browser targets, not by assuming
WASM shares native threading behavior. No network target is involved.

**Stop/go:** if a candidate cannot be interrupted or isolated, do not integrate
it on an authoritative thread. Keep Rust-authored UI available and defer that
candidate. If browser main-thread execution cannot meet isolation requirements,
change the proposed host topology or narrow support; do not claim a worker exists
until it has been demonstrated. React integration is a separate optional gate,
not a prerequisite for script-authored UI.

### Phase 5 — Incremental updates, virtualization, and bounded animation

**Dependencies:** phase 3; script-driven coverage also depends on phase 4.
**Primary lanes:** retained semantics and GPU optimization, coordinated at dirty
paint boundaries.

- [ ] Refine dirty tracking independently for layout, text shaping, paint,
  resource binding, and upload. Apply backend snapshots/bindings in bounded
  batches; retain stable components and paint caches across presentations.
- [ ] Implement viewport-based list virtualization with stable row keys,
  bounded overscan and pools, scrolling, selection/focus continuity, and explicit
  handling of variable-height measurement. Start with fixed-height rows before
  deciding whether variable heights belong in the initial supported scope.
- [ ] Implement bounded visual animation with a monotonic presentation clock,
  property/uniform updates, cancel/remove behavior, and reduced-motion support.
  Animation must not change authoritative simulation time or traffic cadence.
- [ ] Benchmark static, localized-change, animated, and scrolling cases against
  phase 0 and the preceding phase. Record CPU/GPU p95/p99 and input latency;
  tune budgets from evidence rather than publishing numerical promises.

**Acceptance tests:** increasing logical row count while keeping viewport and
overscan fixed does not proportionally increase realized rows, paint commands,
or uploads. Out-of-view updates do not rebuild visible rows unnecessarily;
focus/selection survives scrolling/recycling without applying an edit to the
wrong row. Local property/text changes invalidate only their defined dependencies;
static frames show cache reuse without forbidding necessary scene redraws.
Animation cancel/removal/reduced motion work, and bounded queue/cache growth is
visible in sustained offline runs. Report comparative measurements and regressions
on every tested backend, including browser background/resume behavior.

**Stop/go:** no optimization may weaken ordering, editing correctness, or bounds.
Choose additional layout/paint caching only when the profile justifies it. A large
logical list is not accepted if every row still becomes a retained/rendered node.

### Phase 6 — Custom materials, shader surfaces, selective caching, and pixel APIs

**Dependencies:** phases 1/2 resource/paint contracts. Retained cached-panel
integration depends on phase 3; scripted uniforms depend on phase 4. This lane
need not wait for all virtualization work.
**Primary lane:** GPU execution.

- [ ] Implement first-class custom WGSL material/shader-surface descriptors:
  source/asset identity, entry points, typed uniforms, allowed textures, blend/
  alpha/clip contract, capability requirements, and compile diagnostics. Rust
  owns compilation, bindings, GPU submission, and handle lifetime.
- [ ] Cache compiled/pipeline work by source, entry points, binding schema,
  pipeline state, target format/sample count and capability/device context.
  Track cold/warm compilation, hits/misses, eviction, and failures. Do not promise
  portable persistent driver pipeline caches; choose persistence only if supported
  and measured. Bound compilation queue/cache size and repeated failure retries.
- [ ] Implement optional renderable content textures with explicit
  `RENDER_ATTACHMENT`/sampling usages; allocate storage/copy usages only for
  supported operations. Specify feedback hazards, resolve/format rules, clear
  behavior, and resize/device-loss rebuilding.
- [ ] Cache only panels/subtrees for which retained rendering is cheaper than
  repaint. Invalidate on content/style/font/scale/resource changes; animate a
  cached panel's opacity/transform without repaint only where clipping/semantics
  remain correct. Avoid one offscreen target per component.
- [ ] Distinguish rendering into a texture, sampling/composition of an existing
  texture, and actual texture/buffer copies. Share/sample first. Add real copy
  operations only for justified duplication, compatible transfers, or readback;
  account for their bytes, formats, synchronization and hazards separately.
- [ ] Expose small CPU pixel edits as validated dirty rectangles with bounded
  upload bytes, row layout, and size. Heavy manipulation uses GPU shaders or,
  where available, compute. Provide explicit unsupported responses or a separately
  bounded fallback; never silently emulate heavy compute with a UI-thread loop.
- [ ] Make readback explicitly asynchronous, with request IDs, bounded in-flight
  staging buffers/bytes, readiness/failure/cancellation, and lifetime cleanup.
  No synchronous per-frame pixel getter that stalls UI/GPU execution.
- [ ] Capability-gate effects/compute and bound target count/area/bytes, effect
  passes, dispatch/workgroup dimensions, upload/readback bytes, and shader source/
  compile work. Ship trusted bundled shader content first. Document that limits
  and validation cannot prove shader cost or safely sandbox hostile WGSL.

**Acceptance tests:** valid custom shader visualization composes and clips with
ordinary UI; syntax/binding/capability errors leave last-good content or an
explicit error placeholder. Warm shader/material reuse hits the cache; source,
format, binding, and device changes invalidate it appropriately. An unchanged
cached panel reuses its content texture; invalidation repaints it; animation can
compose that texture without an unnecessary copy. Dirty-pixel fixtures alter
only the requested region and account for bytes. Asynchronous readback completes
or cancels without blocking input and rejects excess requests. Oversized targets,
excess effects/uploads, stale resources, and feedback hazards fail safely.
Compute tests run only with observed support; WebGL2 proves the baseline
fragment/sampling path and honest rejection/alternative for unsupported compute.

**Stop/go:** compare offscreen caching to direct painting before enabling it by
default. Reject claims that all pixels are accessible cheaply or that shader
validation makes arbitrary user shaders safe. Defer expensive effects that lack
a measured benefit or acceptable backend fallback.

### Phase 7 — Extension-contract proof without spatial implementations

**Dependencies:** phases 3/6 input/content texture contracts.
**Primary lanes:** render contract, retained input, and test evidence.

- [ ] Expose renderer-owned content textures via stale-safe borrowed/leased
  handles. Define producer update/release and consumer sampling lifetimes;
  scripts/adapters do not take ownership of the device or raw GPU allocation.
- [ ] Define an adapter boundary for opaque presentation geometry/material
  references, independent surface pose/view metadata, and hit-to-UV results.
  The adapter owns geometry/presentation choices; retained UI owns content layout.
- [ ] Define input-provider events with surface/view and pointer identities,
  normalized UV/surface coordinates, phases/buttons and capture/cancel behavior.
  Support multiple pointer identities without conflating them with one mouse.
- [ ] Write a small **test-only fake adapter** that records presentation requests,
  returns injected synthetic UV hits, and supplies synthetic pointers/poses/views.
  It must not generate planes, cylinders, meshes, ray intersections, lens/stereo
  rendering, or use OpenXR/mobile SDKs. No out-of-box curved/VR showcase is added.

**Acceptance tests:** two fake consumers/views can refer to one content texture
without requiring copies. Changing fake pose/view metadata does not relayout UI
or regenerate content. Synthetic hits map to the same clipped retained targets
as screen input; out-of-range UV, stale surface/view, multiple-pointer capture,
provider disconnect and cancellation have defined outcomes. Resize/release/
device reset invalidates leases as specified. The fake sink verifies opaque
geometry/material handoff without constructing real geometry. Production screen
UI remains independent of the fake adapter.

**Stop/go:** passing these tests proves extensibility only, not curved UI or VR
support. Any real spatial adapter or SDK integration requires a separately scoped
future proposal and acceptance plan; it is not unfinished work in this milestone.

### Phase 8 — Integrated offline showcase and portability hardening

**Dependencies:** required phases 1–7 and their go decisions; any rejected optional
candidate/effect is documented rather than concealed.
**Primary lanes:** showcase/evidence, then targeted fixes in owning lanes.

- [ ] Assemble the staged Render Lab showcase: dynamic-camera/depth 3D, independent
  fixed-orthographic/layered 2D, last-pass overlay, script button/editable text,
  large virtualized list, custom shader visualization, cached panel and animations.
  Include controls to show bounded diagnostics and capability-specific omissions.
- [ ] Verify desktop, WASM compilation, actual browser WebGPU, and actual browser
  WebGL2 separately. Cover resize/scale, focus/IME, lifecycle/background resume,
  cache churn, script failure/restart, device/surface failure, and cleanup.
- [ ] Run bounded offline benchmark/stress fixtures; record the resolved budgets,
  provenance, hardware/backend, CPU/GPU distributions, input latency, shader/cache
  health, and memory/resource high-water marks. Audit fake headless/backend tests
  for no UI/interpreter scheduling dependency.
- [ ] Update implementation documentation and capability/decision records only
  in the later implementation task. Record accessibility and text-provider gaps,
  optional effects/runtime limitations, and regressions requiring follow-up.

**Acceptance evidence:** section 8's definition of done, completed test matrix and
reproducible comparison reports. No curved/VR demo, mobile SDK, cloud target, or
network-load result is required or implied.

**Stop/go:** if no script-authoring candidate satisfies the required gates, or
the selected solution or baseline browser paint path fails validation, report
the milestone as incomplete or explicitly revise its required scope. A Rust-only
fallback is useful progress, not completion of the script showcase.
Do not label compile-only, skipped GPU tests, or unavailable accessibility services
as validated product capabilities.

## 5. Selection gates and open risks

### Scripting candidates: compare host/runtime feasibility, not syntax alone

These are evaluation alternatives, not new dependencies selected by this plan.
No current interpreter compatibility was verified in this documentation task.

| Candidate | Why consider it | Evidence required before selection |
| --- | --- | --- |
| TS compiled to JS, minimal JSX factory over retained components | Preferred authoring ergonomics and tooling; host maps directly to Weaver components without a DOM. | Native JS-engine embedding and WASM/browser host feasibility; exact DTOs; offline transform/bundling; interruption/isolation; GC/memory/startup; useful source locations; bounded tree updates. Define the small supported JSX/component API rather than promising React compatibility. |
| JS/TS plus React custom reconciler | Familiar component/state authoring and reconciliation; still not React DOM. | All JS-host evidence plus actual `react-reconciler` host API/version fit, scheduling and commit semantics, host mutation batching, native embedding/polyfill requirements, bundle/maintenance costs, and update-storm behavior. Concurrent authoring does not make Rust mutation transactions automatically safe. |
| Lua over the same retained host | Potentially compact embedding and an explicit host API; useful comparator where JS isolation/portability is costly. | Concrete native/WASM build and invocation tests, numeric/ID fidelity, interruption and allocation limits, bindings/tooling/error locations, equivalent component/batch semantics, and authoring tradeoffs without pretending Lua is TS/JSX. |
| Restricted declarative/minimal authoring without a general runtime | Can preserve Rust-authoritative components if general scripting is not yet viable. | Honest limits on expressions/events/state; versioned model validation and debugging; evidence it covers the needed showcase. JSX transformation alone is not an interpreter or a safety boundary. This option narrows scope and does not silently satisfy general script execution. |

Choose the simplest candidate that passes the native/browser and boundedness
gates. The decision must distinguish authoring format, transform/type-checking,
runtime/interpreter, retained host, and optional framework. Do not use framework
familiarity as proof of cross-platform execution or safe script interruption.

### Remaining decisions and stop conditions

| Decision/risk | Required gate or mitigation |
| --- | --- |
| Script isolation and scheduling topology | Demonstrate timeout/termination and backend independence on each target. A browser main-thread stall can block Rust WASM too; callbacks and a frame budget alone are insufficient. Worker/process feasibility remains open. |
| Layout implementation | Compare a focused implementation with a compatible existing layout dependency against the limited layout/text/scroll contract. Avoid full CSS and avoid adding a dependency before a useful spike. |
| Glyphon batching, shaping, and font availability | Preserve retained buffers; test clipped/interleaved text, editing, stable IDs, deterministic test fonts, cache pressure, and WASM/backend behavior. Choose cache keys/eviction from actual behavior. |
| Platform text/accessibility providers | Decide native services and optional browser DOM assistance from IME/clipboard/semantic tests. Define availability/failure reporting; DOM must not become the shader-rich UI renderer. |
| Uniform/storage and texture formats | Derive limits from the device, including downlevel WebGL2 support. Use portable batching; negotiate storage/compute/renderable formats instead of inheriting desktop requirements. |
| Cache/surface policy | Measure repaint versus retained offscreen cost, memory and bandwidth. Bound allocations and distinguish sampling from real copies; do not cache everything. |
| Shader trust | Start with trusted assets; bound compile/resource/effect requests and restrict exposed bindings. Validation is not a cost bound or hostile-code sandbox; arbitrary untrusted shaders stay outside initial scope. |
| Script/DTO compatibility and ownership | Version schemas, enforce exact integers and stale handles/revisions, test subscription/resource cleanup. Do not serialize `Any` or share raw Rust references. |
| Modular theme and Host reuse | Demonstrate at least two token sets and independent semantic components. Reuse Host values only after identifying their source and fit; exclude Firebase/entitlements/hosting. |
| Existing work and crate boundaries | Sequence small changes and review diffs against current work. Keep implementation in existing areas unless evidence warrants extraction; no empty future crates. |
| Future curved/VR/mobile requirements | Preserve extension contracts, prove with fake adapters, and defer real adapters/SDKs. Do not turn extension-readiness into a shipped-support claim. |
| GPU/device loss and resource cleanup | Test failure/rebuild and stale-handle rejection; last-good/error UI must not hide leaks or unbounded retries. Unsupported timing instrumentation is reported, not fabricated. |

## 6. Measurement and resource policy

Set concrete test/run limits during phase 0 and record them with each result.
This plan sets no numerical frame-time, throughput, memory, or latency promise.
Acceptance combines correct bounded behavior with measured baseline comparisons;
performance budgets are stop/go decisions supported by those comparisons.

Each bounded offline run records:

- Source revision/local-change provenance, tool/build profile, browser/OS/driver,
  adapter/backend/device features and limits, CPU/GPU hardware, resolution/scale,
  presentation mode, fixture parameters/seed, warm-up/duration and sample limits.
- CPU time distributions for script evaluation, validation/bridge, retained update,
  layout/shaping, paint preparation, upload and render encoding; p95/p99 with
  sample counts and methodology. Distinguish wall-clock waiting from CPU work.
- GPU pass/frame timing where timestamp queries are supported and enabled. If
  unavailable, mark GPU timing unavailable; CPU submission or frame duration is
  not a substitute for GPU execution time. Account for instrumentation overhead.
- Input timestamps from provider receipt through dispatch/state change/paint/
  submission; p95/p99 input-to-submission latency. Measure actual input-to-visible
  latency separately when a presentation/external measurement method exists, and
  label estimates. Do not equate queue submission with displayed pixels.
- Realized/layout/paint node counts, draw/batch counts, allocated/reused bind groups,
  upload bytes/dirty ranges, offscreen passes/area/bytes, **actual copy bytes**,
  readback latency/in-flight bytes, and resource high-water/cleanup behavior.
- Glyph/layout/atlas and shader/material cache hits, misses, evictions, capacity,
  cold/warm compile latency/failures, and shader-request admission/retry behavior.
- Backend work progression, cancellations, dropped/coalesced UI telemetry where
  defined, validation rejections, stale requests and script errors/timeouts.

Test static and localized changes separately from full-screen animation and list
scrolling. Warm/cold text and shader cases need separate results. Compare the same
fixture/limits/hardware; retain raw bounded samples or histograms for review.
Visual screenshot readback is test instrumentation, not a required runtime path.

Bound resources at each admission boundary: node/tree depth, text length,
subscriptions/events, mutation batches/bytes, script work/heap, command queues,
GPU target count/dimensions/bytes, glyph/shader caches, effect passes, compute
dispatch sizes, uploads, copies, readbacks, and retained results. Document
rejection, coalescing, eviction and cancellation policies. Queue rejection is
observable data, not silent success. Avoid unbounded retries after invalid input,
shader failures, or device loss. Resource validation does not prove CPU/GPU cost;
trusted content plus measured limits remain necessary.

## 7. Validation matrix and existing commands

**These are future implementation checks, not commands executed for this plan.**
Run from the Weaver root. The commands below exist in `AGENTS.md`, `README.md`, or
the inspected `xtask` parser; there is no invented UI lab or backend flag.

Focused checks begin with changed crates and the existing lab, for example:

```sh
cargo test -p weaver-render -p weaver-render-wgpu -p weaver-app-core
cargo xtask check render --platform all
cargo xtask build render --platform desktop
cargo xtask build render --platform web
```

Then validate the shared shells against the other existing labs:

```sh
cargo xtask check first-person --platform all
cargo xtask check space --platform all
```

Broader checks prescribed by Weaver guidance, after focused checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
cargo build --workspace --all-targets
```

For a future manual desktop smoke test, `cargo xtask run render --platform desktop`
is supported. `xtask` does not implement web running: web build packages static
output in `dist/render/`, checks that the installed `wasm-bindgen` CLI matches
`Cargo.lock`, and requires `wasm32-unknown-unknown`. A maintainer may serve that
local output separately as described in README. Do not claim `check`/`build`
executes browser GPU tests, and do not use `--platform all` for `build` or `run`.

Keep UI validation offline: use installed/cached tools/dependencies and
`CARGO_NET_OFFLINE=true` when enforcing offline Cargo execution. If prerequisites
are missing, record a blocked check rather than installing/fetching implicitly.
Audit broader workspace tests before treating them as an offline UI suite; do
not invoke Woven/soak/shared/cloud traffic through environment switches. No cloud
mutation, billable load, shared target, credentials operation or network test is
part of this plan's acceptance. Any later real network validation is a separate
approved task under the ecosystem/Woven rules.

| Target/test layer | Required evidence |
| --- | --- |
| Pure Rust / fake host | Order/clip/layout/invalidation/virtualization/DTO/limits/lifetime tests; fake input, fake presentation adapter, synthetic backend scheduling and cancellation. No GPU or network needed for these assertions. |
| Desktop compilation and runtime | Existing desktop build/check; actual native GPU visual/pass tests, focus/IME/scale/lifecycle, shaders/caches/readback and script isolation. Record adapter/capabilities and bounded measurements. |
| WASM compilation/packaging | Existing web checks/build, matching tools, shared-app code compiled without the native QUIC/server graph. This proves packaging, not browser operation. |
| Browser WebGPU runtime | Actual reported WebGPU backend, baseline composition and interaction, capability-gated optional shader/effect/compute tests, browser script isolation and performance/cleanup evidence. |
| Browser WebGL2 runtime | Force WebGPU unavailable through a documented test environment or a narrowly scoped future test-only backend selector; confirm the actual reported fallback backend. Test all baseline draw/order/text/input/batching/resource behavior and rejection of unsupported compute/effects. No new production selector is assumed to exist today. |
| Failure and boundedness | Invalid/stale/oversized input, multiple batches, cache churn, queue floods, shader compile failure, script infinite loop, resize/device failure, asynchronous readback cancellation, provider disconnect, and headless/backend progression while UI stalls. |

Use golden fixtures with deterministic fonts/assets and explicit pixel/color
tolerances where backend rasterization differs. Pair screenshots with structural
pass/order/counter assertions so visual similarity cannot hide truncation,
wrong-depth rendering, or unnecessary copies. Mark unsupported optional features
and unavailable hardware accurately; required baseline runtime coverage remains
an open gate until tested on that backend.

## 8. Definition of done and handoff

The initial milestone is done only when all required items below have evidence;
none is completed by this planning document.

- [ ] Accepted rendering domains share vocabulary/resources/composition while
  retaining independent semantic models; depth, cameras, scene order, UI
  stacking/clipping, alpha/scale and ownership rules are documented and tested.
- [ ] Existing labs/shells remain functional; admitted UI is not silently capped
  at 250, mixed paint ordering and real texture handles are correct, and retained
  glyph/binding/resource caches have bounded lifetimes and measured reuse.
- [ ] Retained components support the agreed layout/input/editing/theme scope,
  with keyboard/IME behavior and honest accessibility-provider limitations.
- [ ] A selected, evidence-backed script host authors button/text controls through
  versioned validated batches, exact `u64` strings, bounded subscriptions and
  actionable errors; native/browser interruption and backend isolation are proven.
- [ ] The large logical list is truly virtualized; incremental updates and
  animations are bounded and do not own simulation or network scheduling.
- [ ] Custom WGSL content, selective cached panels, dirty CPU pixel uploads and
  asynchronous readback obey resource/trust/capability policy. Sampling/composition
  is distinguished from actual copies in both API semantics and measurements.
- [ ] Test-only fake adapters prove content ownership, opaque presentation
  geometry/material handoff, hit-to-UV mapping, multiple pointers, and independent
  poses/views. No real spatial geometry, curved/VR demos, or SDK implementation is
  included or implied.
- [ ] Desktop, WASM packaging, browser WebGPU and baseline WebGL2 evidence is
  recorded separately; unsupported optional capabilities and blocked tests are
  clearly labeled. No numerical performance guarantee is inferred from one run.
- [ ] The complete staged offline showcase and reproducible CPU/GPU p95/p99,
  input-latency, cache/compile and resource reports support the final stop/go review.
- [ ] Headless authoritative/backend execution stays independent of the script/UI
  host, Woven's protocol/network boundary is unchanged, and Host/cloud/mobile/VR
  responsibilities remain outside this implementation's initial scope.

Handoff after each future phase: reviewed changes in its owning lanes, focused
acceptance results, provenance and resolved limits, measured regressions, open
choices, and a written go/defer/stop decision for the next dependency. Preserve
useful Rust-authored UI when a runtime gate fails; do not misrepresent a reduced
scope as the full initial deliverable. Any real spatial adapter, general untrusted
plugin model, mobile SDK, or network/load-test expansion needs its own subsequent
implementation plan and approval boundaries.
