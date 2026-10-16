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
middle-click toggles the remembered third-person distance. Two fingers starting on
the right half pinch to zoom (apart to zoom in, together to zoom out), without
changing the existing left-move/right-look gesture. Look pauses during a pinch;
ending/cancelling it does not replay accumulated look motion. Player and NPC names
remain visible in minimal presentation, independently of hover or interaction.
The local name appears only in third person. Name/speech anchors respect physical
visibility and projection bounds. NPC names share the local player's color and
label formatting; their portraits stay grounded along the scripted routes.
Speech is offset above each name, and retains its four-second hold/two-second fade.
Remote speech additionally requires a finite 3D distance of at most 10 m between
the physical local player's eye and the remote displayed eye; camera zoom cannot
change the radius. Out-of-range messages keep their original expiry clock. Names
and local overhead/HUD echo are not distance-gated. This is a client-side display
rule, not private routing: reliable chat still uses the existing space-wide channel.

## Scripted ambient NPCs; no interaction or inference

The shared Rust scene embeds version-3 `../../assets/npcs.json`: five ambient
slots (`guest_01`–`guest_05`), all with authored display names and interaction
disabled. One lobby resident, two theater residents and two non-overlapping door
visits use a bounded, static 600-second UTC schedule. Walking stays grounded
without bobbing. Theater residents sit in `seat_b7` and `seat_b2`; both visitors
walk to `seat_b12`, sit for 24/26 seconds, then stand and continue their exits.
The lobby resident stays in the lobby. Seated NPCs use the same cropped billboard
presentation and eye-relative labels as seated players. NPCs remain noninteractive.

There is no concession counter/support NPC, player-like guest simulation,
randomized disconnect behavior, TALK/hover prompt, AI call, private conversation,
or NPC Woven entity. Routes are compiled once and sampled analytically, independent
of movie playback and player input. Explicit `resident_seat` schedules and `sit`
actions validate chair IDs, approach access and non-overlapping allocations.
NPC claims make seated chairs unavailable to new player entry. Already-seated
local/remote players take priority: guests wait at the approach and retry while
their scheduled sitting interval remains active. This is local advisory contention,
not a server reservation. Invalid/backward UTC freezes the schedule until recovery;
contention still reconciles, and large forward jumps sample the new phase directly.

The auditorium is 31.5 × 28 m with viewing levels at 0, 1.2 and 2.4 m,
joined by ramps. Twenty-four low-backed chairs sit on the middle/rear flats in
two rows, each with six seats left and six right of the 3.1 m center aisle.
Ramps remain clear; visiting NPCs use the rear cross-aisle around the wider banks.
The original eight seat IDs/indices remain stable; sixteen seats extend outward.
The 25 × 10 m lobby and
hallway have 4.5 m of vertical headroom; the hallway ceiling follows its ramp and
opens into the theater's world-Y=12 m ceiling at Z `-12..-9`. A lintel seals the
void above the hallway. Rendering, camera clearance and visibility share the same
floor/ceiling profiles.

### Player seating

A visible unoccupied chair within 2.5 m of the physical player can be clicked or
tapped. Seating forces first person, faces the screen, and suppresses movement,
sprint and perspective/zoom changes while retaining look/chat. A scene click/tap
stands at a validated exit and restores prior view preferences with current look.
The combined interaction dispatcher preserves synchronous wall-control actions,
exact-target touch release, drag/hold/multitouch rejection, and capture-only mouse
resume. The contextual sit hint remains available in minimal presentation;
there is no stand prompt while seated, but click/tap still exits the chair.
`pick_scene_interaction` returns panels 1–3, portfolio halves 4–5 (GitHub/LinkedIn),
seats 100–123 or seated exit 200;
`activate_seat_interaction` queues one revalidated intent. Temporary lifecycle
invalidation clears intents and snapshots; permanent disposal cannot revive them.
Portfolio links open synchronously from trusted clicks/taps in isolated new tabs.
Final destinations live in `portfolio-links.ts`: GitHub `jdharrison` and LinkedIn
`jacobdharrison`. The plaque's supplied SVG QR codes and matching URL captions
are baked into `assets/portfolio_wall/portfolio_wall.png` by the offline importer;
there are no runtime SVG, QR-generation, or font-download dependencies.

Seating works offline. Known occupied chairs are unavailable; simultaneous claims
converge to the lowest entity ID, without an authoritative reservation guarantee.
Remote seated billboards crop the lower body rather than animating a skeleton.

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

## Browser rendering quality

The browser renderer fits its drawing buffer within 1920×1080 (1080×1920 in portrait),
preserving aspect ratio without upscaling smaller physical viewports. HiDPI uses
physical pixels up to this ceiling and the GPU device limit. Capability-gated 4×
MSAA remains enabled; static textures are reduced to existing mip levels no larger
than 2048 pixels per dimension before GPU allocation, with trilinear filtering and
capability-gated 4× anisotropy (1× fallback).

This replaces the overly soft 960×540 drawing buffer and 1024-pixel texture cap:
engine text/UI, sprites, posters and projected movies all benefit from the larger
buffer. Full-window projection, picking and overlay layout coordinates stay
unchanged. Logical texture dimensions retain sprite proportions and calibration.
Movies retain their existing decoded-resolution, single-mip upload path (up to
1920×1080 pixels); no source-video upscaling or re-encoding is performed.

At the ceiling the buffer contains 4× as many pixels as the old profile,
and a texture can use up to 4× the GPU storage. GPU frame time and memory use need
real-device review, especially on mobile. PNG payloads and CPU decoding are
unchanged. Native renderer quality is unchanged. These are defaults, not a
graphics settings menu.

Scene names use bold 24 logical-pixel text, avatar speech uses 36 logical pixels,
and the seating prompt uses bold 26 logical pixels on a dark backing. Browser
pixel density scales those sizes and their associated spacing. Glyphs are
rasterized for the final drawing buffer rather than shrinking a higher-resolution
window's glyph bitmaps; the scene, including its text, still respects the 1080p
ceiling. The non-minimal native chat/status layout also uses larger type and rows.

Browser HUD text is rendered directly by the DOM at readable CSS sizes, independently
of the scene drawing-buffer limit: status/help and controller feedback use 1rem
(normally 16px), connection branding uses 0.75rem (12px), native form/buttons and
the settings summary use 1.125rem (18px), and captions use a responsive 1.25–2rem
(20–32px). The connection badge has reserved space above help/settings; enlarged
panels wrap or scroll within viewport/safe-area bounds instead of shrinking text.
Controller sprite artwork, calibrated button geometry, and image filtering are
unchanged; text is not enlarged through transform scaling. Controller artwork
labels remain part of the images, not CSS text.

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

`wasm-loading.ts` also observes the single response passed to the WASM initializer,
without buffering/cloning the binary or replacing streaming compilation. It emits
`download` messages with `loadedBytes` and `totalBytes` (integer or `null`), followed
by `initializing` once the response body is consumed. Download reports are throttled
to about 100 ms, with unconditional initial/final reports. Counts are decoded body
bytes, not compressed wire traffic: gzip/br, absent, or contradictory lengths do
not supply a total. The response is bounded to 512 MiB. Neither these messages nor
WASM initialization alone authorize scene readiness; the host still owns `ready`.

## Theater video proof of concept

`TheaterVideoPlayer` in `theater-video-player.ts` owns a hidden decoder element
**inside the engine iframe**. It is not a DOM movie overlay: Weaver consumes its
frames on the 3D theater screen. The theater panel is hidden by default in minimal
presentation, retaining its functional source controls for full presentation.
Minimal web uses the browser controller HUD described below; native and non-minimal
browser presentation retain world-space Sound, Subtitles and Fullscreen panels.
Minimal web skips world-panel geometry and picking, but preserves seat interactions.
Minimal presentation opens the existing expanded controller once after combined
WASM/renderer readiness and visibility, without constructing the legacy startup
confirmation or waiting for media. The current UTC movie still starts muted
independently (`waitForGesture: false`). Non-minimal diagnostic presentation retains
the legacy first-run confirmation with **Play with sound** as the preferred choice. **Play with sound** synchronously aligns,
unmutes and calls `play()` in the trusted click; success closes the confirmation
after observed audible playback and a decoded frame accepted by the bridge. If
audio is rejected, one muted fallback attempt is allowed; silent playback success
plus a decoded frame also dismisses startup. **Continue without sound** (or Escape)
enters immediately and permits muted playback when ready. After manifest failure,
that explicit choice loads the local silent fallback once, with Retry still
available. No movie collection or diagnostic fallback is automatically downloaded
while the schedule is healthy. Preparation/starting that takes
12 seconds offers Retry and the silent fallback; manifest/media failures have
explicit retries, not polling download loops. Retry reloads only the selected
clip (or the failed manifest) and requires a fresh Play gesture for audio. A UTC
boundary during that click prepares the new clip and offers Play again rather
than replaying audible intent after an await. The dialog traps focus, releases
pointer lock, blocks game input and cancels its timer while hidden/disposed.
Full diagnostic controls include
Play / Pause, Restart (seek to zero without changing playback),
Mute / Unmute, Volume, seek/time/status, and independent lobby/auditorium lighting. Control focus releases
held movement keys and pointer lock; its input suppression is OR-combined with
host-chat focus, so leaving one control cannot unlock input owned by the other.
Lighting defaults match the scene (lobby 1, auditorium 0.20).

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
in this mode. There is no native `autoplay` attribute. The shell sets the schedule
controller's `waitForGesture: false`: muted playback proceeds after attachment,
visibility and UTC alignment (including completion of an asynchronous seek), even
while the controller (or diagnostic first-run prompt) remains visible. It does not
wait for the whole clip to download. Minimal startup sets desired audio OFF once
with `player.setMuted(true)`, before playback; the first ready Audio press therefore
enables sound directly in the gesture. Diagnostic mode retains desired audio ON
initially (`state.audioEnabled`), separately from actual muted autoplay. Neither
desired preference nor a successful play promise is proof of audible output.
If even muted playback is denied, “Autoplay blocked/unavailable · press Play”
leaves a manual retry available. Play preserves the current mute setting.
In non-minimal browser mode, the world Sound panel reopens the startup confirmation
and, if metadata is ready, calls schedule `play(true)` synchronously in the trusted
canvas gesture: align the live slot, unmute, restore volume 0 to 0.4, and call
`media.play()`. Loading/failure states offer preparation/retry. An audible ON hit
mutes without pausing. Minimal HUD Audio instead uses desired `state.audioEnabled`:
OFF calls `setMuted(true)`; enabling ready audio directly calls `schedule.play(true)`
without a second startup confirmation. Retry can reload media/manifest, but needs
a fresh Audio gesture when ready; no delayed audible intent survives loading.
Ordinary diagnostic Mute/Volume controls never resume media. No fetch/await precedes
play. Pending is bounded to one promise; a denied attempt reports structured retry
rather than claiming audibility. World Sound-panel ON requires observed `playing`,
unmuted media, nonzero volume, and no pause/end or buffering; it does not verify
physical speaker output. HUD Audio ON instead reflects desired preference. Repeated readiness/media events
never retry an attempted autoplay, and explicit Pause before readiness cancels
it for the current source. The controller allows one initial attempt per source/entry occurrence;
established playback may also resume after visibility restoration or bounded decoder recovery.
Explicit Pause survives slot/source changes and both recovery paths. Source replacement preserves explicit
mute/volume preferences and waits for any old play promise to settle. Mute / Unmute works
while playing; its explicit preference and volume survive pause, replay, and
hide/restore. Volume accepts only finite numbers, clamped to 0–1, and never
unmutes or starts playback. Diagnostic Mute/Volume controls never resume paused
media; explicit Audio enable uses the gesture-bound schedule Play path. Browser
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
There is at most one frame callback and one current play attempt, with no
queued play requests. Paused seeking marks the available frame after `seeked`.
While playback is intended and the scene is visible, one cancellable 1-second
watchdog polls for unexpected pauses and missing decoded frames. A pause is
retried on the next poll; an 8-second frame stall first gets a lightweight
pause/play restart, then at most one decoder reload if still frozen. An unsettled
play or reload times out after 15 seconds. Decoder recovery restores the manual
position or realigns the live UTC schedule before replay; advancing clocks and
schedule seeks do not conceal a frozen frame pipeline. Repeated failure exposes
Retry instead of redownloading forever. Thirty seconds of healthy decoded-frame
progress replenishes the recovery budget. Explicit Pause, hidden/unready scenes,
policy denial, natural endings and disposal cancel recovery. Timed-out play
promises are retired so late settlement cannot interrupt replacement playback.

Hidden/pagehide pauses and silences the same video and cancels video work.
Restoration resumes previously established playback only after live UTC alignment
(or at the remembered position in manual mode). An initially hidden scene, playback
hidden before it first starts, explicit Pause, clock safety pauses and denied autoplay
still require Play; an unsettled sound gesture is not permission to resume audio.
Duplicate visibility/BFCache notifications do not replay a second attempt.
Video Pause does **not** disconnect multiplayer
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
`invalidate_control_panels(): void`. Canvas interactions use the shared
`pick_scene_interaction`, `activate_seat_interaction` and
`invalidate_scene_interactions` exports to preserve seating when panels are absent.

World-panel IDs are 0 none, 1 sound, 2 fullscreen, 3 subtitles; states are
0 unavailable, 1 OFF, 2 pending, 3 ON, 4 retry, 5 LOADING. Sound remains clickable while loading/pending
to reopen its confirmation; fullscreen ignores repeat clicks while pending. Picking uses canvas-relative NDC [-1,1], Y up;
pointer lock picks the crosshair (0,0). Capture handlers consume mouse panel hits
before pointer lock/input and leave misses untouched. Touch sequences always reach
Winit, including drags starting on a panel, preserving the full left-half move /
right-half look split and simultaneous thumbs. Only an undragged, single-finger tap
activates on release (8 CSS pixel movement threshold, at most 500 ms). Mouse
compatibility events are suppressed after a hit, never native touch events.
Cancellation, capture/focus loss, or a hidden page cancels pending taps. Synthetic
input cannot activate panels. Shutdown removes handlers/listeners and invalidates
panels once.

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
and shutdown in offline Linux Chromium, with separate WebGL2/WebGPU runs. Its
startup, silent-continue, failed-clip retry, failed-manifest retry and lifetime
cases are separately bounded; `--web-root build/hosting` selects the prepared
release artifact rather than staged `web/`. It supplies
compatible Vulkan/ANGLE/Dawn flags for this development environment and records
GPU details rather than implying other systems are verified. Screenshots permit
visual confirmation of the in-world screen. Touch/mobile/Safari QA, actual
background-tab policy, audible output/AAC browser support, and real A/V timing
remain unverified by these Node tests; lifecycle
stimuli in the smoke are synthetic. No deployed validation is implied.

### Minimal browser controller HUD

`controller-hud.ts` owns the minimal-only DOM presentation. On the first combined
WASM/renderer-ready and visible scene, the expanded controller opens exactly once,
even if the manifest or movie is slow or failed. An initially hidden scene waits
until first visibility. Dismissal, tab restoration, BFCache restoration and duplicate
ready callbacks do not reopen it. Host chat ownership defers initial presentation;
hidden, fatal or disposed scenes never acquire focus. Minimal mode does not construct
`TheaterStartup`; non-minimal diagnostics retain that optional legacy flow.

After dismissal, collapsed artwork stays bottom-right; **Q**, click or a mobile tap
opens the expanded artwork instantly in
the same corner, over a viewport-wide **70% black** backdrop. The only three large
sprite buttons are **Audio**, **Subtitles** and **Fullscreen**. Images are preloaded
at initialization; interaction does not wait for image loading/decoding. A completed
outside click/tap, fresh **Q**, or **Escape** dismisses it. Q key repeat is ignored,
and Q is not intercepted while editing chat or composing text.

Opening clears held keys, releases pointer lock, traps focus and suppresses
gameplay input only; media, simulation and networking keep their own lifetimes.
Outside input is consumed rather than forwarded to the canvas. Startup dismissal
by Q, Escape or a completed backdrop gesture restores canvas keyboard focus only:
no pointer lock, fullscreen or audio is requested. After manual reopening, explicit
dismissal may restore prior desktop mouse lock best-effort, but blur/cleanup never
captures or restores focus.
The host reports controller-open state to Flutter, hiding its mobile chat entry
while expanded. Focus-only chat close restores iframe keyboard focus so Q works
without mouse capture. Fullscreen uses observed browser/parent-bridge state;
Audio's sprite reflects desired `state.audioEnabled`, with separate pending,
blocked and actual audibility status. Audio starts OFF. Selecting Audio while media
is preparing never queues a later unmute; a failed source/manifest can be retried
silently, followed by a fresh ready press for sound. Pending enabling attempts can
still be switched OFF, and browser-policy rejection does not replay audible intent.
All Audio clicks require `event.isTrusted`, including enabling, muting/cancellation
and retry: synthetic `.click()` or dispatched events cannot change sound or start
retry downloads. Failures remain visible even with desired Audio ON; the status
instructs the user to turn OFF, then retry/turn ON with a fresh press when ready.
Pending ON status explicitly offers OFF to cancel sound.

All eight PNGs are site-owned at `/weaver/assets/controller/`, not embedded in
WASM or supplied by this standalone engine checkout. The site asset README at
`web/weaver/assets/controller/README.md` records provenance, the shortened
498 × 424 x3 base and the current three-row layout. The original source manifest's
old placements and removed Volume row are not the runtime layout.

### Subtitles

The site theater manifest supplies actual `.vtt` tracks through per-entry
`captions` references. `theater-subtitles.ts` presents timed cue text synchronized
with the media clock and source changes: browser-native captions on the hidden
video are not included in the texture uploaded to the theater screen.
Subtitles are **ON by default**, honor stored preference, and remain independently
toggleable even when audio is muted or volume is zero. Audio-off does not force
captions on, and entries without supplied tracks do not fabricate text.

## UTC theater schedule

`theater-schedule-controller.ts` exports `loadTheaterSchedule`,
`DEFAULT_THEATER_SCHEDULE`, and `TheaterScheduleController`. It consumes the
shared `validateTheaterSchedule` / `resolveTheaterSchedule` model without changing
the required timing fields (entries may also reference supplied caption tracks).
`TheaterVideoPlayer.scheduledAutoplay()` is the small readiness/busy seam; the existing player still owns the only decoder and pending play promise.

`meta[name="weaver-theater-schedule"]` defaults to `/weaver/theater-schedule.json`.
The manifest is fetched **once** per shell instance, with a 32 KiB streaming bound
and a 10-second deadline covering request and body. Only a canonical same-origin
root-absolute path is accepted: no credentials, queries, fragments, percent aliases,
or dot segments. Requests omit credentials/referrers, use `cache: "no-store"`,
reject redirects, and require JSON. Failure is isolated from scene/network startup
and visibly reports **Schedule unavailable**, without claiming synchronization or
silently falling back. **Continue without sound** explicitly selects a local silent
fallback once after manifest failure; manifest Retry remains available. Healthy
scheduling does not preload/download that fallback. Non-minimal diagnostics can
also uncheck **Follow schedule** to use the movie meta/default manually.

The parent site supplies/stages the manifest; this source checkout does not create
or deploy it. A single-movie diagnostic manifest looks like this (the site's actual
manifest supplies its portfolio reel and caption references instead):

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
pauses and aligns before any new automatic attempt, unless the user paused,
autoplay was denied, or a startup/clock safety block still requires Play. Manual Play computes/seeks the live slot and calls the
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
source/offset and position, including crossed slots and cycle boundaries, before
resuming previously established playback through `scheduledAutoplay()`. Automatic
play waits for asynchronous seek completion and preserves mute/volume preferences.
Manual pause, startup gating, autoplay denial and invalid-clock safety pauses are
not cleared by restoration. Network restoration still requires a manual join.

The player watchdog owns unexpected pause/stall detection and bounded soft retries
and decoder reload, not a second controller polling/retry loop. After an internal
reload restores metadata and the remembered decoder time, its `onRecover` callback
calls `TheaterScheduleController.recoverPlayback()`. This forces fresh UTC alignment
and authorizes eligible playback through `scheduledAutoplay()`, never gesture Play
or Enable sound. Manual mode and the explicit silent fallback retain the restored
media position instead of seeking to UTC. Recovery cannot restart a held final frame,
failed/blocked media, a hidden/disposed player, or manually paused playback.
Fatal/route disposal aborts the manifest request and cancels the controller before
the player; stale callbacks cannot revive either. No hosted/Woven/time service is
used by this integration. Device clock error, seek/decoder precision, buffering,
and browser autoplay policy limit alignment; these tests do not verify audible
output or sample-accurate synchronization. Parent browser QA/staging tooling must
be updated separately for the new meta and schedule-controlled controls.

Offline tests include fake clocks/timers/media, two-client selection, strict drift
threshold, asynchronous alignment-before-muted-autoplay, source/offset transitions, cycle
wrap, established-playback resume, duplicate lifecycle events, decoder recovery,
manual pause/startup/clock/denial gates, disposal, loop/short-source behavior, and
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

The browser fails startup after 20 seconds without download or initialization
activity and aborts the pending WASM fetch. Advancing response chunks and new
initialization milestones reset this idle watchdog, so a healthy slow download
can exceed 20 seconds. The Rust web platform supplies the ready/fatal hooks;
First-Person's panic hook reports fatal failures immediately. Regenerate the
web release after changing either side.

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
bounded-inbox overflow remain connection failures. Poses use positioned channel-4
datagrams: legacy v1/25 bytes for standing, v2/26 bytes for seating. V2 byte 1 is
0 for standing or 1–24 for an authored seat; six little-endian f32 values begin at
byte 2. Rust validates seated positions against exact seat anchors and rejects
invalid directions. The browser validates version/length/seat-code combinations.
Older clients cannot display seated updates; reload the updated scene. No delivery
fallback or channel-policy override is introduced. Local chat echo is not a peer delivery acknowledgement.

## Local validation and remaining gates

```sh
npm run typecheck
npm test
```

Tests use Node, synthetic data, mocked browser/Woven/WASM seams, and in-memory
esbuild bundles. They do not read generated WASM or contact any endpoint.

The parent site's bounded offline controller check runs from its project root
with Node 22+, Chromium and already-staged artifacts:

```sh
node tool/controller_hud_smoke.mjs --browser /usr/bin/google-chrome --mode both
```

Default `--mode standalone` serves `web/`; `parent` serves the existing `build/web`,
and `both` checks both within one 145-second deadline. The runner does not build
or stage. It owns/stops its servers and headless browser, blocks external and
API/bootstrap/credential routes, and disables WebTransport/WebGPU before scripts.
Reports, screenshots and artifact fingerprints go to `.dart_tool/controller-qa/`.
It checks real WASM/WebGL camera uploads, Q repeat behavior, input pause/recovery,
bottom-right fit, completed backdrop dismissal, direct Audio with no second
startup, muted independent subtitles with real cue text, and advertised fullscreen.
Short landscape/touch/coarse-pointer checks are CDP emulation, not physical-device
QA; safe-area emulation is not notch certification. Audio rejection is permitted
only with muted movie recovery. This does not validate Safari, WebGPU, physical
speakers, live Woven/cloud, deployment, or unstaged changes; parent mode checks
whatever already exists in `build/web`.

Before public multiplayer release, supply an approved bootstrap service/target
configuration and verify real networking/browser behavior. See the
[portfolio release runbook](../../../../docs/PORTFOLIO-RELEASE.md) for static
packaging, Firebase Hosting checks, public-token risks, and activation gates.
Manual browser QA is still required for two-peer late join/chat/rename/leave,
room-corner spatial interest, pointer-lock/focus/IME, small-screen/DPI overlays,
and hidden-tab/BFCache/sleep restoration. No hosted validation is implied.
