# First-Person Lab browser release seam

This shell is desktop-first for chat: a physical keyboard opens/submits chat with
Enter and cancels with Escape. Existing touch movement/look remain available;
mobile virtual-keyboard chat is not implemented. Room/presence/chat remain the
MVP; portfolio content/layout is separate work.

## Scene lifecycle contract

`main.ts` defines these globals **before** importing/initializing WASM:

- `globalThis.weaverSceneReady()` — Rust calls this after `install_renderer`
  succeeds, including required asset uploads. WASM startup completion is not
  renderer readiness. Both completion and this callback are required to join.
- `globalThis.weaverSceneFatal(message)` — Rust calls this on fatal graphics,
  initialization, window, or render failures. It disconnects/cancels the network,
  disables joining, and exposes a page reload action. A later ready callback
  cannot revive a fatal scene.

The browser also fails startup after 20 seconds without full readiness. The Rust
web platform now supplies these hooks; First-Person's panic hook reports fatal
failures immediately. Regenerate the WASM build after changing either side.

`visibilitychange` to hidden and `pagehide` fully disconnect, abort bootstrap or
admission, invalidate pending receives/handshakes, clear pending publishes, and
cancel the delayed connection log. Visible/BFCache restoration permits **manual**
rejoin, never automatic rejoin. The client API cannot directly abort its initial
WebTransport handshake; its existing 10-second bound remains, and a client
returned after cancellation is immediately closed. A replacement attempt cannot
overlap that pending attempt.

## Opt-in anonymous bootstrap v1

The checked-in HTML has two **empty non-secret** metas. No anonymous request or
public autoconnect happens until configured:

```html
<meta name="weaver-lobby-bootstrap" content="/api/first-person-lobby">
<meta name="weaver-lobby-target" content='{"version":1,"url":"https://realtime.example:4434/webtransport","namespaceId":"2","sessionId":"2","spaceId":"3","spaceEpoch":"1"}'>
```

- Bootstrap meta: empty or literal `off` disables this mode. Otherwise it must be
  a canonical same-origin absolute path, not a full URL, relative path,
  protocol-relative URL, or a path with query/fragment/backslash/whitespace.
- Target meta: exact JSON fields `version`, `url`, `namespaceId`, `sessionId`,
  `spaceId`, `spaceEpoch`. It pins the **exact HTTPS WebTransport endpoint**
  (including origin/port/path) and scope, not merely an arbitrary response URL.
  Userinfo, query, fragment, and root-only endpoint URLs are rejected. URLs are
  canonicalized with `URL`; equivalent default-port spellings compare equally.
- Version must be numeric `1`. IDs are canonical positive decimal **strings**,
  with no leading zeros, in `1..18446744073709551615`; `spaceId` must be at least
  `3`. JSON numbers are not accepted for IDs.

The route must respond with `Content-Type: application/json` and exactly the
following fields (the token placeholder below is descriptive, not a valid token):

```json
{
  "version": 1,
  "url": "https://realtime.example:4434/webtransport",
  "namespaceId": "2",
  "sessionId": "2",
  "spaceId": "3",
  "spaceEpoch": "1",
  "token": "<exactly 64 lowercase hexadecimal characters>"
}
```

The response endpoint and all scope IDs must match the target meta. Extra fields,
including invented TTL/identity fields, are rejected. This contract makes **no
TTL, uniqueness, per-visitor credential, or authenticated-identity claim**.
Issuance/security/rate limiting are the bootstrap service owner's responsibility;
this is a client contract, not a new credential model or data-plane proxy.

Each initial join or manual retry makes at most one request with `cache: no-store`,
`credentials: omit`, `redirect: error`, and `referrerPolicy: no-referrer`. The total
request/body deadline is 10 seconds and the streamed body ceiling is 8,192 bytes,
regardless of Content-Length. There are no retry loops. The service must also
prevent intermediary caching; client no-store is not a server cache policy.

Bootstrap credentials stay in memory, never fill the operator credential field,
never enter localStorage, and are not logged. Diagnostic text is bounded and
redacted. Retry fetches anew instead of retaining an anonymous token for reuse.

## Explicit modes and payload handling

Advanced operator/development settings remain explicit. Their configured endpoint
is operator-selected rather than constrained by the anonymous target meta; the
existing bounded operator credential and optional development certificate hash
paths remain intact. The existing localhost:8000 / 127.0.0.1:8000 local-token path
still autoconnects, **only after renderer readiness**, with a bounded, no-store
read. No local-token fetch runs on public origins. Public artifacts must exclude
`woven.local-token`; release configuration remains parent-owned.

Reliable channel-1/type-1 application payloads exceeding 2,048 bytes or containing
invalid UTF-8 are individually dropped before entering WASM. Rust remains
responsible for JSON/name/chat semantics. Actual transport/protocol failures and
bounded-inbox overflow remain connection failures. Poses still use positioned
25-byte channel-4 datagrams; no delivery fallback or channel-policy override is
introduced. Local chat echo is not a peer delivery acknowledgement.

## Local validation and remaining gates

```sh
npm run typecheck
npm test
```

Tests use Node, synthetic data, mocked browser/Woven/WASM seams, and in-memory
esbuild bundles. They do not read generated WASM or contact any endpoint.

Before public multiplayer release, supply an approved bootstrap service/target
configuration and verify real networking/browser behavior. See the
[portfolio release runbook](../../../../docs/PORTFOLIO-RELEASE.md) for static
packaging, Firebase Hosting checks, public-token risks, and activation gates.
Manual browser QA is still required for two-peer late join/chat/rename/leave,
room-corner spatial interest, pointer-lock/focus/IME, small-screen/DPI overlays,
and hidden-tab/BFCache/sleep restoration. No hosted validation is implied.
