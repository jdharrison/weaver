# ADR 3: WGPU as the initial graphics backend

## Status

Accepted

## Context

The bootstrap milestone needs a cross-platform GPU backend with reasonable stability and Rust ecosystem support.

## Decision

Use WGPU 25 as the first graphics backend. Pipelines are implemented directly with instanced draws for meshes, sprites, and particles. Text is rendered via glyphon/cosmic-text.

## Consequences

- Works on Vulkan, Metal, DX12, and WebGPU-capable targets.
- Keeps the backend simple and observable; no speculative render graph yet.
- Shader code is WGSL, loaded from source strings.
