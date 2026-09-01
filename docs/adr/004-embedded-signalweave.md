# ADR 4: Embedded Signalweave for offline mode

## Status

Accepted

## Context

Weaver must run completely offline without requiring network sockets or external services. At the same time, future connectivity modes (local host, remote, hybrid) must be pluggable.

## Decision

`weaver-signalweave` implements an `OfflineEmbedded` mode that creates an in-process `SignalweaveCore` worker, provisions a namespace/session/space/connection/entity, and uses development authentication. Other modes return explicit unsupported errors in this milestone.

## Consequences

- Offline use requires no network configuration.
- The adapter preserves Signalweave semantics: delivery class, sequence, revision, space epoch.
- Later work will implement local-host and remote managed modes behind the same enum.
