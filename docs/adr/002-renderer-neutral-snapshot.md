# ADR 2: Renderer-neutral snapshot extraction

## Status

Accepted

## Context

Simulation systems must not issue GPU commands, and the renderer must be swappable. The simulation layer needs a clean way to describe visible state.

## Decision

The simulation extracts an immutable `SceneSnapshot` containing camera, meshes, sprites, particles, text, and UI. The `weaver-render` crate defines this vocabulary and depends on no graphics API. `weaver-render-wgpu` consumes the snapshot and issues GPU commands.

## Consequences

- Simulation and rendering are decoupled.
- Headless execution is possible by simply not rendering.
- Future backends (Vulkan, Metal, software) can reuse `weaver-render`.
