# ADR 1: Weaver as a separate consumer of Woven and Worldline

## Status

Accepted

## Context

Weaver needs distributed session/state/event capabilities and time-aware simulation, both of which exist in sibling repositories.

## Decision

Weaver consumes Woven and Worldline without exposing their internal types. It uses the sibling `../woven` checkout for the Woven development server, client, and protocol crates. All interaction passes through narrow adapter crates (`weaver-woven`, `weaver-worldline`) that translate to Weaver-owned vocabulary.

`weaver-woven` starts Woven's development composition on loopback and connects through the public `WVN1` protocol. Weaver does not link to or invoke `woven-core` directly.

## Consequences

- Woven and Worldline remain independent.
- Weaver can evolve its own entity, command, and snapshot models without forcing changes upstream.
- Adapter crates must be kept thin; no internal types leak past them.
- Local development exercises the same protocol/transport boundary as remote Woven targets.
