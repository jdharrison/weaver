# ADR 4: Embedded local Woven node for development

## Status

Accepted

## Context

Weaver needs a self-contained local development path while preserving the Woven protocol boundary used by deployed targets. The previous in-process `SignalweaveCore` worker did not exercise that boundary and was tied to the superseded API.

## Decision

`weaver-woven` implements `EmbeddedLocalNode`. It starts Woven's development composition on ephemeral loopback ports and connects through the official native `WVN1` QUIC client. Woven provisions the development namespace/session/space/channel definitions and assigns the client entity during subscription.

`Remote` remains explicitly unsupported until it has an operator-supplied endpoint, credentials, capability discovery, and bounded lifecycle controls.

## Consequences

- Local development requires loopback sockets but no externally deployed service.
- The adapter uses the real Woven handshake, authentication, subscription, server-assigned entity, delivery, and persistence semantics.
- Weaver does not access `woven-core` directly.
- Later remote support can use the same protocol adapter rather than a separate embedded implementation.
