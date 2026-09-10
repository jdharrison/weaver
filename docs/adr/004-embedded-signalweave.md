# ADR 4: Embedded local Woven node for development

## Status

Accepted

## Context

Weaver needs a self-contained local development path while preserving the Woven protocol boundary used by deployed targets. The previous in-process `SignalweaveCore` worker did not exercise that boundary and was tied to the superseded API.

## Decision

`weaver-woven` implements `EmbeddedLocalNode`. It starts Woven's development composition on ephemeral loopback ports and connects through the official native `WVN1` QUIC client. Woven provisions the development namespace/session/space/channel definitions and assigns the client entity during subscription.

At adoption, `Remote` remained unsupported pending an operator-supplied endpoint,
credentials and bounded lifecycle controls. The subsequent `RemoteQuic` path now
uses explicit CA trust, a file-supplied static token and the public verified QUIC
client. The lab requires a 1–300 second deadline and bounded launch counts/rates;
local modes retain their original development behavior. This first remote path
targets the sibling server's fixed namespace/session/space/channel composition,
not capability discovery or general production tenant provisioning. See the
README's secure remote section for exact settings, safeguards and limitations.

## Consequences

- Local development requires loopback sockets but no externally deployed service.
- The adapter uses the real Woven handshake, authentication, subscription, server-assigned entity, delivery, and persistence semantics.
- Weaver does not access `woven-core` directly.
- Later remote support can use the same protocol adapter rather than a separate embedded implementation.
