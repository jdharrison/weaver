# ADR 9: Unified accelerated rendering and modular scriptable UI

## Status

Accepted — architectural direction. Implementation is deferred and tracked in
[the separate implementation plan](../UI-IMPLEMENTATION-PLAN.md).

Date: 2026-09-30.

Acceptance of this ADR does not mean that a component library, scripting runtime,
custom-material system, spatial UI integration, or performance target has shipped.

## Context

Weaver separates application/simulation state, renderer-neutral scene
vocabulary, WGPU execution, and desktop/browser platform shells. This is a sound
foundation, but the existing UI drawing primitives are not a complete application
UI toolkit. Extending them ad hoc would require repeatedly solving layout, text
editing, focus, accessibility, and component behavior inside application Rust.

Authors need modular design systems and a scripting/composition layer without
moving authoritative simulation, networking, or GPU execution into scripts.
Material UI is a useful model for component ergonomics and design recipes, not a
portable native rendering implementation or a selected dependency.

UI and ordinary 2D content share graphics needs: shapes, images, glyphs,
transforms, materials, clipping, and composition. Their higher-level semantics
are different. A game sprite must not become a widget to use the renderer.

The graphics layer must also support custom shaders, pixel operations, render
targets, and reusable surfaces without defaulting to CPU repaint/readback.
GPU acceleration alone is not a performance guarantee; invalidation, layout,
shaping, uploads, batching, overdraw, and synchronization remain significant.

Future diegetic/curved displays and VR should be possible through extension
contracts without rewriting the component system. The requirement is
**supported through extensibility, not included off the shelf**. Implementing
those integrations now would exceed the agreed scope.

## Decision

### 1. Use one shared graphics foundation

Extend renderer-neutral graphics/view/composition descriptions and the WGPU
backend for scene and UI consumers. Share GPU resources, materials, drawing,
text, targets, and composition where appropriate.

Share the low-level rendering contract, not a mandatory universal entity,
component, or scene-tree model. UI can retain component identity and layout;
2D/3D producers can retain their own state and submit efficient draw updates.

Keep `SceneSnapshot` a scene/render description, not the scripting heap,
application observation model, or widget database. Preserve the simulation versus
GPU execution boundary established by ADR 2.

### 2. Limit initial built-in presentation to three modes

| Mode | Camera/coordinates | Ordering |
|---|---|---|
| 3D scene | World space; dynamic camera, perspective available | Scene depth testing and appropriate transparency |
| 2D scene | Flat X/Y; fixed orthographic camera | Explicit layer/draw ordering; no scene depth by default |
| UI overlay | Screen-space logical coordinates; orthographic | Final overlay above scene content, with its own stacking/clipping and no scene depth |

Viewport/resize projection changes do not make the fixed 2D mode a dynamic
perspective/orbit camera. Scene-view placement/order is explicit; 2D content does
not inherently cover every 3D view.

Render scene content and scene-only effects before the UI overlay. A 3D preview
inside UI may use its own depth-tested target and then be composed into that
overlay. "Above all" is within the application frame, not OS-level ordering or a
license to ignore internal UI ordering.

### 3. Build modular components and design systems above graphics

Separate foundation behavior (layout, text, activation, scrolling, focus,
selection, accessibility) from visual tokens/recipes and application components.
Material-inspired, console, and game-specific design systems can consume the
same foundation without becoming core graphics policy.

Do not require every screen to be authored in Rust. A scripting layer owns
composition, local UI state, bindings, and event handlers. TypeScript/JSX is a
preferred candidate; the language, interpreter, component framework/reconciler,
and isolation topology are not selected by this ADR. JSX does not require React,
and existing React DOM/MUI components do not become portable automatically.

Rust/application services retain authoritative state, scheduling, cancellation,
limits, and GPU ownership. Use bounded, validated mutation batches and explicit
application capabilities. Cross-language/process contracts must be versioned,
runtime validated, and preserve 64-bit identifiers exactly. UI observation must
not block a runner or simulation; pausing simulation must not freeze UI controls.

### 4. Make efficient drawing and custom GPU content first-class

Use ordering-correct batching, incremental invalidation, text/glyph reuse,
virtualization, persistent resources, and selective surface caching. Texture
copy and sampled composition are distinct operations; caching every widget is
not the default.

Provide an implementation path for custom WGSL materials/shader surfaces,
bounded region uploads, GPU-resident visualization, and explicit asynchronous
readback. Effects/compute require declared capabilities, resource dependencies,
and bounded intermediate work. The WebGL2 fallback must not claim WebGPU compute
semantics. Unsupported features are explicit, with tested alternatives where
provided.

No synchronous CPU readback is required for ordinary same-device UI/scene
composition. Avoid per-draw script callbacks and full-tree/full-surface updates
as default behavior. Compile/cache material resources outside routine draw
submission where possible. Validate performance through representative offline
measurements rather than advertising an unmeasured FPS or latency target.

Shader validation is not a cost proof; embedded scripting is not automatically a
secure sandbox. Begin with explicit trust/capability policies and bounded
resources, not promises about arbitrary third-party code.

### 5. Support future presentation extensions without implementing them

Separate logical UI content and GPU target ownership from its presentation and
interaction mapping. The initial presentation consumer is the screen-space
overlay. An extension may supply geometry/material/view placement and a mapping
from an external hit to UI-local coordinates.

Do not assume every UI root fills an OS window, every surface is flat, only one
pointer/view exists, or a pose change requires content repaint. Keep input
identity, resource lifetime, coordinate conversion, and content versus
presentation updates explicit.

This does **not** require built-in plane/cylinder components, curved geometry,
spatial ray mapping, diegetic displays, multi-view rendering, VR/OpenXR,
controllers, headsets, or native mobile integration. Validate seams with a
small test-only adapter, not a shipped spatial library. Extensible architecture
is not evidence of future platform/runtime compatibility.

### 6. Preserve platform and product boundaries

Native WGPU and browser canvas/WGPU are the primary graphics path, with explicit
WebGPU/WebGL2 capability handling. Platform adapters provide input, text editing,
IME, clipboard, and accessibility; GPU pixels do not implement those services.
DOM can support browser semantics/text integration without being the rendering
source of truth for the GPU component system.

No desktop web shell is mandated. Host tokens/patterns may be shared deliberately;
Host authentication/entitlements are not dependencies of local Weaver UI.
Networking still uses Woven's public protocol/client path. Do not add UI or
domain behavior to Woven or use Host as a data-plane proxy.

## Alternatives considered

- **Expand custom Rust rectangles/text into an all-Rust application toolkit:**
  retains direct integration but concentrates authoring and substantial widget
  infrastructure in Rust. Keep native diagnostics useful; do not make this the
  required application-authoring model.
- **DOM UI everywhere through a browser/webview:** mature component and text
  facilities, but does not establish a shared native GPU drawing/material layer;
  native viewport composition is not automatically solved. Remains an optional
  consumer/delivery choice.
- **React/MUI unchanged across targets:** browser/DOM dependencies prevent this
  from supplying native WGPU components by itself. Reuse design concepts or
  genuinely portable assets, not assumed implementation compatibility.
- **Declarative native toolkit such as Qt/QML or Slint:** credible UI authoring
  option, but introduces different binding/packaging and rendering integration
  decisions. Not selected as the shared graphics foundation.
- **Separate UI and 2D graphics engines:** duplicates resources and compositing
  facilities and makes mixed content harder. Prefer shared graphics contracts
  with specialized higher-level producers.
- **Ship curved surfaces and VR now:** exceeds scope and creates runtime/input
  obligations unrelated to the initial three modes. Preserve extension contracts
  instead of bundling those features.
- **Design a universal renderer/plugin graph in advance:** risks empty
  scaffolding. Introduce only dependencies/contracts exercised by actual
  consumers and tests.

## Consequences

### Positive

- High-level UI authoring does not require moving execution out of Rust.
- UI, 2D drawing, custom GPU visuals, and 3D previews can share resources and
  composition without a universal high-level object model.
- Rendering scope and overlay ordering are explicit and testable.
- Design systems remain replaceable; networking and hosted concerns remain
  separate.
- Future presentation/input extensions have a defined boundary instead of
  requiring component rewrites.

### Costs and constraints

- Retained components, layout, text editing, accessibility, script hosting,
  resource lifetimes, and recovery are substantial implementation work.
- A custom GPU renderer does not inherit browser widget behavior or accessibility.
- Capability differences, alpha/color conventions, transparent ordering, cache
  invalidation, and resource limits require explicit contracts and tests.
- Performance and portability must be demonstrated on identified targets;
  architectural acceptance is not certification or a speed guarantee.
- No empty crates, speculative spatial implementations, or interpreter/framework
  dependencies are authorized solely by documenting this decision.

## Relationship to earlier decisions

- **ADR 2:** retain renderer-neutral extraction and simulation/GPU separation;
  specialized UI contracts complement, not replace, scene snapshots.
- **ADR 3:** retain WGPU and Glyphon/Cosmic Text as current foundations. Introduce
  effect dependency planning only when real consumers require it, not a
  speculative universal render graph.
- **ADR 6:** preserve independent execution timing; UI scripts and rendering do
  not become authoritative simulation/load clocks.
- **ADR 8:** UI-scoped scripting now has an accepted architectural direction and a
  deferred implementation plan. General scripting/WASM execution, editor,
  physics, audio, hosting, and other unrelated bootstrap deferrals remain intact.

## Supporting documents

- [Architecture and detailed contracts](../UI-ARCHITECTURE.md)
- [Deferred implementation plan and acceptance gates](../UI-IMPLEMENTATION-PLAN.md)
