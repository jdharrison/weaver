# ADR 6: Fixed simulation updates with independent rendering

## Status

Accepted

## Context

Simulation and rendering must run at different rates, and the simulation must remain deterministic.

## Decision

The application loop advances the Worldline-backed simulation clock at a fixed tick rate. Each tick commits a new world revision. The renderer extracts a `SceneSnapshot` at the current revision and presents independently, interpolating transforms from bounded history when needed.

## Consequences

- Frame drops do not affect simulation correctness.
- Render state is always tagged with revision and simulation time.
- The renderer can run faster or slower than the simulation.
