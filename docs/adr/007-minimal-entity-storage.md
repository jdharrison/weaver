# ADR 7: Minimal entity storage before choosing an ECS

## Status

Accepted

## Context

A full ECS selection is premature, but the runtime needs entity identity and component storage for the vertical slice.

## Decision

`weaver-core` provides a minimal `EntityRegistry<T>` backed by a `Vec`. It exposes only spawn, remove, get, and iteration. This will be replaced by a real ECS once requirements from rendering, simulation, and Signalweave are better understood.

## Consequences

- The bootstrap milestone is not blocked by ECS evaluation.
- Systems are written against small, replaceable interfaces.
- Migration to an ECS will require updating callers of `EntityRegistry`.
