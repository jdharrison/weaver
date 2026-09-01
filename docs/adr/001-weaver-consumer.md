# ADR 1: Weaver as a separate consumer of Signalweave and Worldline

## Status

Accepted

## Context

Weaver needs distributed session/state/event capabilities and time-aware simulation, both of which exist in sibling repositories.

## Decision

Weaver consumes Signalweave and Worldline as pinned Git dependencies. It does not modify them and does not expose their internal types. All interaction passes through narrow adapter crates (`weaver-signalweave`, `weaver-worldline`) that translate to Weaver-owned vocabulary.

## Consequences

- Signalweave and Worldline remain independent.
- Weaver can evolve its own entity, command, and snapshot models without forcing changes upstream.
- Adapter crates must be kept thin; no internal types leak past them.
