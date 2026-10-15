# First-Person Lab browser release seam

This shell is desktop-first for chat: a physical keyboard opens/submits chat with
Enter and cancels with Escape. In minimal presentation, Enter requests the host's
on-demand composer; the host owns text/IME/mobile keyboard input. An accepted send
closes the host composer. Messages appear briefly above player avatars, with the
local player's message at the top-center HUD in first person instead. There is
no visible transcript in minimal presentation. Existing touch movement/look remain
available; the standalone shell does not supply a mobile virtual-keyboard composer.

The camera starts at a 2 m third-person setback, collision-clipped before its first
snapshot (1.8 m at spawn). Wheel zoom spans 0–6 m; zero selects first person, and
middle-click toggles the remembered third-person distance. Player and NPC names
remain visible in minimal presentation, independently of hover or interaction.
The local name appears only in third person. Name/speech anchors respect physical
visibility and projection bounds; names follow the NPC portraits' scripted bob.
Speech is offset above each name, and retains its four-second hold/two-second fade.

## Scripted ambient NPCs; no interaction or inference

The shared Rust scene embeds version-3 `../../assets/npcs.json`: five ambient
slots (`guest_01`–`guest_05`), all with authored display names and interaction
disabled. One lobby resident, two theater residents and two non-overlapping door
visits use a bounded, static 600-second UTC schedule. Portraits visibly bob;
NPCs do not react to players, block them, or become interaction targets.

There is no concession counter/support NPC, player-like guest simulation,
randomized disconnect behavior, TALK/hover prompt, AI call, private conversation,
or NPC Woven entity. Routes are compiled once and sampled analytically, independent
of movie playback, player input and networking. Invalid/backward UTC freezes the
last safe poses until recovery; large forward jumps sample the new phase directly.

The chair-free auditorium is 31.5 × 28 m with viewing levels at 0, 1.2 and 2.4 m,
joined by ramps. Legacy seat IDs are standing marks only. The 25 × 10 m lobby and
hallway have 4.5 m of vertical headroom; the hallway ceiling follows its ramp and
opens into the theater's world-Y=12 m ceiling at Z `-12..-9`. A lintel seals the
void above the hallway. Rendering, camera clearance and visibility share the same
floor/ceiling profiles.

### Inert compatibility bridge

The existing exports remain for older hosts, but cannot enable dialogue:

- `weaverGetNpcInteractionContext()` / `get_npc_interaction_context()` returns null.
- `weaverSubmitNpcQuestion(...)` / `submit_npc_question(...)` returns false.
- `weaverPresentNpcResponse(...)` / `present_npc_response(...)` returns false.
- `weaverClearNpcDialogue()` / `clear_npc_dialogue()` is inert.
- `weaverGetNpcDefinitions()` exposes the embedded ambient catalog, not interaction
  authorization. Readiness/show-cycle compatibility setters cannot reactivate NPCs.

Enter requests ordinary player chat through the existing host bridge. Hosts must
continue to reject unexpected private intent rather than route it into public chat.
The theater sound/fullscreen controls remain separate from NPC interaction.

Focused checks: `cargo test --locked -p first-person-lab --lib`, browser `npm test`
and `npm run typecheck`. Build using `cargo xtask build first-person --platform web`;
its validated artifact is `dist/first-person`. Stage it separately in the host site;
never edit generated JS/WASM or copy source JSON/PNGs into the release export.

## Embedded load progress

The browser shell emits JSON `weaver-host` v1 `progress` messages to its parent,
with `stage`, integer `completed`, and `total: 14`, targeting only its own origin.
Milestones are shell setup (1), module import (3), WASM initialization (7), local
profile/presentation setup (10), scene start (12), and initialized renderer
readiness (14). These are discrete initialization milestones, not download-byte
percentages. Repeated/regressing milestones are suppressed and fatal startup
stops progress. No credentials, diagnostics, or network admission state enter
these events; multiplayer/movie loading does not gate scene reveal. The existing
host adapter continues to own `ready`/`fatal` notifications. A parent must validate
both origin and the active iframe source before accepting any bridge event.

## Theater video proof of concept

`TheaterVideoPlayer` in `theater-video-player.ts` owns a hidden decoder element
**inside the engine iframe**. It is not a DOM movie overlay: Weaver consumes its
frames on the 3D theater screen. The theater panel is hidden by default in minimal
presentation, retaining its functional source controls for full presentation.
Sound and fullscreen are in-world WASM-picked panels in both modes; the old HTML
Enable sound prompt is removed entirely. Full diagnostic controls include
Play / Pause, Restart (seek to zero without changing playback),
Mute / Unmute, Volume, seek/time/status, and independent lobby/auditorium lighting. Control focus releases
held movement keys and pointer lock; its input suppression is OR-combined with
host-chat focus, so leaving one control cannot unlock input owned by the other.
Lighting defaults match the scene (lobby 1, auditorium 0.18).

Default movie URL (same-origin, root-absolute):
`/weaver/assets/theater/Music_SpaceNoirJazz.mp4`. The parent site stages this
long movie: 3981.386 seconds (about 66m21s), 638×360 at 30 fps, H.264 Main
video with stereo HE-AAC at 44.1 kHz, approximately 91 MB, with faststart.
Both earlier clips remain unchanged: `Jellyfish_1080_10s_10MB.mp4` (video-only)
and `Jellyfish_1080_10s_audio_test.mp4` (synthetic AAC test audio). The standalone
engine does not include these assets and can legitimately show a missing-media error.
Override with `meta[name="weaver-theater-video"]` using another same-origin
root-absolute path (empty uses the default). Remote/relative URLs are rejected.
Loading may make same-origin HTTP media/range requests. Audio uses only the
same browser video element: no separate audio player, AudioContext, or spatial
audio. UTC schedule selection is local wall-clock presentation, not an authoritative
multiplayer clock or sample-accurate A/V synchronization.

The reusable component's `autoplay` option defaults to `false`; opting in starts
muted. `main.ts` uses `scheduled: true`: the element starts **muted** at volume
0.4, inline, with `preload="metadata"`. Constructor/readiness autoplay is disabled
in this mode. There is no native `autoplay` attribute; the schedule controller
authorizes a muted attempt only after attachment, initialized scene readiness,
visibility, and schedule alignment (including completion of an asynchronous seek).
This works under policies that block audible autoplay but permit muted playback.
If even muted playback is denied, “Autoplay blocked/unavailable · press Play”
leaves a manual retry available. Play preserves the current mute setting.
The in-world sound panel calls schedule `play(true)` synchronously in the trusted
canvas gesture: align the live slot, unmute, restore volume 0 to 0.4, and call
`media.play()`. An ON hit mutes without pausing; ordinary diagnostic Mute/Volume
controls never resume media. No fetch/await precedes play. Pending is bounded to
one promise; a denied attempt reports structured retry rather than optimistic ON.
ON requires observed `playing`, unmuted media, nonzero volume, and no pause/end
or buffering; it does not verify physical speaker output. Repeated readiness/media events
never retry an attempted autoplay, and explicit Pause before readiness cancels
it for the current source. The controller allows one attempt per source/entry occurrence, while explicit
Pause survives slot/source changes. Source replacement preserves explicit
mute/volume preferences and waits for any old play promise to settle. Mute / Unmute works
while playing; its explicit preference and volume survive pause, replay, and
hide/restore. Volume accepts only finite numbers, clamped to 0–1, and never
unmutes or starts playback. Audio controls never resume paused media. Browser
`volumechange` events synchronize status and controls, including external changes;
“audio enabled” describes media settings, not verified speaker output. Browser
play rejection leaves the attachment ready for an explicit retry. Attachment happens once per
source only after metadata **and** initialized renderer readiness. A decoded
`requestVideoFrameCallback` marks a new frame dirty for the next Weaver redraw;
it does not upload directly or run another renderer. WebGL2 copies the video
source directly; WebGPU stages through a reusable detached 2D canvas before
copying into the same renderer-owned texture. This avoids direct-import failures
for software-decoded video frames without GPU backing; neither path performs
`getImageData` readback. Browser-internal acceleration/copies remain platform-dependent.
The cancellable fallback
polls `currentTime` with RAF only during playback and never marks unchanged ticks.
There is at most one frame callback and one outstanding play promise, with no
queued play requests. Paused seeking marks the available frame after `seeked`.

Hidden/pagehide pauses and silences the same video and cancels video work;
restoration never resumes playback
without another user gesture. Video Pause does **not** disconnect multiplayer
(the existing page-hide network policy is unchanged). Fatal scene failure and
`globalThis.weaverDisposeTheaterVideo()` detach, remove the media source, call
`load()` to release decoding, and remove the element. The site host should call
that idempotent hook before shutting down/removing the iframe. Late play promises
and cancelled frame callbacks cannot restart disposed/hidden media.

The manually intersected `FirstPersonWasm` type expects these Rust exports:
`attach_theater_video(HTMLVideoElement): boolean`,
`mark_theater_video_frame(): boolean`, `detach_theater_video(): void`, and
`set_room_lighting(lobby: number, auditorium: number): boolean`,
`pick_control_panel(x: number, y: number): number`,
`set_control_panel_state(panel: number, state: number): boolean`, and
`invalidate_control_panels(): void`.

Panel IDs are 0 none, 1 sound, 2 fullscreen; states are 0 unavailable, 1 OFF,
2 pending, 3 ON, 4 retry. Picking uses canvas-relative NDC [-1,1], Y up;
pointer lock picks the crosshair (0,0). Capture handlers consume panel hits
before pointer lock/input, leave misses untouched, and activate touch only on
an undragged tap (8 CSS pixel movement threshold). Synthetic input cannot activate
panels. Shutdown removes handlers/listeners and invalidates panels once.

The parent installs `jdharrisonToggleFullscreen()` and
`jdharrisonGetFullscreenState()` **on the child window**. The getter and child-window
`CustomEvent("jdharrison-fullscreen-state")` detail share the runtime-validated
`{available:boolean, active:boolean, pending:boolean, error:boolean}` contract.
The shell calls the toggle directly in the gesture stack and projects observed
state, never optimistic active state. Without the bridge it uses this document's
Fullscreen API and fullscreenchange/fullscreenerror events. It never accesses the
parent document. No site files or generated WASM assets are changed here.
Matching WASM/generated bindings are required; these browser changes do not
regenerate or stage them. Media/attachment failures are visible in the theater
status without making a healthy network connection fail.

Node fake-media/scheduler and browser-shell tests cover readiness, frame cadence,
fallback, seeks, cancellation, stale promises, errors, disposal, chat focus,
one-shot muted autoplay and rejection/manual retry, gesture-enabled audio, source
replacement during pending playback, persistent explicit mute, bounded volume,
and external audio changes.
The parent site's bounded `tool/theater_video_smoke.mjs` checks actual H.264
frames, texture reuse, pause/seek/restart/loop, lighting, lifecycle cancellation,
and shutdown in offline Linux Chromium under both WebGL2 and WebGPU. It supplies
compatible Vulkan/ANGLE/Dawn flags for this development environment and records
GPU details rather than implying other systems are verified. Screenshots permit
visual confirmation of the in-world screen. Touch/mobile/Safari QA, actual
background-tab policy, audible output/AAC browser support, and real A/V timing
remain unverified by these Node tests; lifecycle
stimuli in the smoke are synthetic. No deployed validation is implied.

### Subtitle limitation

No caption/subtitle tracks, WebVTT/SRT files, or transcripts are supplied by this
lab. Subtitles therefore cannot currently be displayed by default. The hidden
video is a decoder source uploaded as a texture: browser-native `<track>` captions
are not included in those pixels. No transcripts or empty caption controls are
invented. Adding subtitles requires real timed caption assets and an explicit
cue-text overlay (or renderer composition) synchronized to the media clock and
source changes; simply enabling native tracks would not show captions in-world.

## UTC theater schedule

`theater-schedule-controller.ts` exports `loadTheaterSchedule`,
`DEFAULT_THEATER_SCHEDULE`, and `TheaterScheduleController`. It consumes the
shared `validateTheaterSchedule` / `resolveTheaterSchedule` model without changing
that schema. `TheaterVideoPlayer.scheduledAutoplay()` is the small readiness/busy
seam; the existing player still owns the only decoder and pending play promise.

`meta[name="weaver-theater-schedule"]` defaults to `/weaver/theater-schedule.json`.
The manifest is fetched **once** per shell instance, with a 32 KiB streaming bound
and a 10-second deadline covering request and body. Only a canonical same-origin
root-absolute path is accepted: no credentials, queries, fragments, percent aliases,
or dot segments. Requests omit credentials/referrers, use `cache: "no-store"`,
reject redirects, and require JSON. Failure is isolated from scene/network startup
and visibly reports **Schedule unavailable**, without claiming synchronization or
silently falling back. Uncheck **Follow schedule** for local manual diagnostics
using the movie meta/default; there is no automatic local playback fallback.

The parent site supplies/stages the manifest; this source checkout does not create
or deploy it. The intended default is:

```json
{
  "version": 1,
  "revision": "space-noir-jazz-v1",
  "anchorUnixSeconds": 0,
  "cycleSeconds": 3600,
  "driftToleranceSeconds": 5,
  "entries": [{
    "id": "space-noir-jazz",
    "title": "Space Noir Jazz",
    "startSeconds": 0,
    "durationSeconds": 3600,
    "source": "/weaver/assets/theater/Music_SpaceNoirJazz.mp4",
    "sourceStartSeconds": 0,
    "loop": false
  }]
}
```

Follow schedule defaults on. Its separate status shows slot title, current UTC
hour/time, observed drift, revision, and short-source holding. Restart/seek are
disabled in this mode; Play/Pause and local Mute/Volume remain usable. Manual mode
restores restart/seek and the old asset looping behavior. Toggling back to Follow
pauses and aligns before any new automatic attempt, unless the user paused or
previously hid the page. Manual Play computes/seeks the live slot and calls the
existing `play()` synchronously within the gesture; no fetch/await consumes it.

Selection uses `Date.now() / 1000` once per second while visible, and media
readiness events complete pending alignment. Initial join, entry/cycle rollover,
follow toggle, restore, and detected clock jumps force alignment; otherwise only
drift **greater than** the manifest tolerance seeks (exactly 5s does not for the
default). Active `media.seeking` is ignored. Same-asset slot/offset changes do not
reload the source. Looping maps elapsed time into the available duration after
`sourceStartSeconds`; natural source end realigns/restarts an already-playing loop,
not a blocked autoplay. Non-looping media shorter than its slot holds the last
frame silently until the next slot. Audio preferences are never reset by scheduling.

Hidden/pagehide cancels the timer and pauses audio. Restore computes the live
position but requires a Play gesture, even across a subsequent slot change.
Fatal/route disposal aborts the manifest request and cancels the controller before
the player; stale callbacks cannot revive either. No hosted/Woven/time service is
used by this integration. Device clock error, seek/decoder precision, buffering,
and browser autoplay policy limit alignment; these tests do not verify audible
output or sample-accurate synchronization. Parent browser QA/staging tooling must
be updated separately for the new meta and schedule-controlled controls.

Offline tests include fake clocks/timers/media, two-client selection, strict drift
threshold, asynchronous alignment-before-muted-autoplay, source/offset transitions, cycle
wrap, pause/restore/disposal, blocked autoplay, loop/short-source behavior, and
bounded manifest rejection. Run `npm run typecheck` and `npm test` in this directory.

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
