# jdharrison.com — shared-room MVP release

## Scope and architecture

The intended first public release is the First-Person Lab room: visitors join
anonymously, see other real visitors, change their display name, move, and chat. Portfolio content,
career history, and room redesign come next. Chat is desktop-first; touch movement
is not evidence of working mobile virtual-keyboard chat.

Firebase Hosting serves the web shell and Weaver WASM. Woven is a separate
WebTransport service, not embedded in WASM or proxied through Hosting/Host. A small
application-owned HTTPS bootstrap can return the room connection descriptor;
it is not the Woven data plane and requires no visitor account.

**Current state:** static packaging, lifecycle hardening, an opt-in bootstrap
client, and local Hosting checks are implemented. No production Firebase project
or site is selected. No bootstrap backend or Hosting API rewrite is implemented;
visitor metas are deliberately empty. A static deploy alone would render an
**offline/operator-configured room**, not the anonymous multiplayer MVP.

## Release identity and compatibility

The Weaver workspace release is **0.2.0**. A pre-1.0 minor increment reflects
breaking frame/input and realtime command/event API changes, plus First-Person's
new positioned-datagram/spatial-space requirement. Crates continue to inherit
`workspace.package.version`; do not assign independent per-crate versions.

The exact compatible Woven source is
**`2bc74d3298b2f317cc00a1508b09b54250c612d8`**: **runtime 0.4.0**, **protocol,
Rust client and npm client 0.3.0**, and **wire version 1 (`WVN1`)**. Both workspaces
now declare **minimum Rust 1.98**; local checks and CI use **Rust 1.98.0**.
Rust 1.88 is not a supported release-candidate toolchain. Cargo and shell npm
lockfiles are refreshed for the coordinated versions.

**Weaver source revision:** the commit containing the **0.2.0 workspace manifest
and this release handoff**. Record that created commit's ID in release/CI metadata;
do not substitute an earlier HEAD or embed a self-referential hash in its contents.

The configured `.github/workflows/ci.yml` pins the exact Woven SHA above, places a
real sibling checkout (not a symlink), disables credential persistence in all eight
checkout steps, and uses locked Rust 1.98.0 validation/builds. Its web job checks,
builds, validates and uploads `dist/first-person` including the legal notices.
**No GitHub Actions run is claimed here.** Local candidate evidence is recorded
separately below; the earlier baseline and packaging-only pass remain history.

## Pre-version-bump local baseline (2026-10-03)

These checks passed before the coordinated version bumps; retain them as baseline
evidence, not as validation of the final Weaver/Woven release pair.

- Formatting and strict workspace Clippy passed.
- All 167 Rust tests and all 45 browser-shell tests passed.
- Native workspace build, First-Person desktop/WASM checks, and release web build passed.
- Final 10-file artifact inventory and Firebase Hosting asset/header/404 smoke passed.
- Real isolated headless Firefox 155.0.1 reached `Weaver ready on Gl` and rendered
  the room without page errors. This caught and verified the fix for a WebGL2 UI
  uniform alignment defect. WebGPU, interactive input, two-peer networking, live
  TLS/origin/domain configuration, and mobile chat remain unverified.
- No production/cloud/DNS/secret changes, deployments, or hosted Woven traffic.

## Security and provisioning gates

Before enabling anonymous joining:

- Explicitly accept that the existing managed Bearer is a **shared, extractable,
  non-expiring session credential**. An HTTPS bootstrap keeps it out of source
  and static artifacts; it cannot make a browser-delivered token secret. The
  current protocol does not issue per-visitor tokens or support TTL/PATCH token
  rotation. Revocation requires deleting/recreating the whole managed session.
- Use a dedicated public session with **no private content or future private
  spaces**. The credential grants that session's configured spaces/channels,
  including later spatial additions. Server-assigned entities are nevertheless
  connection-owned; display names/local UUIDs are not authenticated identities.
- Record and approve the endpoint, namespace/session/spatial-space/epoch, CCU
  cap, aggregate publish ceiling, and hosting budget before activation. Do not
  reuse demo IDs or infer capacity from example form defaults.
- The space must be preconfigured Cartesian3D/SpatialGrid3D, ID >=3, epoch 1,
  with bounds covering the 12-by-16-unit room and eye height. Configure spatial
  interest to cover the **whole room**, including opposite corners. Verify
  this with two peers rather than assuming a grid radius is sufficient.
- Channel 1 is ReliableOrdered/Ephemeral chat/profile; channel 4 is
  UnreliableSequenced/Ephemeral, positioned 25-byte poses over real datagrams.
  The node must negotiate positioned state and unreliable support. No client
  policy override, unpositioned broadcast, or reliable pose fallback exists.
- Poses publish at 10 Hz, **plus** profiles/chat. Managed `tickRateHz` counts
  aggregate publish attempts across lanes. A 10 Hz ceiling leaves no chat
  headroom. For Host provisioning, verify a product/rate/CCU combination that
  supports the room rather than selecting its 10 Hz `web` product by name.
- Serve WebTransport using browser-trusted TLS for the endpoint hostname,
  reachable over its configured UDP port. `api.woven.host:4434/webtransport`
  is a documented endpoint, not evidence of verified live reachability.
- Allow the exact page origin `https://jdharrison.com` on the node. Only add
  `https://www.jdharrison.com` if actually serving there. Origin rules do not
  protect a copied token from non-browser clients.
- **Plan any restart before changing the node:** managed sessions/verifiers are
  in memory, node incarnation changes, and Host does not automatically replay
  provisioning. Preserve existing users through an approved recovery/reprovision
  plan; do not casually restart a shared node for an origin edit.

Authoritative references: sibling `woven/docs/managed-sessions.md`,
`woven/docs/deploy-woven-01.md`, and `woven/ops/deploy/woven-server.service`.

## Anonymous bootstrap contract

The shell's [browser README](../examples/first-person-lab/platforms/web/README.md)
contains the exact v1 schema and HTML metas. The non-secret target config pins
an exact endpoint and decimal-string scope IDs. The same-origin bootstrap route
must return precisely that target plus a 64-lowercase-hex Bearer token.

Before supplying an implementation, confirm the portfolio Firebase project and
hosting site, backend choice/region, public-token acceptance, and cost limits.
For Firebase Functions, bind the shared credential from protected backend
configuration; never put it in a checked-in file or browser build environment.
Use `Cache-Control: no-store, private`, JSON content type, no token/body logging,
bounded request/response processing, throttling, and explicit concurrency/cost
caps. Throttling the bootstrap is **not** Woven authorization or a defence against
reuse of a previously copied session token. Do not expose Host owner credentials,
node admin tokens, TLS private keys, or encryption/signing keys.

The client fetches at most once per initial/manual attempt, with a 10-second
request/body deadline and 8 KiB ceiling. Credentials stay in memory. No automatic
retry storm or invented expiring-token semantics. The existing owner-only Host
connection endpoint is not an anonymous bootstrap route.

## Build and local checks

Run from the Weaver repository root, with the compatible sibling Woven checkout:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
cargo build --locked --workspace --all-targets --all-features
cargo xtask check first-person --platform all
cargo xtask build first-person --platform web
node scripts/check-web-artifact.mjs dist/first-person
```

Shell checks (from `examples/first-person-lab/platforms/web`):

```sh
npm run typecheck
npm test
```

`xtask` checks the wasm-bindgen CLI against Cargo.lock, uses locked Cargo/npm
inputs, copies only runtime files/licenses, validates the final artifact, and
replaces `dist/first-person` only after success. Never deploy `woven.local-token`.
The standalone validator rejects that file **before reading it**, plus unexpected
source/tests/dotfiles, symlinks, and missing/truncated runtime artifacts.

The demo-only Firebase mapping has no default production project. Port 8002
avoids the normal local development server on 8000. This bounded smoke starts
and stops only the Hosting emulator and does not connect to Woven:

```sh
CI=true FIREBASE_CLI_DISABLE_UPDATE_CHECK=1 firebase emulators:exec \
  --project demo-weaver-portfolio --only hosting \
  "node scripts/test-hosting.mjs"
```

It checks served runtime assets, WASM MIME/validity, security/cache headers, and
404 responses for missing assets, source, credentials, and the not-yet-configured
bootstrap route. It is not a renderer or two-peer networking test.

An optional real offline browser smoke uses installed snap Firefox/geckodriver
and Python stdlib, a fresh isolated profile, a deny proxy for non-loopback browser
background traffic, and bounded execution. Create `target/local-browser-qa`, then
run from that directory:

```sh
timeout --signal=INT --kill-after=3s 85s \
  env CI=true FIREBASE_CLI_DISABLE_UPDATE_CHECK=1 \
  firebase emulators:exec --config ../../firebase.json \
  --project demo-weaver-portfolio --only hosting \
  "python3 ../../scripts/test-browser-local.py"
```

Reports/screenshots stay under ignored `target/local-browser-qa`; never upload
that directory. This smoke requires empty visitor metas and never joins a lobby.

## Redistribution notices — web packaging omission fixed

The packager retains Weaver's Apache license, Noto Sans's full OFL copyright/license,
and, for First-Person, Woven, Woven Client, and FlatBuffers licenses. Existing
`NOTICE`/`NOTICE.txt` files from those roots remain supported; none were present
in the reviewed inputs. No synthetic Apache NOTICE is invented.

All three web labs now also require `licenses/Rust-INVENTORY.json` and
`licenses/Rust-THIRD-PARTY.txt`. The reviewed snapshot is derived from each lab's
locked, default-feature, **normal wasm32 target graph**, excluding host proc-macro
subtrees and build/dev tools. Staging rejects package/feature/license/source drift
and verifies exact installed upstream legal bytes. Validation compares the declared
inventory and complete notice bundle against the trusted repository snapshot, not
against artifact-provided hashes. Fixed allowlists, regular-file/ancestor-symlink
checks, validation-before-publish, and rollback remain intact.

The First-Person omission is resolved: full `fontdb` MIT and `rustybuzz` MIT/HarfBuzz
notices are present, along with the other reviewed notices, combined-license
obligations and embedded source notices. See [WEB-LICENSES.md](WEB-LICENSES.md)
for choices, exceptions, counts, and the **separate native inventory route**.
This is a bounded engineering review, not legal proof or a whole-toolchain audit.
Native archives still need their own target-specific dependency notices, Weaver
license and full Noto OFL alongside binaries; the WASM bundle is not sufficient.

### Focused packaging validation (2026-10-03)

Historical packaging-only pass, before final source identity and MSRV alignment:

- Formatting and strict xtask Clippy passed with locked/offline Cargo inputs.
- All **21 xtask tests** passed, including **9 Node licensing regressions**. The
  latter individually remove/truncate every distinct First-Person legal text and
  check combined obligations, graph/license drift, symlinks, and filename rejection.
- Offline First-Person release web build passed with the sibling TypeScript client
  **0.3.0**, including locked npm install, client build, and shell typecheck/bundle.
- Final standalone artifact validation passed: **12 files**, up from the historical
  10-file artifact; **118** normal WASM packages (**112** registry / **6** local),
  **181** legal-source references across **168** source files, **63** distinct texts.
- Demo Hosting on port **8002** passed **13 HTTP checks** (8 served assets and
  5 expected 404s), including exact served inventory/notice text and release
  headers. Firebase reported stale authentication but the demo test passed;
  no reauthentication was attempted, and live deployment remains gated.
- These focused checks did not repeat the historical renderer/browser smoke or
  the full native/workspace test suite, nor establish the final source revision
  pair. At that stage, lockfile/CI metadata and native archive review were still
  separate gates.
- No production/cloud/DNS/secret changes, deployment, or Woven network traffic.

## Final-candidate local validation

The latest comprehensive local pass after the version/lockfile and Rust 1.98
updates passed the following. These checks are recorded from that pass, not all
newly rerun for this documentation handoff:

- **168 Rust tests**, plus **9 Node licensing regressions** invoked by xtask;
  all **45 browser-shell tests** and shell typechecking passed.
- Locked/offline workspace build across all targets/features, First-Person
  desktop + WASM checks, and Render/Space web checks passed.
- Offline First-Person release web rebuild and final standalone validation passed:
  **12 files**. All three reviewed licensing snapshots matched fresh locked normal
  target graphs and exact upstream legal bytes after the MSRV change; no inventory
  update was needed. Reviewed local crate versions remain **0.2.0**.

Final confirmation at clean Woven
`2bc74d3298b2f317cc00a1508b09b54250c612d8`, with Weaver's behavior-equivalent
`Duration::from_mins(15)` adapter fix, passed **formatting**, **strict locked/offline
workspace Clippy**, and all **33 focused weaver-woven adapter tests**. The full
168-test suite, shell tests and web build above were not repeated by this final
format/lint/adapter-only confirmation.

Earlier demo Hosting on **8002** passed **13 HTTP checks** (8 assets / 5 expected
404s), including complete served notices and release headers. Earlier isolated
headless Firefox **155.0.1** passed **14 startup checks**, reached
**`Weaver ready on Gl`**, rendered the room, and had no console errors or external
page requests. **Neither HTTP/emulator nor Firefox smoke was newly rerun for this
handoff.** They did not exercise live Woven, anonymous bootstrap, interactive
keyboard/pointer acceptance, two-peer networking, WebGPU or mobile chat. Historical
Firebase authentication warnings do not block demo checks but still require
separate operator attention before any approved deployment.

Source-candidate credential review is limited to tracked/nonignored text files;
ignored environment/private files and generated artifacts are not scanned. No live
credential was identified; the reviewed material includes documented QA credential
file paths and a public throwaway test-only TLS identity, not production secrets.
This is a bounded scan, not proof that every possible credential form is absent.

Local commits are authorized separately; publication and deployment remain gated.
Native archive notice review remains a separate redistribution gate; web checks
do not establish native licensing completeness.
No hosted/shared Woven traffic, cloud/DNS/secret mutations or GitHub run occurred.

## Hosting and activation

`firebase.json` publishes **only** `dist/first-person` to target `portfolio`, with
artifact validation as a predeploy gate and independent secret/source exclusions.
It intentionally has no SPA catch-all; missing JS/WASM must remain 404. Fixed-name
assets revalidate, WASM has `application/wasm`, and framing/referrer/unused-permission
headers are set. CSP is **Report-Only**, not an enforced security boundary; test
real WebGPU/WebGL2 and WebTransport before enforcing it, and adjust `connect-src`
if the approved realtime endpoint differs. No unnecessary COOP/COEP is added.

Do not use sibling `woven-host/firebase.json` or its Hosting site for this release.

Activation order, each infrastructure step requiring approval:

1. **Source publication gate:** after explicit push approval, push Woven commit
   `2bc74d3298b2f317cc00a1508b09b54250c612d8` **first**, then the Weaver commit
   containing its 0.2.0 manifest/handoff and matching exact CI pin. Record the
   actual Weaver commit ID externally. Wait for the real GitHub validation/build
   jobs on that published pair, inspect the uploaded web artifact/notices, and
   complete target-specific native notice review before distributing native
   archives. The CI configuration is present; no remote run or push has been
   performed or authorized by this handoff. Do not tag or deploy implicitly.
2. Confirm and approve Firebase project/site, apex vs www, budget, and Woven room
   limits. The anonymous bootstrap backend, dedicated public lobby and live
   acceptance are still absent/unverified; source/CI success is not MVP activation.
3. Plan and apply any endpoint/origin updates; recover/provision the dedicated
   lobby after any node restart. Obtain its client descriptor privately.
4. Implement/deploy the approved bootstrap, verify non-cacheability and caps,
   configure the two non-secret metas, and rebuild/revalidate the web artifact.
5. Map Hosting target `portfolio` to the confirmed site using explicit
   `--project`, then deploy **only** `hosting:portfolio`. Configure the custom
   domain/certificate without changing unrelated sites or APIs.
6. Run approved, bounded two-browser QA before announcing public multiplayer.
   Do not label local builds or successful HTTP asset tests as deployed proof.

Firebase CLI may require interactive reauthentication before real deployment;
local demo Hosting does not prove the operator's cloud credentials are valid.

## Browser acceptance and rollback

- Two separate visible browsers: join, late join, move, see real capsules/names,
  bidirectional chat, rename, disconnect, manual rejoin, full/queued admission.
- Opposite room corners still see/chat with each other.
- Real renderer readiness, WebGL2 fallback, resize/DPI, mouse lock, keyboard chat,
  unavailable WebTransport, and actionable graphics/bootstrap/admission failure.
- Hidden tabs disconnect intentionally to prevent bounded inbox overflow while
  browser redraws are throttled. Restoration requires manual rejoin. Test sleep,
  pending-handshake cancellation, and back-forward cache as well as tab switching.
- Malformed/oversized chat messages are dropped individually; they must not
  disconnect recipients. An actual transport/protocol/inbox failure stays visible.
- Mobile virtual-keyboard chat is not an MVP claim.

For frontend problems, restore the previously verified Hosting release. For
credential exposure/abuse, stop distributing the descriptor and **revoke the
session**, not just the bootstrap; old copied tokens otherwise still work.
Recreate the lobby with a fresh credential/scope, update the target/bootstrap,
then rejoin. Whole-session revocation disconnects every visitor. Static rollback
alone does not restore a deleted session or reverse node incarnation changes.
