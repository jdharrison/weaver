# ADR 8: Deferred scripting, editor, physics, audio, and managed hosting

## Status

Accepted

## Context

Many features are desirable but out of scope for the bootstrap milestone.

## Decision

The following are explicitly deferred and are not scaffolded as empty architectural theater:

- General scripting / WASM runtime
- Editor
- Physics engine
- Audio engine
- Production asset pipeline
- Hot reload
- General relativity
- Production authentication UI
- Cloud provisioning / `signalweave.host` deployment
- Automatic distributed simulation

The code seams (command sink, snapshot interface, connectivity mode enum, frame domain enum) are designed to accommodate these later without rework.

## Consequences

- The bootstrap codebase stays focused and verifiable.
- Deferred features are documented so they are not accidentally designed out.
- Future milestones will add each feature with real behavior and tests.
