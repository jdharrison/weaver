import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { TheaterVideoPlayer } from "./theater-video-player.ts";
const bundle = await build({ entryPoints: ["theater-schedule-controller.ts"], bundle: true, write: false, format: "esm", platform: "browser" });
const { TheaterScheduleController, loadTheaterSchedule } = await import("data:text/javascript;base64," + Buffer.from(bundle.outputFiles[0].text).toString("base64"));
const flush = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
const entry = (id, start, duration, source = "/a.mp4", offset = 0, loop = false) => ({ id, title: id, startSeconds: start, durationSeconds: duration, source, sourceStartSeconds: offset, loop });
const schedule = (entries = [entry("a", 0, 3600)], cycle = 3600) => ({ version: 1, revision: "r1", anchorUnixSeconds: 0, cycleSeconds: cycle, driftToleranceSeconds: 5, entries });
const response = (data) => new Response(JSON.stringify(data), { headers: { "content-type": "application/json" } });
class Media extends EventTarget {
  src = ""; muted = false; volume = 1; loop = true; currentTime = 0; duration = 4000;
  readyState = 0; paused = true; seeking = false; ended = false; error = null;
  loads = 0; plays = []; blocked = false; asyncSeek = false;
  load() { this.loads++; this.readyState = 0; }
  removeAttribute() { this.src = ""; }
  pause() { this.paused = true; this.dispatchEvent(new Event("pause")); }
  play() { this.plays.push(this.currentTime); if (this.blocked) return Promise.reject(new Error("blocked")); this.paused = false; this.dispatchEvent(new Event("playing")); return Promise.resolve(); }
  event(type) { this.dispatchEvent(new Event(type)); }
}
async function fixture(data = schedule(), now = 123, blocked = false, onScheduleLoaded, waitForGesture = false) {
  const media = new Media(); media.blocked = blocked;
  const timers = new Map(); let timerId = 0, controller; const seeks = []; const statuses = [];
  const player = new TheaterVideoPlayer({ media, scheduled: true, autoplay: true,
    attach: () => true, markFrame: () => true, detach: () => {}, requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => {}, onChange: () => controller?.playerChanged(),
  });
  const seek = player.seek.bind(player);
  player.seek = (value) => { seeks.push(value); seek(value); if (media.asyncSeek) media.seeking = true; };
  player.setSceneReady(true);
  controller = new TheaterScheduleController({ player, media, path: "/schedule.json", origin: "https://local.test", localSource: "/local.mp4",
    fetcher: async () => response(data), now: () => now,
    setTimeout: (callback) => { timers.set(++timerId, callback); return timerId; }, clearTimeout: (id) => timers.delete(id),
    onChange: (status) => statuses.push(status), onScheduleLoaded, waitForGesture,
  });
  await controller.start();
  const ready = () => { media.readyState = 2; media.event("loadedmetadata"); media.event("canplay"); };
  const tick = (time) => { now = time; const callbacks = [...timers.values()]; timers.clear(); callbacks.forEach((callback) => callback()); };
  const dispose = () => { controller.dispose(); player.dispose(); };
  return { media, player, controller, timers, seeks, statuses, ready, tick, dispose, setNow: (time) => { now = time; } };
}

async function watchdogFixture(data = schedule(), initialUtc = 123) {
  const media = new Media(), frames = new Map(), timers = new Map();
  const seeks = [], loads = [], starts = [], recoveries = [], attaches = [], statuses = [];
  let now = 0, utcOffset = initialUtc, serial = 0, controller, playPromise = null;
  const timerOptions = (owner) => ({
    setTimeout: (callback, milliseconds) => { timers.set(++serial, { callback, due: now + milliseconds, owner }); return serial; },
    clearTimeout: (id) => timers.delete(id),
  });
  media.requestVideoFrameCallback = (callback) => { frames.set(++serial, callback); return serial; };
  media.cancelVideoFrameCallback = (id) => frames.delete(id);
  const load = media.load.bind(media), play = media.play.bind(media);
  media.load = () => {
    loads.push({ at: now, source: media.src }); load();
    media.currentTime = 0; media.paused = true; media.seeking = false;
  };
  media.play = () => { starts.push({ at: now, source: media.src, time: media.currentTime }); const result = play(); return playPromise ?? result; };
  const player = new TheaterVideoPlayer({ media, scheduled: true, now: () => now, ...timerOptions("player"),
    attach: () => { attaches.push({ source: media.src, readyState: media.readyState }); return media.readyState >= 1; },
    markFrame: () => true, detach() {}, requestAnimationFrame: () => assert.fail("decoded callback path expected"), cancelAnimationFrame() {},
    onChange: () => controller?.playerChanged(),
    onRecover: () => { recoveries.push({ source: media.src, time: media.currentTime, ready: player.state.ready }); controller.recoverPlayback(); },
  });
  const seek = player.seek.bind(player);
  player.seek = (time) => {
    seeks.push({ at: now, time }); seek(time);
    if (media.asyncSeek) media.seeking = true;
    else media.event("seeked");
  };
  player.setSceneReady(true);
  controller = new TheaterScheduleController({ player, media, path: "/schedule.json", origin: "https://local.test",
    fetcher: async () => response(data), now: () => utcOffset + now / 1000, ...timerOptions("schedule"),
    onChange: (status) => statuses.push(status),
  });
  await controller.start();
  const metadata = () => { media.readyState = 1; media.event("loadedmetadata"); };
  const ready = async () => { metadata(); media.readyState = 2; media.event("loadeddata"); await flush(); };
  const advance = async (milliseconds) => {
    const end = now + milliseconds;
    while (timers.size) {
      const [id, timer] = [...timers].sort((a, b) => a[1].due - b[1].due)[0];
      if (timer.due > end) break;
      now = timer.due; timers.delete(id); timer.callback(); await flush();
    }
    now = end;
  };
  return { media, player, controller, frames, timers, seeks, loads, starts, recoveries, attaches, statuses, metadata, ready, advance,
    setUtc: (time) => { utcOffset = time - now / 1000; }, setPlayPromise: (promise) => { playPromise = promise; },
    hide: () => { controller.setVisible(false); player.setVisible(false); },
    restore: () => { player.setVisible(true); controller.setVisible(true); },
    dispose: () => { controller.dispose(); player.dispose(); },
  };
}

test("real watchdog reload and metadata recovery realign UTC despite ongoing schedule seeks and missing decoded callbacks", async (t) => {
  const f = await watchdogFixture(); t.after(f.dispose); await f.ready();
  assert.equal(f.frames.size, 1);
  await f.advance(7000); assert.equal(f.starts.length, 1);
  await f.advance(1000); assert.equal(f.starts.length, 2); assert.equal(f.loads.length, 1);
  await f.advance(8000);
  assert.ok(f.seeks.length >= 3, "the schedule kept polling and seeking the advancing UTC clock");
  assert.equal(f.loads.length, 2); assert.equal(f.player.state.ready, false); assert.equal(f.frames.size, 0);
  assert.equal(f.recoveries.length, 0); assert.equal(f.media.currentTime, 0);
  f.player.play = f.player.enableSound = () => assert.fail("recovery must remain controller-authorized autoplay");
  f.setUtc(140); f.metadata(); await flush();
  assert.deepEqual(f.recoveries, [{ source: "/a.mp4", time: 135, ready: true }]);
  assert.deepEqual(f.starts.map(({ time }) => time), [123, 129, 140]);
  assert.equal(f.media.muted, true); assert.equal(f.player.state.playing, true); assert.equal(f.frames.size, 1);
  await f.advance(8000);
  assert.equal(f.player.state.failed, true, "polling seeks must not replenish the watchdog recovery budget");
  const starts = f.starts.length, loads = f.loads.length;
  await f.advance(5000); assert.equal(f.starts.length, starts); assert.equal(f.loads.length, loads);
});

for (const [utc, source, desired] of [[31, "/a.mp4", 101], [61, "/c.mp4", 201], [91, "/a.mp4", 11]]) {
  test(`real watchdog metadata recovery selects the latest UTC source/offset at ${utc}s`, async (t) => {
    const data = schedule([entry("a", 0, 30, "/a.mp4", 10), entry("b", 30, 30, "/a.mp4", 100), entry("c", 60, 30, "/c.mp4", 200)], 90);
    const f = await watchdogFixture(data, 1); t.after(f.dispose); await f.ready(); await f.advance(16_000);
    assert.equal(f.player.state.ready, false); assert.equal(f.starts.length, 2);
    // The UTC boundary arrives before another schedule tick: the metadata callback must select it.
    f.setUtc(utc); f.metadata(); await flush();
    assert.equal(f.recoveries.length, 1);
    if (source !== "/a.mp4") {
      assert.equal(f.attaches.length, 1, "old metadata must not authorize attachment of a replacement source");
      assert.equal(f.media.readyState, 0); assert.equal(f.starts.length, 2);
      f.metadata(); await flush();
    }
    assert.equal(f.media.src, source); assert.equal(f.player.state.failed, false); assert.equal(f.starts.length, 3);
    assert.deepEqual(f.starts.at(-1), { at: 16_000, source, time: desired });
    assert.equal(f.media.currentTime, desired);
  });
}

for (const cancellation of ["pause", "hidden", "dispose"]) {
  test(`real watchdog reload pending metadata is cancelled by ${cancellation}`, async (t) => {
    const f = await watchdogFixture(); t.after(f.dispose); await f.ready(); await f.advance(16_000);
    assert.equal(f.player.state.ready, false); assert.equal(f.loads.length, 2);
    const stale = [...f.timers.values()].filter(({ owner }) => owner === "player").map(({ callback }) => callback);
    if (cancellation === "pause") f.controller.pause();
    else if (cancellation === "hidden") f.hide();
    else f.dispose();
    const loads = f.loads.length;
    f.metadata(); await f.advance(30_000); stale.forEach((callback) => callback()); await flush();
    assert.equal(f.starts.length, 2); assert.equal(f.loads.length, loads);
    assert.equal(f.recoveries.length, 0); assert.equal(f.media.paused, true);
    if (cancellation === "hidden") {
      f.setUtc(201); f.restore(); await flush();
      assert.equal(f.starts.length, 3); assert.equal(f.starts.at(-1).time, 201);
      assert.equal(f.recoveries.length, 0, "restoration cannot revive the cancelled reload callback");
    }
  });
}

for (const audible of [false, true]) {
  test(`initial ${audible ? "audible intent is not replayed" : "muted autoplay recovers"} after the real watchdog play timeout`, async (t) => {
    const f = await watchdogFixture(); t.after(f.dispose);
    let settle;
    const pending = new Promise((resolve) => { settle = resolve; });
    f.setPlayPromise(pending);
    if (audible) f.player.setMuted(false);
    await f.ready(); await f.advance(14_000);
    assert.equal(f.starts.length, 1); assert.equal(f.loads.length, 1);
    await f.advance(1000);
    assert.equal(f.loads.length, 2); assert.equal(f.player.state.ready, false);
    assert.equal(f.player.state.soundPending, false);
    f.setPlayPromise(null); f.setUtc(143); f.metadata(); await flush();
    assert.equal(f.recoveries.length, 1);
    if (audible) { assert.equal(f.starts.length, 1); assert.equal(f.media.paused, true); }
    else { assert.equal(f.starts.length, 2); assert.equal(f.starts.at(-1).time, 143); assert.equal(f.media.paused, false); }
    settle(); await flush();
    assert.equal(f.media.paused, audible); assert.equal(f.player.state.autoplayBlocked, false);
  });
}

test("returning before cancelled reload metadata remains unready until metadata permits UTC-aligned playback", async (t) => {
  const f = await watchdogFixture(); t.after(f.dispose); await f.ready(); await f.advance(16_000);
  f.hide(); f.setUtc(141); f.restore(); await flush();
  assert.equal(f.player.state.ready, false); assert.equal(f.starts.length, 2);
  f.metadata(); await flush();
  assert.equal(f.starts.length, 3); assert.equal(f.starts.at(-1).time, 141);
  assert.equal(f.recoveries.length, 0); assert.equal(f.player.state.playing, true);
});

test("construction is inert and explicit start fetches once without a fallback request", async () => {
  const sources = [], timers = new Map(); let fetches = 0;
  const media = new Media();
  const controller = new TheaterScheduleController({
    player: { state: { ready: false, duration: 0 }, setSource(source) { sources.push(source); media.src = source; }, pause() {} },
    media, path: "/schedule.json", origin: "https://local.test",
    fetcher: async () => { fetches++; return response(schedule()); },
    now: () => 123, setTimeout: (callback) => { timers.set(1, callback); return 1; }, clearTimeout: (id) => timers.delete(id),
    onChange() {},
  });
  controller.setFollowing(false); controller.setFollowing(true);
  assert.equal(fetches, 0); assert.deepEqual(sources, []); assert.equal(timers.size, 0);
  const loaded = controller.start(); assert.equal(controller.start(), loaded);
  await loaded;
  assert.equal(controller.start(), loaded);
  assert.equal(fetches, 1); assert.deepEqual(sources, ["/a.mp4"]); assert.equal(timers.size, 1);
  controller.dispose();
});

test("disposal before start prevents manifest, fallback and scheduled work", async () => {
  let fetches = 0, sources = 0;
  const controller = new TheaterScheduleController({
    player: { setSource() { sources++; } }, media: { src: "" }, path: "/schedule.json", origin: "https://local.test",
    fetcher: async () => { fetches++; return response(schedule()); }, onChange() {},
  });
  controller.setFollowing(false); controller.dispose();
  await controller.start(); await controller.start(); controller.setVisible(true); controller.setFollowing(false);
  assert.equal(fetches, 0); assert.equal(sources, 0);
});

test("local diagnostics after media failure reselect the scheduled source within the same UTC slot", async () => {
  const f = await fixture(); f.ready(); await flush();
  f.media.error = { code: 4 }; f.media.event("error");
  assert.equal(f.media.src, "");
  f.controller.setFollowing(false); assert.equal(f.media.src, "/local.mp4");
  f.ready(); f.player.play(); await flush();
  f.setNow(127); f.controller.setFollowing(true);
  assert.equal(f.media.src, "/a.mp4");
  f.ready(); await flush(); assert.equal(f.media.currentTime, 127);
  f.dispose();
});

test("startup prepares only the live clip and waits for a synchronous sound gesture, even across UTC boundaries", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 1, false, undefined, true);
  assert.equal(f.media.src, "/a.mp4");
  f.ready(); await flush();
  assert.equal(f.media.currentTime, 1); assert.equal(f.media.plays.length, 0);
  f.tick(11); f.ready(); await flush();
  assert.equal(f.media.src, "/b.mp4"); assert.equal(f.media.plays.length, 0);
  f.setNow(12); f.controller.play(true);
  assert.deepEqual(f.media.plays, [2]); assert.equal(f.media.muted, false);
  await flush(); f.tick(21); f.ready(); await flush();
  assert.deepEqual(f.media.plays, [2, 1]); // Consent survives scheduled clip changes.
  f.dispose();
});

test("a boundary during the initial Play prepares the replacement but never replays audio intent asynchronously", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 9, false, undefined, true);
  f.ready(); f.setNow(11); f.controller.play(true);
  assert.equal(f.media.src, "/b.mp4"); assert.equal(f.media.plays.length, 0);
  f.ready(); await flush(); f.tick(12);
  assert.equal(f.media.plays.length, 0); assert.equal(f.media.muted, true);
  f.controller.play(true); assert.deepEqual(f.media.plays, [2]);
  await flush(); f.dispose();
});

test("Continue muted releases initial preparation and established playback resumes after visibility alignment", async (t) => {
  const f = await fixture(schedule(), 123, false, undefined, true); t.after(f.dispose);
  f.controller.continueMuted(); assert.equal(f.media.plays.length, 0);
  f.ready(); await flush(); assert.deepEqual(f.media.plays, [123]); assert.equal(f.media.muted, true);
  f.controller.setVisible(false); f.player.setVisible(false);
  assert.equal(f.media.paused, true); assert.equal(f.timers.size, 0);
  f.setNow(124); f.player.setVisible(true); f.controller.setVisible(true); await flush();
  assert.deepEqual(f.media.plays, [123, 124]); assert.equal(f.media.muted, true);
  f.tick(125); await flush(); assert.equal(f.media.plays.length, 2);
});

test("duplicate visibility events resume once and asynchronous restore alignment completes before playback", async (t) => {
  const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
  const loads = f.media.loads;
  f.player.setVolume(0.25);
  f.player.play = f.player.enableSound = () => assert.fail("automatic resume must not use gesture methods");
  f.controller.setVisible(false); f.player.setVisible(false);
  f.controller.setVisible(false); f.player.setVisible(false);
  f.tick(124); f.controller.recoverPlayback(); assert.equal(f.timers.size, 0);
  f.media.asyncSeek = true; f.setNow(125);
  f.player.setVisible(true); f.controller.setVisible(true);
  assert.equal(f.media.currentTime, 125); assert.equal(f.media.plays.length, 1);
  assert.equal(f.media.seeking, true); assert.equal(f.timers.size, 1);
  f.controller.setVisible(true); assert.equal(f.timers.size, 1);
  f.media.seeking = false; f.media.event("seeked"); await flush();
  assert.deepEqual(f.media.plays, [123, 125]); assert.equal(f.media.volume, 0.25);
  assert.equal(f.media.muted, true); assert.equal(f.media.loads, loads);
  f.controller.setVisible(true); f.tick(126); await flush(); assert.equal(f.media.plays.length, 2);
});

test("cancelled pre-hide callbacks cannot replace the restored schedule polling timer or revive disposal", async (t) => {
  const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
  const stale = [...f.timers.values()][0];
  f.controller.setVisible(false); f.player.setVisible(false); f.setNow(125);
  f.player.setVisible(true); f.controller.setVisible(true); await flush();
  const statuses = f.statuses.length;
  stale(); assert.equal(f.timers.size, 1); assert.equal(f.statuses.length, statuses);
  f.tick(126); assert.equal(f.timers.size, 1); assert.equal(f.media.plays.length, 2);
  f.dispose(); stale(); f.controller.recoverPlayback();
  assert.equal(f.timers.size, 0); assert.equal(f.media.plays.length, 2);
});

test("audible visibility resume preserves preference and uses the bounded muted fallback if policy changes", async (t) => {
  const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
  f.controller.play(true); await flush(); f.player.setVolume(0.3);
  f.controller.setVisible(false); f.player.setVisible(false);
  const attempts = [], play = f.media.play.bind(f.media);
  f.media.play = () => {
    attempts.push(f.media.muted);
    if (!f.media.muted) return Promise.reject(new DOMException("denied", "NotAllowedError"));
    return play();
  };
  f.player.play = f.player.enableSound = () => assert.fail("restoration must not manufacture a sound gesture");
  f.setNow(130); f.player.setVisible(true); f.controller.setVisible(true); await flush();
  assert.deepEqual(attempts, [false, true]); assert.equal(f.media.paused, false);
  assert.equal(f.media.currentTime, 130); assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.3);
  assert.equal(f.player.state.audioEnabled, true); assert.equal(f.player.state.autoplayBlocked, false);
  f.tick(131); await flush(); assert.deepEqual(attempts, [false, true]);
});

test("restore follows live offsets, replacement sources and cycle boundaries without replaying retired slots", async (t) => {
  const data = schedule([entry("a", 0, 10), entry("b", 10, 10, "/a.mp4", 50), entry("c", 20, 10, "/c.mp4", 7)], 30);
  for (const [time, source, desired] of [[11, "/a.mp4", 51], [21, "/c.mp4", 8], [31, "/a.mp4", 1]]) {
    const f = await fixture(data, 9); t.after(f.dispose); f.ready(); await flush();
    const loads = f.media.loads;
    f.controller.setVisible(false); f.player.setVisible(false); f.setNow(time);
    f.player.setVisible(true); f.controller.setVisible(true);
    assert.equal(f.media.src, source);
    if (source !== "/a.mp4") { assert.equal(f.media.plays.length, 1); f.ready(); }
    else assert.equal(f.media.loads, loads);
    await flush(); assert.deepEqual(f.media.plays, [9, desired]); assert.equal(f.media.currentTime, desired);
    assert.equal(f.timers.size, 1);
  }
});

test("unsettled audible intent is not permission to resume after visibility restoration", async (t) => {
  const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
  let resolve;
  const play = f.media.play.bind(f.media);
  f.media.play = () => { f.media.plays.push(f.media.currentTime); return new Promise((yes) => { resolve = yes; }); };
  f.controller.play(true); assert.equal(f.player.state.soundPending, true);
  f.controller.setVisible(false); f.player.setVisible(false);
  resolve(); await flush(); f.media.play = play;
  f.setNow(130); f.player.setVisible(true); f.controller.setVisible(true);
  f.controller.recoverPlayback(); f.tick(131); await flush();
  assert.equal(f.media.plays.length, 2); assert.equal(f.media.paused, true);
});

test("blocked sound keeps muted schedule playing across source changes without replaying audible intent", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 1);
  f.ready(); await flush();
  assert.equal(f.player.state.playing, true);
  assert.equal(f.media.muted, true);
  const play = f.media.play.bind(f.media);
  const attempts = [];
  let gesture = false;
  f.media.play = () => {
    attempts.push(f.media.muted);
    if (!f.media.muted) {
      assert.equal(gesture, true);
      return Promise.reject(new DOMException("denied", "NotAllowedError"));
    }
    return play();
  };
  gesture = true; f.controller.play(true); gesture = false;
  await flush();
  assert.deepEqual(attempts, [false, true]);
  assert.equal(f.player.state.playing, true);
  assert.equal(f.player.state.autoplayBlocked, false);
  f.controller.continueMuted(); await flush();
  assert.equal(f.player.state.playing, true);
  f.tick(11); f.ready(); await flush();
  assert.equal(f.media.src, "/b.mp4");
  assert.equal(f.media.currentTime, 1);
  assert.equal(f.player.state.playing, true);
  assert.equal(f.media.muted, true);
  assert.deepEqual(attempts, [false, true, true]);
  f.dispose();
});

test("failed current clip stays failed during polling; explicit Retry reloads the same slot once and requires Play", async () => {
  const f = await fixture(schedule(), 123, false, undefined, true);
  f.ready(); f.controller.play(true); await flush();
  f.media.error = { code: 2 }; f.media.event("error");
  assert.equal(f.player.state.failed, true); assert.equal(f.media.src, "");
  const loads = f.media.loads;
  f.tick(124); f.tick(125); f.controller.play(true);
  assert.equal(f.media.loads, loads); assert.equal(f.media.plays.length, 1);
  f.media.error = null; f.controller.retry(); f.controller.retry();
  assert.equal(f.media.src, "/a.mp4"); assert.equal(f.media.loads, loads + 1);
  assert.equal(f.player.state.failed, false);
  f.ready(); await flush(); assert.equal(f.media.plays.length, 1);
  f.setNow(126); f.controller.play(true); assert.deepEqual(f.media.plays, [123, 126]);
  await flush(); f.dispose();
});

test("ordinary Retry is inert during metadata load; explicit slow retry reloads only the selected clip", async () => {
  const f = await fixture(schedule(), 123, false, undefined, true);
  const loads = f.media.loads;
  f.controller.retry(); assert.equal(f.media.loads, loads);
  f.controller.retry(true); assert.equal(f.media.src, "/a.mp4"); assert.equal(f.media.loads, loads + 2);
  f.ready(); await flush(); assert.equal(f.media.plays.length, 0);
  f.controller.setVisible(false); f.controller.retry(true); assert.equal(f.media.loads, loads + 2);
  f.dispose(); const disposedLoads = f.media.loads;
  f.controller.retry(true); f.controller.continueMuted(); assert.equal(f.media.loads, disposedLoads);
});

test("manifest failure is structured, retry deduplicates fetches, and the recovered source still waits for confirmation", async () => {
  const media = new Media(); let fetches = 0, controller;
  const player = new TheaterVideoPlayer({ media, scheduled: true, attach: () => true, markFrame: () => true,
    detach() {}, requestAnimationFrame: () => 1, cancelAnimationFrame() {}, onChange: () => controller?.playerChanged() });
  player.setSceneReady(true);
  controller = new TheaterScheduleController({ player, media, path: "/schedule.json", origin: "https://local.test",
    waitForGesture: true, now: () => 123, setTimeout: () => 1, clearTimeout() {}, onChange() {},
    fetcher: async () => { fetches++; if (fetches === 1) throw new Error("offline"); return response(schedule()); } });
  const initial = controller.start();
  assert.deepEqual(controller.state, { loading: true, failed: false });
  controller.retry(); assert.equal(fetches, 1);
  await initial;
  assert.deepEqual(controller.state, { loading: false, failed: true }); assert.equal(media.src, "");
  controller.retry(); controller.retry(); await controller.start();
  assert.equal(fetches, 2); assert.deepEqual(controller.state, { loading: false, failed: false });
  assert.equal(media.src, "/a.mp4"); media.readyState = 2; media.event("loadedmetadata"); media.event("canplay"); await flush();
  assert.equal(media.plays.length, 0);
  controller.play(true); assert.deepEqual(media.plays, [123]); assert.equal(media.muted, false);
  await flush(); controller.dispose(); player.dispose();
});

for (const continueDuringLoad of [false, true]) {
  test(`explicit silent fallback after manifest failure is one-shot and schedule retry recovers (${continueDuringLoad ? "loading" : "failed"})`, async () => {
    const media = new Media(); let controller, fetches = 0, reject;
    const offline = new Promise((_resolve, no) => { reject = no; });
    const statuses = [];
    const player = new TheaterVideoPlayer({ media, scheduled: true, attach: () => true, markFrame: () => true,
      detach() {}, requestAnimationFrame: () => 1, cancelAnimationFrame() {}, onChange: () => controller?.playerChanged() });
    player.setSceneReady(true);
    controller = new TheaterScheduleController({ player, media, path: "/schedule.json", origin: "https://local.test",
      localSource: "/silent-local.mp4", now: () => 123, setTimeout: () => 1, clearTimeout() {},
      onChange: (status) => statuses.push(status), fetcher: () => ++fetches === 1 ? offline : Promise.resolve(response(schedule())) });
    const initial = controller.start();
    assert.equal(media.loads, 0);
    if (continueDuringLoad) controller.continueMuted();
    reject(new Error("offline")); await initial;
    if (!continueDuringLoad) {
      assert.equal(media.loads, 0, "failure alone must not download diagnostics");
      controller.continueMuted();
    }
    assert.deepEqual(controller.state, { loading: false, failed: true });
    assert.equal(media.src, "/silent-local.mp4");
    assert.match(statuses.at(-1), /Schedule unavailable.*local silent fallback/);
    media.readyState = 2; media.event("loadedmetadata"); media.event("canplay"); await flush();
    assert.equal(player.state.playing, true);
    assert.equal(media.muted, true);
    const loads = media.loads;
    for (let i = 0; i < 3; i++) { controller.playerChanged(); media.event("timeupdate"); }
    assert.equal(media.loads, loads);
    media.error = { code: 2 }; media.event("error");
    for (let i = 0; i < 3; i++) controller.playerChanged();
    assert.equal(media.loads, loads + 1, "failure releases source but never redownloads it");
    media.error = null;
    controller.retry(); controller.retry(); await controller.start();
    assert.equal(fetches, 2);
    assert.equal(controller.state.failed, false);
    assert.equal(media.src, "/a.mp4");
    media.readyState = 2; media.event("loadedmetadata"); media.event("canplay"); await flush();
    assert.equal(player.state.playing, true);
    assert.equal(media.muted, true);
    controller.dispose(); player.dispose();
  });
}

test("world observer receives one validated frozen manifest, independently of decoder controls", async () => {
  const manifests = [];
  const f = await fixture(schedule(), 123, false, (manifest) => manifests.push(manifest));
  assert.equal(manifests.length, 1);
  assert.equal(manifests[0].anchorUnixSeconds, 0);
  assert.equal(manifests[0].cycleSeconds, 3600);
  assert.ok(Object.isFrozen(manifests[0]));
  assert.ok(Object.isFrozen(manifests[0].entries));
  f.ready(); f.controller.pause(); f.controller.setFollowing(false);
  f.controller.setVisible(false); f.controller.setVisible(true); f.tick(3600);
  assert.equal(manifests.length, 1);
  f.dispose();
  const invalid = await fixture({ ...schedule(), cycleSeconds: 0 }, 123, false, () => manifests.push("invalid"));
  assert.equal(manifests.length, 1);
  invalid.dispose();
});

test("throwing world observers do not disable theater playback or polling", async () => {
  const f = await fixture(schedule(), 123, false, () => { throw new Error("observer failure"); });
  assert.equal(f.timers.size, 1);
  f.ready(); await flush();
  assert.deepEqual(f.media.plays, [123]);
  assert.doesNotMatch(f.statuses.at(-1), /unavailable/);
  f.dispose();
});

test("Enable sound realigns and plays in the gesture; manual pause survives visibility restoration", async () => {
  const f = await fixture();
  let gesture = false;
  const play = f.media.play.bind(f.media);
  f.media.play = () => {
    assert.ok(f.media.muted || gesture, "audible autoplay denied");
    return play();
  };
  f.ready(); await flush();
  assert.deepEqual(f.media.plays, [123]); assert.equal(f.media.muted, true);
  f.player.setVolume(0.25); f.setNow(130);
  gesture = true; f.controller.play(true); gesture = false;
  assert.deepEqual(f.media.plays, [123, 130]); assert.equal(f.media.muted, false);
  assert.equal(f.media.volume, 0.25); await flush();
  f.controller.pause(); f.player.setMuted(true); f.tick(131);
  assert.equal(f.media.plays.length, 2); assert.equal(f.media.paused, true);
  f.controller.setVisible(false); f.player.setVisible(false);
  f.setNow(140); f.player.setVisible(true); f.controller.setVisible(true);
  assert.equal(f.media.plays.length, 2); assert.equal(f.media.currentTime, 140);
  f.setNow(141); gesture = true; f.controller.play(true); gesture = false;
  assert.deepEqual(f.media.plays, [123, 130, 141]);
  await flush(); f.dispose();
});

test("two clients join the same live slot, align before muted playback, and use strict >5 drift", async () => {
  const a = await fixture(), b = await fixture();
  for (const f of [a, b]) {
    assert.equal(f.media.plays.length, 0); f.ready(); await flush();
    assert.deepEqual(f.media.plays, [123]); assert.equal(f.media.loop, false);
    f.media.currentTime = 119; f.tick(124); assert.equal(f.seeks.length, 1);
    f.media.currentTime = 119; f.tick(125); assert.deepEqual(f.seeks, [123, 125]);
    f.dispose();
  }
});

test("decoded frame and UI clocks do not create a second schedule polling loop", async () => {
  const f = await fixture(); f.ready(); await flush(); const count = f.statuses.length;
  for (let i = 0; i < 100; i++) f.media.event("timeupdate");
  assert.equal(f.statuses.length, count); f.dispose();
});

test("Enable sound during asynchronous alignment invokes play before seek completion", async () => {
  const f = await fixture(); f.media.asyncSeek = true; f.ready();
  assert.equal(f.media.seeking, true); assert.equal(f.media.plays.length, 0);
  f.setNow(125); f.controller.play(true);
  assert.deepEqual(f.media.plays, [125]); assert.equal(f.media.muted, false);
  assert.equal(f.media.seeking, true); await flush();
  f.media.seeking = false; f.media.event("seeked"); await flush();
  assert.deepEqual(f.media.plays, [125]); f.dispose();
});

test("metadata seek completes before autoplay; seeking events cannot recursively align", async () => {
  const f = await fixture(); f.media.asyncSeek = true; f.ready();
  assert.equal(f.media.plays.length, 0); assert.deepEqual(f.seeks, [123]);
  f.media.seeking = false; f.media.event("seeked"); await flush();
  assert.deepEqual(f.media.plays, [123]); f.dispose();
});

test("decoder recovery forces fresh UTC alignment and waits for asynchronous seeking before scheduled autoplay", async (t) => {
  const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
  f.player.pause(); f.media.load(); f.media.currentTime = 123; f.setNow(124); f.ready();
  const loads = f.media.loads, seeks = f.seeks.length;
  assert.equal(f.media.plays.length, 1);
  f.player.play = f.player.enableSound = () => assert.fail("decoder recovery must not use gesture methods");
  f.media.asyncSeek = true; f.controller.recoverPlayback();
  assert.equal(f.media.currentTime, 124); assert.equal(f.seeks.length, seeks + 1);
  assert.equal(f.media.seeking, true); assert.equal(f.media.plays.length, 1);
  f.media.seeking = false; f.media.event("seeked"); await flush();
  assert.deepEqual(f.media.plays, [123, 124]); assert.equal(f.media.loads, loads);
  f.tick(125); f.media.event("canplay"); await flush();
  assert.equal(f.media.plays.length, 2); assert.equal(f.timers.size, 1);
});

for (const interruption of ["pause", "clock", "dispose"]) {
  test(`asynchronous decoder recovery cannot replay after ${interruption} interrupts alignment`, async (t) => {
    const f = await fixture(); t.after(f.dispose); f.ready(); await flush();
    f.player.pause(); f.media.load(); f.ready(); f.setNow(124);
    f.media.asyncSeek = true; f.controller.recoverPlayback();
    assert.equal(f.media.seeking, true); assert.equal(f.media.plays.length, 1);
    if (interruption === "pause") f.controller.pause();
    if (interruption === "dispose") f.controller.dispose();
    if (interruption === "clock") f.setNow(NaN);
    f.media.seeking = false; f.media.event("seeked"); await flush();
    f.media.asyncSeek = false; f.setNow(125); f.controller.recoverPlayback(); f.tick(125); await flush();
    assert.equal(f.media.plays.length, 1); assert.equal(f.media.paused, true);
    assert.equal(f.timers.size, interruption === "dispose" ? 0 : 1);
  });
}

test("decoder recovery selects current UTC offsets and sources instead of its remembered decoder position", async (t) => {
  const data = schedule([entry("a", 0, 10), entry("b", 10, 10, "/a.mp4", 50), entry("c", 20, 10, "/c.mp4", 7)], 30);
  for (const [time, source, desired] of [[11, "/a.mp4", 51], [21, "/c.mp4", 8], [31, "/a.mp4", 1]]) {
    const f = await fixture(data, 9); t.after(f.dispose); f.ready(); await flush();
    f.player.pause(); f.media.load(); f.media.currentTime = 9; f.setNow(time); f.ready();
    const loads = f.media.loads;
    f.controller.recoverPlayback(); assert.equal(f.media.src, source);
    if (source !== "/a.mp4") { assert.equal(f.media.plays.length, 1); f.ready(); }
    else assert.equal(f.media.loads, loads);
    await flush(); assert.deepEqual(f.media.plays, [9, desired]);
  }
});

test("manual and silent fallback recovery preserve decoder position, mute and volume across seeking and visibility", async (t) => {
  for (const fallback of [false, true]) {
    const f = await fixture(fallback ? { ...schedule(), cycleSeconds: 0 } : schedule()); t.after(f.dispose);
    if (fallback) { f.controller.continueMuted(); f.ready(); }
    else { f.controller.setFollowing(false); f.ready(); f.controller.play(); }
    await flush(); f.player.seek(7); f.player.setVolume(0.2);
    f.player.pause(); f.media.load(); f.media.currentTime = 7; f.setNow(3602); f.ready();
    const seeks = f.seeks.length, loads = f.media.loads;
    f.player.play = f.player.enableSound = () => assert.fail("local recovery must not use gesture methods");
    f.media.seeking = true; f.controller.recoverPlayback(); assert.equal(f.media.plays.length, 1);
    f.media.seeking = false; f.media.event("seeked"); await flush();
    assert.equal(f.media.plays.length, 2); assert.equal(f.media.plays.at(-1), 7);
    assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.2); assert.equal(f.media.loop, true);
    f.controller.setVisible(false); f.player.setVisible(false); f.setNow(7202);
    f.player.setVisible(true); f.controller.setVisible(true); await flush();
    assert.equal(f.media.plays.length, 3); assert.equal(f.media.plays.at(-1), 7);
    assert.equal(f.seeks.length, seeks); assert.equal(f.media.loads, loads);
    f.controller.pause(); f.controller.recoverPlayback(); f.media.event("canplay"); await flush();
    assert.equal(f.media.plays.length, 3);
  }
});

for (const gate of ["startup", "pause", "clock", "denial", "failed", "ended", "dispose"]) {
  test(`decoder recovery and visibility cannot bypass the ${gate} gate`, async (t) => {
    const f = await fixture(schedule(), 123, gate === "denial", undefined, gate === "startup"); t.after(f.dispose);
    f.ready(); await flush();
    if (gate === "pause") f.controller.pause();
    if (gate === "clock") f.tick(NaN);
    if (gate === "failed") { f.media.error = { code: 2 }; f.media.event("error"); }
    if (gate === "ended") { f.media.duration = 124; f.setNow(125); f.controller.recoverPlayback(); }
    const count = f.media.plays.length;
    f.controller.setVisible(false); f.player.setVisible(false);
    if (gate === "dispose") f.controller.dispose();
    f.media.blocked = false; f.setNow(126); f.controller.recoverPlayback();
    f.player.setVisible(true); f.controller.setVisible(true);
    f.controller.recoverPlayback(); f.tick(127); f.media.event("canplay"); await flush();
    assert.equal(f.media.plays.length, count); assert.equal(f.media.paused, true);
    assert.equal(f.timers.size, gate === "dispose" ? 0 : 1);
  });
}

test("same asset slot offsets and cycle wrap seek without reload, new source uses setSource", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/a.mp4", 50), entry("c", 20, 10, "/c.mp4", 7)], 30), 9);
  f.ready(); await flush(); const loads = f.media.loads;
  f.tick(10); assert.equal(f.media.currentTime, 50); assert.equal(f.media.loads, loads);
  f.tick(20); assert.equal(f.media.src, "/c.mp4"); assert.ok(f.media.loads > loads);
  assert.equal(f.media.paused, true); f.ready(); await flush(); assert.equal(f.media.plays.at(-1), 7);
  f.tick(30); f.ready(); await flush(); assert.equal(f.media.currentTime, 0); f.dispose();
});

test("hour rollover and clock jumps force live alignment", async () => {
  const f = await fixture(schedule(), 3599); f.ready(); await flush(); const loads = f.media.loads;
  f.tick(3600); assert.equal(f.media.currentTime, 0); assert.equal(f.media.loads, loads);
  f.tick(3601); f.media.currentTime = 1; f.tick(3599); assert.equal(f.media.currentTime, 3599);
  f.dispose();
});

test("pause survives entries, sources, hide; restore aligns but needs gesture; local mode restores seek", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 1);
  f.ready(); await flush(); f.controller.pause(); const plays = f.media.plays.length;
  f.tick(11); f.ready(); await flush(); assert.equal(f.media.plays.length, plays);
  f.controller.setVisible(false); f.player.setVisible(false); assert.equal(f.timers.size, 0);
  f.setNow(15); f.player.setVisible(true); f.controller.setVisible(true); assert.equal(f.media.currentTime, 5);
  assert.equal(f.media.plays.length, plays);
  f.setNow(16); f.controller.play(); assert.equal(f.media.plays.at(-1), 6); await flush();
  f.controller.setFollowing(false); f.player.seek(2); f.tick(17); assert.equal(f.media.currentTime, 2);
  f.controller.setFollowing(true); assert.equal(f.media.currentTime, 7);
  f.dispose(); const count = f.media.plays.length; f.tick(18); f.controller.play(); f.media.event("canplay"); assert.equal(f.media.plays.length, count);
});

test("blocked autoplay does not poll play; explicit mute and volume stay local", async () => {
  const f = await fixture(schedule(), 100, true); f.player.setMuted(true); f.player.setVolume(0.2); f.ready(); await flush();
  for (let i = 101; i < 110; i++) { f.tick(i); f.media.event("canplay"); }
  assert.equal(f.media.plays.length, 1); assert.match(f.player.state.status, /blocked/);
  assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.2);
  f.media.blocked = false; f.setNow(110); f.controller.play(); assert.equal(f.media.plays.at(-1), 110);
  await flush(); f.dispose();
});

test("denied scheduled autoplay survives same/new assets, cycle rollover and Follow toggles until gesture Play", async () => {
  const data = schedule([entry("a", 0, 10), entry("b", 10, 10, "/a.mp4", 50), entry("c", 20, 10, "/c.mp4", 7)], 30);
  const f = await fixture(data, 1, true); f.ready(); await flush();
  assert.equal(f.player.state.autoplayBlocked, true); assert.equal(f.media.plays.length, 1);
  // Simulate unrelated browser activation granting audio permission. It must not
  // resurrect the denied attempt through any scheduler boundary or ready event.
  f.media.blocked = false; const initialLoads = f.media.loads;
  f.tick(10); await flush(); assert.equal(f.media.currentTime, 50); assert.equal(f.media.loads, initialLoads);
  f.tick(20); f.ready(); await flush(); assert.equal(f.media.src, "/c.mp4");
  f.tick(30); f.ready(); await flush(); assert.equal(f.media.currentTime, 0);
  f.controller.setFollowing(false); f.controller.setFollowing(true); f.media.event("canplay"); await flush();
  assert.equal(f.media.plays.length, 1); assert.equal(f.media.paused, true);
  assert.equal(f.player.state.autoplayBlocked, true);
  f.setNow(31); f.controller.play(); // No flush: seek and play must happen in this gesture.
  assert.equal(f.media.plays.length, 2); assert.equal(f.media.plays.at(-1), 1);
  assert.equal(f.player.state.autoplayBlocked, false);
  await flush(); f.tick(50); f.ready(); await flush(); assert.equal(f.media.plays.length, 3);
  f.dispose();
});

test("denial settling after a source transition still prevents future scheduled attempts", async () => {
  const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 1);
  let reject;
  const pending = new Promise((_resolve, no) => { reject = no; });
  const originalPlay = f.media.play;
  f.media.play = () => { f.media.plays.push(f.media.currentTime); return pending; };
  f.ready(); assert.equal(f.media.plays.length, 1);
  f.tick(11); f.ready(); assert.equal(f.media.plays.length, 1);
  reject(new DOMException("Autoplay denied", "NotAllowedError")); await flush();
  assert.equal(f.player.state.autoplayBlocked, true);
  f.media.play = originalPlay;
  for (const time of [12, 21, 31]) { f.tick(time); f.ready(); await flush(); }
  assert.equal(f.media.plays.length, 1); assert.equal(f.media.paused, true);
  f.setNow(32); f.controller.play(); assert.equal(f.media.plays.length, 2); assert.equal(f.media.plays.at(-1), 2);
  await flush(); f.dispose();
});

test("source-reset AbortError and unknown stale errors allow replacement schedule autoplay", async () => {
  for (const reason of [new DOMException("Play interrupted by source reset", "AbortError"), new Error("Old play cancelled")]) {
    const f = await fixture(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20), 1);
    let reject;
    const pending = new Promise((_resolve, no) => { reject = no; });
    const originalPlay = f.media.play;
    f.media.play = () => { f.media.plays.push(f.media.currentTime); return pending; };
    f.ready(); assert.equal(f.media.plays.length, 1);
    f.tick(11); f.ready(); assert.equal(f.media.plays.length, 1); assert.equal(f.media.src, "/b.mp4");
    f.media.play = originalPlay; reject(reason); await flush();
    assert.equal(f.player.state.autoplayBlocked, false);
    f.tick(12); await flush();
    assert.equal(f.media.plays.length, 2); assert.equal(f.media.plays.at(-1), 1);
    assert.equal(f.media.paused, false); assert.equal(f.player.state.autoplayBlocked, false);
    f.dispose();
  }
});

test("controller retains denial even when a fake player clears its source-local blocked state", async () => {
  let controller, now = 1, attempts = 0, gestures = 0;
  const timers = new Map(); let id = 0;
  const media = { src: "/a.mp4", loop: false, seeking: false, ended: false };
  const state = { status: "Ready", ready: true, playing: false, autoplayBlocked: false,
    muted: false, volume: 0.4, currentTime: 0, duration: 4000, disposed: false };
  const player = {
    state,
    setSource(source) { media.src = source; state.autoplayBlocked = false; controller.playerChanged(); },
    seek(time) { state.currentTime = time; controller.playerChanged(); },
    pause() { state.playing = false; controller.playerChanged(); },
    play() { gestures++; state.autoplayBlocked = false; state.playing = true; controller.playerChanged(); },
    scheduledAutoplay() { attempts++; state.autoplayBlocked = true; controller.playerChanged(); return true; },
  };
  controller = new TheaterScheduleController({ player, media, path: "/schedule.json", origin: "https://local.test",
    fetcher: async () => response(schedule([entry("a", 0, 10), entry("b", 10, 10, "/b.mp4")], 20)), now: () => now,
    setTimeout: (callback) => { timers.set(++id, callback); return id; }, clearTimeout: (timer) => timers.delete(timer), onChange: () => {},
  });
  await controller.start(); assert.equal(attempts, 1);
  for (const time of [11, 21]) {
    now = time; const callbacks = [...timers.values()]; timers.clear(); callbacks.forEach((callback) => callback());
    assert.equal(state.autoplayBlocked, false); assert.equal(attempts, 1);
  }
  controller.setFollowing(false); controller.setFollowing(true); assert.equal(attempts, 1);
  now = 22; controller.play(); assert.equal(gestures, 1); assert.equal(state.currentTime, 2);
  controller.dispose(); assert.equal(timers.size, 0);
});

const invalidClocks = [NaN, Infinity, -Infinity, Number.MAX_SAFE_INTEGER + 1,
  -Number.MAX_SAFE_INTEGER - 1, Number.MAX_SAFE_INTEGER, 8_640_000_000_001, -8_640_000_000_001];

test("invalid clock during manifest load stays a clock error, keeps timer alive and recovers without audio", async () => {
  for (const clock of invalidClocks) {
    const f = await fixture(schedule(), clock);
    assert.match(f.statuses.at(-1), /^Clock unavailable/); assert.equal(f.timers.size, 1);
    f.ready(); await flush(); assert.equal(f.media.plays.length, 0);
    assert.doesNotThrow(() => f.tick(clock)); assert.equal(f.timers.size, 1);
    assert.match(f.statuses.at(-1), /^Clock unavailable/);
    assert.doesNotThrow(() => f.tick(120)); f.ready(); await flush();
    assert.equal(f.media.currentTime, 120); assert.equal(f.media.plays.length, 0); assert.equal(f.timers.size, 1);
    assert.match(f.statuses.at(-1), /UTC/);
    f.tick(3601); assert.equal(f.media.plays.length, 0); // Safety pause survives cycle boundaries.
    f.setNow(3602); f.controller.play(); assert.equal(f.media.plays.at(-1), 2);
    await flush(); f.dispose(); assert.equal(f.timers.size, 0);
  }
});

test("invalid periodic clock safely pauses audio, rearms and forces recovery even within drift tolerance", async () => {
  for (const clock of invalidClocks) {
    const f = await fixture(schedule(), 123); f.ready(); await flush(); assert.equal(f.media.plays.length, 1);
    const loads = f.media.loads, seeks = f.seeks.length;
    assert.doesNotThrow(() => f.tick(clock)); await flush();
    assert.match(f.statuses.at(-1), /^Clock unavailable/); assert.equal(f.media.paused, true);
    assert.equal(f.timers.size, 1); assert.equal(f.media.loads, loads); assert.equal(f.seeks.length, seeks);
    // Explicit Play with an invalid clock also stays safely paused.
    assert.doesNotThrow(() => f.controller.play()); assert.equal(f.media.plays.length, 1);
    assert.doesNotThrow(() => f.tick(124)); await flush();
    assert.equal(f.media.currentTime, 124); assert.equal(f.seeks.length, seeks + 1);
    assert.equal(f.media.plays.length, 1); assert.equal(f.timers.size, 1);
    f.controller.setFollowing(false); f.controller.setFollowing(true); assert.equal(f.media.plays.length, 1);
    f.setNow(125); f.controller.play(); assert.equal(f.media.plays.at(-1), 125);
    await flush(); f.dispose(); assert.equal(f.timers.size, 0);
    const count = f.statuses.length; f.tick(clock); f.controller.playerChanged(); assert.equal(f.statuses.length, count);
  }
});

test("short non-loop holds silent final frame; looping uses available duration after offset", async () => {
  const short = await fixture(schedule(), 20); short.media.duration = 10; short.ready();
  assert.equal(short.media.currentTime, 9.999); assert.equal(short.media.plays.length, 0);
  short.tick(21); assert.equal(short.media.paused, true); short.dispose();
  const loop = await fixture(schedule([entry("loop", 0, 3600, "/a.mp4", 3, true)]), 16);
  loop.media.duration = 10; loop.ready(); await flush(); assert.equal(loop.media.currentTime, 5);
  loop.media.currentTime = 10; loop.media.ended = true; loop.media.paused = true;
  loop.media.event("ended"); await flush(); assert.equal(loop.media.plays.at(-1), 5); assert.equal(loop.media.plays.length, 2); loop.dispose();
});

test("loader rejects invalid paths, redirects, invalid schema, oversized/offline responses", async () => {
  const signal = new AbortController().signal;
  for (const path of ["https://other.test/x", "//other.test/x", "/a/../b", "/a?b", "/a%2fb", "/a#b"]) {
    await assert.rejects(loadTheaterSchedule(path, "https://local.test", signal, () => { throw new Error("must not fetch"); }));
  }
  let init;
  await loadTheaterSchedule("/schedule.json", "https://local.test", signal, async (_url, options) => { init = options; return response(schedule()); });
  assert.equal(init.credentials, "omit"); assert.equal(init.cache, "no-store"); assert.equal(init.redirect, "error");
  for (const fetcher of [async () => response({ token: "bootstrap fixture" }), async () => { throw new Error("offline"); },
    async () => response("x".repeat(32769)), async () => { const r = response(schedule()); Object.defineProperty(r, "redirected", { value: true }); return r; }]) {
    await assert.rejects(loadTheaterSchedule("/schedule.json", "https://local.test", signal, fetcher));
  }
  const f = await fixture({}); f.ready(); assert.match(f.statuses.at(-1), /unavailable/); assert.equal(f.media.plays.length, 0);
  f.controller.play(); assert.equal(f.media.plays.length, 0);
  assert.equal(f.media.src, "");
  f.controller.setFollowing(false); assert.equal(f.media.src, "/local.mp4");
  f.ready(); f.controller.play(); assert.equal(f.media.plays.length, 1); f.dispose();
});

test("loader deadline covers a stalled body and cancels it at 10 seconds", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let cancelled = false;
  const pending = loadTheaterSchedule("/schedule.json", "https://local.test", new AbortController().signal,
    async () => new Response(new ReadableStream({ cancel() { cancelled = true; } }), { headers: { "content-type": "application/json" } }));
  const rejected = assert.rejects(pending, /timed out/); await flush();
  t.mock.timers.tick(9999); await flush(); assert.equal(cancelled, false);
  t.mock.timers.tick(1); await rejected; await flush(); assert.equal(cancelled, true);
});

test("dispose aborts pending manifest; late fetch resolution cannot revive scheduler", async () => {
  let resolve, signal, calls = 0;
  const pending = new Promise((yes) => { resolve = yes; });
  const controller = new TheaterScheduleController({ player: {}, media: {}, path: "/schedule.json", origin: "https://local.test",
    fetcher: async (_url, options) => { signal = options.signal; return pending; }, onChange: () => calls++,
  });
  const loaded = controller.start();
  controller.dispose(); assert.equal(signal.aborted, true); resolve(response(schedule())); await loaded;
  assert.equal(calls, 2);
});
