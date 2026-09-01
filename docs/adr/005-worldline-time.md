# ADR 5: Worldline-backed time and frame state

## Status

Accepted

## Context

Simulation time must be deterministic, frame-addressed, and capable of running without a renderer.

## Decision

`weaver-worldline` wraps Worldline's `SimEngine` and exposes explicit start/pause/resume/step/reset controls, absolute classical simulation time, a time multiplier, fixed-step advancement, and bounded trajectory history. Frame relationships form a tree with cycle rejection.

## Consequences

- Headless tests can step deterministically.
- Presentation-time interpolation can smooth between simulation ticks.
- Relativity is explicitly deferred; the seams (classical/Minkowski/curved domains) are preserved in naming and module structure.
