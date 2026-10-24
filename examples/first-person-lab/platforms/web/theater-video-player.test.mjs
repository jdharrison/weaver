import assert from "node:assert/strict";
import test from "node:test";
import { TheaterVideoPlayer, DEFAULT_THEATER_VIDEO } from "./theater-video-player.ts";

const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

class Media extends EventTarget {
  src = "";
  _muted = false;
  _volume = 1;
  get muted() { return this._muted; }
  set muted(value) { this._muted = value; this.event("volumechange"); }
  get volume() { return this._volume; }
  set volume(value) { this._volume = value; this.event("volumechange"); }
  currentTime = 0;
  duration = 10;
  readyState = 0;
  paused = true;
  seeking = false;
  error = null;
  plays = 0;
  pauses = 0;
  loads = 0;
  pending = null;
  load() { this.loads++; }
  removeAttribute(name) { if (name === "src") this.src = ""; }
  play() { this.plays++; this.paused = false; this.dispatchEvent(new Event("playing")); return this.pending?.promise ?? Promise.resolve(); }
  pause() { this.pauses++; this.paused = true; this.dispatchEvent(new Event("pause")); }
  event(name) { this.dispatchEvent(new Event(name)); }
}

function fixture({ videoFrames = true, visible = true, attachResult = true, markResult = true, autoplay = false, scheduled = false, onRecover } = {}) {
  const media = new Media();
  const frames = new Map();
  const rafs = new Map();
  const timers = new Map();
  let now = 0;
  let id = 0, attaches = 0, marks = 0, detaches = 0;
  if (videoFrames) {
    media.requestVideoFrameCallback = (callback) => { frames.set(++id, callback); return id; };
    media.cancelVideoFrameCallback = (id) => frames.delete(id);
  }
  const states = [];
  const player = new TheaterVideoPlayer({ media, visible, autoplay, scheduled,
    onRecover: () => onRecover?.(player), now: () => now,
    setTimeout: (callback, milliseconds) => { timers.set(++id, { callback, due: now + milliseconds }); return id; },
    clearTimeout: (id) => timers.delete(id),
    attach: () => { attaches++; return attachResult; },
    markFrame: () => { marks++; return markResult; },
    detach: () => { detaches++; },
    requestAnimationFrame: (callback) => { rafs.set(++id, callback); return id; },
    cancelAnimationFrame: (id) => rafs.delete(id),
    onChange: (state) => states.push(state),
  });
  const fire = (queue, time) => {
    const [id, callback] = queue.entries().next().value;
    queue.delete(id);
    if (time !== undefined) media.currentTime = time;
    callback(0, { mediaTime: media.currentTime });
  };
  const metadata = () => { media.readyState = 1; media.event("loadedmetadata"); };
  const ready = () => {
    if (scheduled && !media.src) player.setSource("/scheduled.mp4");
    player.setSceneReady(true); metadata(); media.readyState = 2; media.event("loadeddata");
  };
  const advance = async (milliseconds) => {
    const end = now + milliseconds;
    while (timers.size) {
      const [id, timer] = [...timers].sort((a, b) => a[1].due - b[1].due)[0];
      if (timer.due > end) break;
      now = timer.due; timers.delete(id); timer.callback(); await flush();
    }
    now = end;
  };
  return { media, player, frames, rafs, timers, states, metadata, ready, fire, advance,
    get attaches() { return attaches; }, get marks() { return marks; }, get detaches() { return detaches; } };
}

test("desired audio defaults ON and never queues an audible readiness retry", async () => {
  const f = fixture({ scheduled: true });
  assert.equal(f.player.state.audioEnabled, true);
  assert.equal(f.player.state.muted, true);
  assert.equal(f.player.state.audible, false);
  f.player.setMuted(true); assert.equal(f.player.state.audioEnabled, false);
  f.player.enableSound(); assert.equal(f.player.state.audioEnabled, true);
  assert.equal(f.media.plays, 0);
  f.ready(); await flush();
  assert.equal(f.media.plays, 0);
  f.player.scheduledAutoplay(); await flush();
  assert.equal(f.player.state.muted, true);
  assert.equal(f.player.state.audioEnabled, true);
  f.player.enableSound(); await flush();
  f.media.event("waiting");
  assert.equal(f.player.state.audible, false);
  assert.equal(f.player.state.audioEnabled, true);
  f.player.setMuted(true);
  assert.equal(f.player.state.audioEnabled, false);
  assert.equal(f.player.state.playing, true);
  f.player.setVisible(false); f.player.setSource("/next.mp4");
  assert.equal(f.player.state.audioEnabled, false);
  f.player.dispose();
});

test("stale audible scheduled denial does not block silent replacement autoplay", async () => {
  const f = fixture({ scheduled: true }); f.ready();
  f.player.setMuted(false);
  const pending = deferred(); f.media.pending = pending;
  f.player.scheduledAutoplay();
  f.player.setSource("/next.mp4"); f.metadata();
  f.media.pending = null;
  pending.reject(new DOMException("denied", "NotAllowedError")); await flush();
  assert.equal(f.player.state.autoplayBlocked, false);
  assert.equal(f.player.state.audioEnabled, true);
  assert.equal(f.player.state.muted, true);
  assert.equal(f.player.scheduledAutoplay(), true); await flush();
  assert.equal(f.player.state.playing, true);
  assert.equal(f.player.state.audible, false);
  f.player.dispose();
});

test("scheduled construction and readiness never load the fallback or attach a sourceless video", () => {
  const f = fixture({ scheduled: true, autoplay: true });
  assert.equal(f.media.src, ""); assert.equal(f.media.loads, 0);
  f.player.setSceneReady(true); f.metadata(); f.player.setVisible(true);
  assert.equal(f.media.src, ""); assert.equal(f.media.loads, 0);
  assert.equal(f.attaches, 0); assert.equal(f.player.state.ready, false);
  assert.equal(f.player.scheduledAutoplay(), false); assert.equal(f.media.plays, 0);
  f.player.setSource("/selected.mp4"); f.metadata();
  assert.equal(f.media.src, "/selected.mp4"); assert.equal(f.media.loads, 1);
  assert.equal(f.attaches, 1); assert.equal(f.media.plays, 0);
  f.player.dispose();
});

test("disposing an unstarted scheduled player never loads media, even after late readiness", () => {
  const f = fixture({ scheduled: true });
  f.player.dispose(); f.player.dispose();
  f.player.setSceneReady(true); f.player.setVisible(true); f.metadata();
  f.player.setSource(DEFAULT_THEATER_VIDEO); f.player.play();
  assert.equal(f.media.src, ""); assert.equal(f.media.loads, 0);
  assert.equal(f.attaches, 0); assert.equal(f.media.plays, 0);
});

test("loading, decoded frame readiness, buffering and terminal failure are distinct", async () => {
  const f = fixture({ scheduled: true });
  assert.equal(f.player.state.failed, false); assert.equal(f.player.state.ready, false);
  assert.equal(f.player.state.frameReady, false);
  f.player.setSource("/current.mp4"); f.player.setSceneReady(true); f.metadata();
  assert.equal(f.player.state.ready, true); assert.equal(f.player.state.frameReady, false);
  f.media.readyState = 2; f.media.event("loadeddata");
  assert.equal(f.player.state.frameReady, true);
  f.player.enableSound(); await flush(); f.media.event("waiting");
  assert.equal(f.player.state.buffering, true); assert.equal(f.player.state.audible, false);
  f.media.readyState = 4; f.media.event("canplay"); f.media.event("playing");
  assert.equal(f.player.state.buffering, false); assert.equal(f.player.state.audible, true);
  f.media.seeking = true; f.media.event("seeking"); assert.equal(f.player.state.frameReady, false);
  f.media.seeking = false; f.media.event("seeked"); assert.equal(f.player.state.frameReady, true);
  f.media.error = { code: 2 }; f.media.event("error");
  assert.equal(f.player.state.failed, true); assert.equal(f.player.state.frameReady, false);
  assert.equal(f.player.state.ready, false); assert.equal(f.media.src, "");
  f.media.readyState = 0; f.media.error = null; f.player.setSource("/current.mp4");
  assert.equal(f.player.state.failed, false); assert.equal(f.player.state.soundError, false);
  assert.equal(f.player.state.frameReady, false); f.player.dispose();
});

test("a queued error from a retired resource cannot poison the replacement source", () => {
  const f = fixture({ scheduled: true }); f.ready();
  f.media.error = { code: 2 }; f.media.event("error"); assert.equal(f.player.state.failed, true);
  f.media.error = null; f.player.setSource("/replacement.mp4");
  f.media.event("error"); assert.equal(f.player.state.failed, false);
  assert.equal(f.media.src, "/replacement.mp4"); f.player.dispose();
});

test("sound pending, denial, observed playback, zero-volume recovery and external changes are structured", async () => {
  const f = fixture({ scheduled: true }); f.ready();
  f.player.setVolume(0);
  const pending = deferred(); f.media.pending = pending;
  // play() can resolve without an observed playing event; that is not ON.
  f.media.play = () => { f.media.plays++; f.media.paused = false; return f.media.pending?.promise ?? Promise.resolve(); };
  f.player.enableSound();
  assert.equal(f.media.volume, 0.4); assert.equal(f.media.muted, false);
  assert.equal(f.player.state.soundPending, true); assert.equal(f.player.state.audible, false);
  pending.reject(new DOMException("denied", "NotAllowedError")); await flush();
  assert.equal(f.player.state.soundPending, false); assert.equal(f.player.state.soundError, true);
  assert.equal(f.player.state.audible, false);
  f.media.pending = null; f.player.enableSound(); await flush();
  assert.equal(f.player.state.soundError, false); assert.equal(f.player.state.audible, false);
  f.media.event("playing"); assert.equal(f.player.state.audible, true);
  f.media.event("waiting"); assert.equal(f.player.state.audible, false);
  f.media.event("playing"); assert.equal(f.player.state.audible, true);
  f.media.volume = 0; assert.equal(f.player.state.audible, false);
  f.media.volume = 0.2; assert.equal(f.player.state.audible, true);
  f.media.muted = true; assert.equal(f.player.state.audible, false);
  f.media.muted = false; f.media.pause(); assert.equal(f.player.state.audible, false);
  f.player.dispose();
});

test("muted autoplay passes audible policy and Enable sound plays synchronously in a gesture", async () => {
  const f = fixture({ autoplay: true });
  let gesture = false;
  const play = f.media.play.bind(f.media);
  f.media.play = () => {
    assert.ok(f.media.muted || gesture, "audible play requires activation");
    return play();
  };
  f.ready(); await flush();
  assert.equal(f.media.plays, 1); assert.equal(f.media.muted, true);
  f.player.setVolume(0.2);
  gesture = true; f.player.enableSound(); gesture = false;
  assert.equal(f.media.plays, 2); assert.equal(f.media.muted, false);
  assert.equal(f.media.volume, 0.2); await flush();
  f.player.pause(); f.player.setMuted(true); f.player.setVolume(0.3);
  assert.equal(f.media.paused, true); assert.equal(f.media.plays, 2);
  gesture = true; f.player.enableSound(); gesture = false;
  assert.equal(f.media.plays, 3); assert.equal(f.media.paused, false);
  await flush(); f.player.dispose();
});

test("scheduled player never races readiness, and controller authorization respects audio preferences", async () => {
  const f = fixture({ autoplay: true, scheduled: true });
  f.ready(); await flush(); assert.equal(f.media.plays, 0);
  f.player.setMuted(true); f.player.setVolume(0.15); f.player.seek(6);
  assert.equal(f.player.scheduledAutoplay(), true); await flush();
  assert.equal(f.media.plays, 1); assert.equal(f.media.currentTime, 6);
  assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.15);
  f.player.setSource("/next.mp4"); f.metadata(); await flush();
  assert.equal(f.media.plays, 1);
  f.player.dispose(); assert.equal(f.player.scheduledAutoplay(), false);
});

test("scheduled denial is structured and survives source resets until explicit Play", async () => {
  for (const synchronous of [true, false]) {
    const f = fixture({ scheduled: true, autoplay: true }); f.ready();
    const original = f.media.play;
    if (synchronous) f.media.play = () => { f.media.plays++; throw new Error("NotAllowedError"); };
    else f.media.pending = deferred();
    assert.equal(f.player.state.autoplayBlocked, false);
    assert.equal(f.player.scheduledAutoplay(), true);
    if (!synchronous) f.media.pending.reject(new Error("NotAllowedError"));
    await flush(); assert.equal(f.player.state.autoplayBlocked, true);
    assert.equal(f.states.some((state) => state.autoplayBlocked), true);
    f.media.play = original; f.media.pending = null;
    assert.equal(f.player.scheduledAutoplay(), false); assert.equal(f.media.plays, 1);
    f.player.setSource("/next.mp4"); f.metadata(); await flush();
    assert.equal(f.player.state.autoplayBlocked, true);
    assert.equal(f.player.scheduledAutoplay(), false); assert.equal(f.media.plays, 1);
    f.player.seek(4); f.player.play(); assert.equal(f.player.state.autoplayBlocked, false);
    assert.equal(f.media.plays, 2); assert.equal(f.media.currentTime, 4);
    await flush(); f.player.dispose();
  }
});

test("late scheduled denial is remembered across replacement but ignored after disposal", async () => {
  for (const disposed of [false, true]) {
    const f = fixture({ scheduled: true, autoplay: true }); f.ready();
    const pending = deferred(); f.media.pending = pending; f.player.scheduledAutoplay();
    f.player.setSource("/next.mp4"); f.metadata();
    if (disposed) f.player.dispose();
    const states = f.states.length; pending.reject(new DOMException("Autoplay denied", "NotAllowedError")); await flush();
    assert.equal(f.player.state.autoplayBlocked, !disposed); assert.equal(f.media.plays, 1);
    if (disposed) assert.equal(f.states.length, states);
    else { assert.equal(f.player.scheduledAutoplay(), false); f.player.dispose(); }
  }
});

test("stale source-reset cancellation does not permission-block replacement scheduled autoplay", async () => {
  for (const reason of [new DOMException("Play interrupted by load", "AbortError"),
    new DOMException("Old source unavailable", "NotSupportedError"), new Error("NotAllowedError"), undefined]) {
    const f = fixture({ scheduled: true, autoplay: true }); f.ready();
    const pending = deferred(); f.media.pending = pending;
    assert.equal(f.player.scheduledAutoplay(), true); assert.equal(f.media.plays, 1);
    f.player.setSource("/replacement.mp4"); f.metadata();
    assert.equal(f.player.scheduledAutoplay(), false); // Old promise still owns the play slot.
    const states = f.states.length;
    f.media.pending = null; pending.reject(reason); await flush();
    assert.equal(f.player.state.autoplayBlocked, false);
    assert.equal(f.states.length, states + 1); // Pending settlement is a structured state change.
    assert.equal(f.states.at(-1).soundPending, false);
    assert.equal(f.player.state.ready, true); assert.equal(f.media.paused, true);
    f.player.seek(4); assert.equal(f.player.scheduledAutoplay(), true); await flush();
    assert.equal(f.media.plays, 2); assert.equal(f.media.currentTime, 4);
    assert.equal(f.player.state.autoplayBlocked, false); assert.equal(f.media.paused, false);
    f.player.dispose();
  }
});

test("standalone source replacement preserves its documented fresh autoplay attempt after denial", async () => {
  const f = fixture({ autoplay: true });
  const pending = deferred(); f.media.pending = pending; f.ready(); pending.reject(new Error("NotAllowedError"));
  await flush(); assert.equal(f.player.state.autoplayBlocked, true);
  f.media.pending = null; f.player.setSource("/next.mp4"); assert.equal(f.player.state.autoplayBlocked, false);
  f.metadata(); await flush(); assert.equal(f.media.plays, 2); f.player.dispose();
});

test("ended releases playback intent so an aligned scheduled loop can restart", async () => {
  const f = fixture({ scheduled: true }); f.ready(); f.player.play(); await flush();
  f.media.paused = true; f.media.event("ended"); assert.equal(f.player.state.playing, false);
  f.player.seek(3); f.player.play(); await flush(); assert.equal(f.media.plays, 2); f.player.dispose();
});

test("enabled autoplay waits for metadata and scene readiness, plays muted once, and never restores", async () => {
  for (const metadataFirst of [true, false]) {
    const f = fixture({ autoplay: true });
    assert.equal(f.media.muted, true);
    assert.equal(f.media.volume, 0.4);
    assert.equal(f.media.autoplay, false);
    f.player.setSceneReady(false);
    if (metadataFirst) f.metadata(); else f.player.setSceneReady(true);
    assert.equal(f.media.plays, 0);
    if (metadataFirst) f.player.setSceneReady(true); else f.metadata();
    await flush();
    assert.equal(f.media.plays, 1);
    assert.match(f.player.state.status, /Playing · muted/);
    for (const event of ["loadedmetadata", "loadeddata", "canplay", "seeked", "timeupdate"]) f.media.event(event);
    f.player.setSceneReady(true);
    assert.equal(f.media.plays, 1);
    f.player.setVisible(false); f.player.setVisible(true);
    f.player.setSceneReady(false); f.player.setSceneReady(true);
    assert.equal(f.media.plays, 1);
    assert.equal(f.media.paused, true);
    f.player.play(); await flush();
    assert.equal(f.media.plays, 2);
    f.player.dispose();
  }
});

test("autoplay rejection (sync or async) is actionable, never retried by events, and manual Play retries once", async () => {
  for (const synchronous of [true, false]) {
    const f = fixture({ autoplay: true });
    const play = f.media.play;
    if (synchronous) f.media.play = () => { f.media.plays++; throw new Error("NotAllowedError"); };
    else f.media.pending = deferred();
    f.ready();
    if (!synchronous) f.media.pending.reject(new Error("NotAllowedError"));
    await flush();
    assert.equal(f.player.state.status, "Autoplay blocked/unavailable · press Play");
    assert.equal(f.player.state.ready, true);
    assert.equal(f.player.state.playing, false);
    assert.equal(f.player.state.autoplayBlocked, true);
    assert.equal(f.media.muted, true);
    assert.equal(f.detaches, 0);
    for (const event of ["loadedmetadata", "loadeddata", "canplay", "seeked", "timeupdate"]) f.media.event(event);
    f.player.setSceneReady(true);
    assert.equal(f.media.plays, 1);
    f.player.setVisible(false); f.player.setVisible(true);
    assert.equal(f.media.plays, 1);
    f.media.play = play; f.media.pending = null;
    f.player.play(); f.player.play(); await flush();
    assert.equal(f.media.plays, 2);
    assert.match(f.player.state.status, /Playing · muted/);
    assert.equal(f.player.state.autoplayBlocked, false);
    f.player.dispose();
  }
});

test("autoplay never attempts while hidden or disposed, and explicit Pause before readiness cancels it", async () => {
  const hidden = fixture({ autoplay: true, visible: false }); hidden.ready();
  assert.equal(hidden.media.plays, 0);
  hidden.player.dispose(); hidden.player.setVisible(true); hidden.metadata();
  assert.equal(hidden.media.plays, 0);
  const paused = fixture({ autoplay: true });
  paused.player.pause(); paused.ready(); await flush();
  assert.equal(paused.media.plays, 0);
  paused.player.play(); await flush(); assert.equal(paused.media.plays, 1);
  paused.player.dispose();
  const hiddenBeforeReady = fixture({ autoplay: true });
  hiddenBeforeReady.player.setVisible(false); hiddenBeforeReady.ready(); hiddenBeforeReady.player.setVisible(true);
  assert.equal(hiddenBeforeReady.media.plays, 0);
  hiddenBeforeReady.player.dispose();
});

test("source replacement waits for old play settlement, ignores stale results, and starts only the newest source", async () => {
  for (const reject of [false, true]) {
    const f = fixture({ autoplay: true });
    const old = deferred(); f.media.pending = old; f.ready();
    assert.equal(f.media.plays, 1);
    f.player.setSource("/superseded.mp4"); f.metadata();
    f.player.setSource("/newest.mp4"); f.metadata();
    f.player.play();
    assert.equal(f.media.plays, 1);
    assert.equal(f.media.paused, true);
    const newest = deferred(); f.media.pending = newest;
    if (reject) old.reject(new Error("old source")); else old.resolve();
    await flush();
    assert.equal(f.media.src, "/newest.mp4");
    assert.equal(f.media.plays, 2);
    assert.equal(f.media.paused, false);
    assert.equal(f.player.state.playing, true);
    // Another readiness event or gesture cannot overlap the new pending play.
    f.metadata(); f.player.play(); assert.equal(f.media.plays, 2);
    newest.resolve(); await flush();
    assert.match(f.player.state.status, /Playing · muted/);
    assert.equal(f.frames.size, 1);
    f.player.dispose();
  }
});

test("replacement is not autostarted by stale play completion if paused, hidden, disposed, or unready", async () => {
  for (const cancel of ["pause", "hide", "dispose", "unready"]) {
    const f = fixture({ autoplay: true });
    const old = deferred(); f.media.pending = old; f.ready();
    f.player.setSource("/replacement.mp4");
    if (cancel === "pause") f.player.pause();
    else if (cancel === "hide") f.player.setVisible(false);
    else if (cancel === "dispose") f.player.dispose();
    else f.media.readyState = 0;
    f.media.pending = null; old.resolve(); await flush();
    assert.equal(f.media.plays, 1);
    assert.equal(f.media.paused, true);
    if (cancel === "hide") f.player.setVisible(true);
    if (cancel !== "unready") { f.metadata(); assert.equal(f.media.plays, 1); }
    else { f.metadata(); await flush(); assert.equal(f.media.plays, 2); }
    f.player.dispose();
  }
});

test("source reset enables another autoplay while preserving explicit mute and volume", async () => {
  const f = fixture({ autoplay: true });
  f.player.setMuted(true); f.player.setVolume(0.25); f.ready(); await flush();
  assert.equal(f.media.muted, true); assert.equal(f.media.plays, 1);
  f.player.setVisible(false); f.player.setVisible(true);
  assert.equal(f.media.plays, 1);
  f.player.setSource("/replacement.mp4"); f.metadata(); await flush();
  assert.equal(f.media.plays, 2); assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.25);
  f.player.pause(); f.player.play(); await flush();
  assert.equal(f.media.muted, true); assert.equal(f.media.plays, 3);
  f.player.dispose();
});

test("metadata and real scene readiness are both required, attach exactly once, never autoplay", () => {
  for (const metadataFirst of [true, false]) {
    const f = fixture();
    assert.equal(f.media.src, DEFAULT_THEATER_VIDEO);
    assert.equal(f.media.autoplay, false);
    assert.equal(f.media.muted, true);
    assert.equal(f.media.volume, 0.4);
    assert.equal(f.media.loop, true);
    assert.equal(f.media.playsInline, true);
    assert.equal(f.media.preload, "metadata");
    f.player.play();
    assert.equal(f.media.plays, 0);
    if (metadataFirst) f.metadata(); else f.player.setSceneReady(true);
    assert.equal(f.attaches, 0);
    if (metadataFirst) f.player.setSceneReady(true); else f.metadata();
    f.metadata(); f.player.setSceneReady(true);
    assert.equal(f.attaches, 1);
    assert.equal(f.marks, 0);
    assert.equal(f.media.plays, 0);
    f.player.dispose();
  }
});

test("first explicit Play stays muted on the same video and audio controls never autoplay", async () => {
  const f = fixture(); f.ready();
  assert.equal(DEFAULT_THEATER_VIDEO, "/weaver/assets/theater/Music_SpaceNoirJazz.mp4");
  assert.equal(f.player.state.status, "Ready · press Play");
  f.player.setVolume(0.3);
  assert.equal(f.media.muted, true);
  assert.equal(f.media.plays, 0);
  f.player.play(); await flush();
  assert.equal(f.media.muted, true);
  assert.equal(f.player.state.volume, 0.3);
  assert.match(f.player.state.status, /Playing · muted/);
  f.player.setMuted(true);
  assert.match(f.player.state.status, /Playing · muted/);
  f.player.setMuted(false);
  assert.match(f.player.state.status, /Playing · audio enabled/);
  assert.equal(f.media.plays, 1);
  f.player.dispose();
});

test("explicit mute persists through first Play, replay, hide and scene readiness toggles", async () => {
  const f = fixture(); f.ready(); f.player.setMuted(true);
  f.player.play(); await flush();
  assert.equal(f.media.muted, true);
  f.player.setVolume(0.25);
  f.player.pause(); f.player.play(); await flush();
  assert.equal(f.media.muted, true);
  f.player.setVisible(false);
  assert.equal(f.media.paused, true);
  f.player.setVisible(true);
  f.player.setSceneReady(false); f.player.setSceneReady(true);
  assert.equal(f.media.paused, true);
  assert.equal(f.media.plays, 2);
  f.player.play(); await flush();
  assert.equal(f.media.muted, true);
  assert.equal(f.media.volume, 0.25);
  f.player.setMuted(false); f.player.pause(); f.player.play(); await flush();
  assert.equal(f.media.muted, false);
  f.player.dispose();
});

test("audio setters reject coerced/non-finite values and clamp finite normalized volume", () => {
  const f = fixture(); f.ready();
  for (const invalid of [NaN, Infinity, -Infinity, "0.7", null, undefined, true, {}, new Number(0.7)]) {
    f.player.setVolume(invalid);
    assert.equal(f.media.volume, 0.4);
  }
  for (const invalid of [0, 1, "false", null, undefined, {}]) {
    f.player.setMuted(invalid);
    assert.equal(f.media.muted, true);
  }
  for (const [input, expected] of [[-1, 0], [2, 1], [0.125, 0.125], [0, 0], [1, 1]]) {
    f.player.setVolume(input);
    assert.equal(f.media.volume, expected);
    assert.equal(f.player.state.volume, expected);
  }
  assert.equal(f.media.plays, 0);
  f.player.play(); assert.equal(f.media.muted, true);
  f.player.dispose();
});

test("external volumechange synchronizes media state and playing status without restarting", async () => {
  const f = fixture(); f.ready(); f.player.play(); await flush();
  f.media.muted = true; f.media.volume = 0.7;
  assert.equal(f.states.at(-1).muted, true);
  assert.equal(f.states.at(-1).volume, 0.7);
  assert.match(f.player.state.status, /Playing · muted/);
  f.media.muted = false; f.media.volume = 0;
  assert.match(f.player.state.status, /Playing · volume 0/);
  f.media.volume = 0.6;
  assert.match(f.player.state.status, /Playing · audio enabled/);
  assert.equal(f.media.plays, 1);
  f.player.dispose();
  const emissions = f.states.length;
  f.media.muted = true; f.media.volume = 0.2;
  f.player.setMuted(false); f.player.setVolume(0.9);
  assert.equal(f.states.length, emissions);
  assert.equal(f.media.muted, true);
  assert.equal(f.media.volume, 0.2);
});

test("audio property failures are actionable and retryable without starting another player", async () => {
  const f = fixture(); f.ready(); f.player.setMuted(true); f.player.play(); await flush();
  Object.defineProperty(f.media, "muted", { configurable: true, get: () => true, set: () => { throw new Error("browser restriction"); } });
  f.player.setMuted(false);
  assert.match(f.player.state.status, /Audio change unavailable/);
  assert.equal(f.media.plays, 1);
  delete f.media.muted;
  f.player.setMuted(false);
  assert.match(f.player.state.status, /audio enabled/);
  Object.defineProperty(f.media, "volume", { configurable: true, get: () => 0.4, set: () => { throw new Error("unsupported volume"); } });
  f.player.setVolume(0.8);
  assert.match(f.player.state.status, /Volume change unavailable/);
  delete f.media.volume; f.player.setVolume(0.8);
  assert.equal(f.player.state.volume, 0.8);
  f.player.dispose();
});

test("Enable sound property failures keep video playable and synchronous failures remain retryable", async () => {
  const f = fixture(); f.ready();
  Object.defineProperty(f.media, "muted", { configurable: true, get: () => true, set: () => { throw new Error("browser restriction"); } });
  f.player.enableSound();
  await flush();
  assert.equal(f.player.state.soundError, true);
  assert.equal(f.player.state.ready, true);
  assert.equal(f.player.state.playing, true);
  assert.equal(f.media.plays, 1);
  delete f.media.muted;
  const play = f.media.play;
  f.media.play = () => { throw new Error("NotAllowedError"); };
  f.player.enableSound();
  assert.match(f.player.state.status, /Play failed/);
  assert.equal(f.player.state.ready, true);
  assert.equal(f.player.state.playing, false);
  f.media.play = play;
  f.player.enableSound(); await flush();
  assert.equal(f.media.muted, false);
  assert.equal(f.media.plays, 2);
  assert.match(f.player.state.status, /audio enabled/);
  f.player.dispose();
});

test("unavailable volume still permits muted playback", async () => {
  const f = fixture(); f.ready(); f.player.setVolume(0);
  Object.defineProperty(f.media, "volume", { configurable: true, get: () => 0, set: () => { throw new Error("unsupported volume"); } });
  f.player.enableSound(); await flush();
  assert.equal(f.player.state.playing, true);
  assert.equal(f.player.state.muted, true);
  assert.equal(f.player.state.soundError, true);
  assert.equal(f.player.state.autoplayBlocked, false);
  assert.equal(f.media.plays, 1);
  f.player.dispose();
});

test("retired audible denial never retries against a replacement source", async () => {
  const f = fixture({ scheduled: true }); f.ready();
  const pending = deferred(); f.media.pending = pending;
  f.player.enableSound();
  f.player.setSource("/replacement.mp4"); f.metadata();
  pending.reject(new DOMException("denied", "NotAllowedError")); await flush();
  assert.equal(f.media.plays, 1);
  assert.equal(f.player.state.autoplayBlocked, false);
  assert.equal(f.player.state.playing, false);
  f.player.dispose();
});

for (const synchronous of [false, true]) {
  test(`denied audible play falls back once to muted video (${synchronous ? "sync" : "async"})`, async () => {
    const f = fixture({ scheduled: true }); f.ready();
    const play = f.media.play.bind(f.media);
    const attempts = [];
    let gesture = true;
    f.media.play = () => {
      attempts.push({ muted: f.media.muted, gesture });
      if (!f.media.muted) {
        const reason = new DOMException("denied", "NotAllowedError");
        if (synchronous) throw reason;
        return Promise.reject(reason);
      }
      return play();
    };
    f.player.enableSound(); gesture = false;
    await flush();
    assert.deepEqual(attempts.map(({ muted }) => muted), [false, true]);
    assert.equal(attempts[0].gesture, true);
    assert.equal(f.player.state.playing, true);
    assert.equal(f.player.state.muted, true);
    assert.equal(f.player.state.audible, false);
    assert.equal(f.player.state.soundError, true);
    assert.equal(f.player.state.audioEnabled, true);
    assert.equal(f.player.state.autoplayBlocked, false);
    assert.equal(f.player.state.soundPending, false);
    f.media.event("canplay"); f.media.event("timeupdate"); await flush();
    assert.equal(attempts.length, 2, "no asynchronous audible replay");
    const pauses = f.media.pauses;
    f.player.setMuted(true);
    assert.equal(f.media.pauses, pauses);
    assert.equal(f.player.state.audioEnabled, false);
    assert.equal(f.player.state.playing, true);
    f.player.dispose();
  });
}

test("watchdog resumes an unexpected native pause without losing mute, volume or source", async () => {
  const f = fixture({ scheduled: true }); f.ready();
  f.player.setMuted(true); f.player.setVolume(0.23); f.player.play(); await flush();
  const loads = f.media.loads;
  f.media.pause();
  assert.equal(f.player.state.playing, false);
  await f.advance(1000);
  assert.equal(f.media.plays, 2);
  assert.equal(f.player.state.playing, true);
  assert.equal(f.media.loads, loads);
  assert.equal(f.media.muted, true); assert.equal(f.media.volume, 0.23);
  assert.equal(f.timers.size, 1); assert.equal(f.frames.size, 1);
  f.player.dispose(); assert.equal(f.timers.size, 0);
});

test("watchdog leaves progressing decoded and RAF playback alone", async () => {
  for (const videoFrames of [true, false]) {
    const f = fixture({ videoFrames }); f.ready(); f.player.play(); await flush();
    const loads = f.media.loads;
    for (let time = 1; time <= 40; time++) {
      f.fire(videoFrames ? f.frames : f.rafs, time);
      await f.advance(1000);
    }
    assert.equal(f.media.plays, 1); assert.equal(f.media.loads, loads);
    assert.equal(f.timers.size, 1); f.player.dispose();
  }
});

test("frozen decoded frames trigger one soft restart then one decoder reload, not endless retries", async () => {
  let recoveries = 0;
  const f = fixture({ scheduled: true, onRecover: (player) => {
    recoveries++; player.seek(7); player.scheduledAutoplay();
  } });
  f.ready(); f.player.play(); await flush();
  const loads = f.media.loads;
  const staleFrame = [...f.frames.values()][0];
  // Advancing media clocks and schedule seeks cannot conceal missing decoded frames.
  for (let time = 1; time <= 8; time++) {
    f.player.seek(time); f.media.event("timeupdate"); await f.advance(1000);
  }
  assert.equal(f.media.plays, 2); assert.equal(f.media.loads, loads);
  f.media.event("stalled"); await f.advance(8000);
  assert.equal(f.media.loads, loads + 1); assert.equal(f.player.state.ready, false);
  assert.equal(recoveries, 0);
  f.metadata(); await flush();
  assert.equal(recoveries, 1); assert.equal(f.media.currentTime, 7);
  assert.equal(f.media.plays, 3); assert.equal(f.frames.size, 1);
  staleFrame(0, { mediaTime: 9 }); assert.equal(f.frames.size, 1);
  await f.advance(8000);
  assert.equal(f.player.state.failed, true); assert.match(f.player.state.status, /remains stalled/);
  assert.equal(f.timers.size, 0); assert.equal(f.frames.size, 0);
  const plays = f.media.plays, finalLoads = f.media.loads;
  await f.advance(60_000); f.media.event("canplay");
  assert.equal(f.media.plays, plays); assert.equal(f.media.loads, finalLoads);
  f.player.dispose();
});

test("a hung play is retired by decoder recovery and late settlement cannot pause its replacement", async () => {
  for (const reject of [false, true]) {
    const f = fixture({ scheduled: true, onRecover: (player) => player.scheduledAutoplay() });
    f.ready(); const old = deferred(); f.media.pending = old;
    f.player.play(); await f.advance(14_000);
    assert.equal(f.media.plays, 1); assert.equal(f.media.loads, 1);
    await f.advance(1000);
    assert.equal(f.media.loads, 2); assert.equal(f.player.state.soundPending, false);
    f.media.pending = null; f.metadata(); await flush();
    assert.equal(f.media.plays, 2); assert.equal(f.player.state.playing, true);
    if (reject) old.reject(new DOMException("retired", "NotAllowedError")); else old.resolve();
    await flush();
    assert.equal(f.media.paused, false); assert.equal(f.player.state.playing, true);
    assert.equal(f.player.state.autoplayBlocked, false); assert.equal(f.frames.size, 1);
    f.player.dispose();
  }
});

test("manual pause, hiding, unready scene and disposal cancel watchdogs and ignore queued polls", async () => {
  for (const cancel of ["pause", "hidden", "unready", "dispose"]) {
    const f = fixture(); f.ready(); f.player.play(); await flush();
    const stalePoll = [...f.timers.values()][0].callback;
    if (cancel === "pause") f.player.pause();
    else if (cancel === "hidden") f.player.setVisible(false);
    else if (cancel === "unready") f.player.setSceneReady(false);
    else f.player.dispose();
    const loads = f.media.loads;
    await f.advance(60_000); stalePoll();
    f.media.event("waiting"); f.media.event("canplay");
    assert.equal(f.media.plays, 1); assert.equal(f.media.loads, loads);
    assert.equal(f.timers.size, 0); assert.equal(f.frames.size, 0);
    f.player.dispose();
  }
});

test("autoplay denial and natural endings never start a watchdog retry loop", async () => {
  const blocked = fixture(); blocked.ready();
  blocked.media.play = () => { blocked.media.plays++; return Promise.reject(new DOMException("denied", "NotAllowedError")); };
  blocked.player.scheduledAutoplay(); await flush(); await blocked.advance(60_000);
  assert.equal(blocked.media.plays, 1); assert.equal(blocked.timers.size, 0);
  blocked.player.dispose();
  const ended = fixture(); ended.ready(); ended.player.play(); await flush();
  ended.media.paused = true; ended.media.event("ended"); await ended.advance(60_000);
  assert.equal(ended.media.plays, 1); assert.equal(ended.timers.size, 0); ended.player.dispose();
});

test("a frozen seek reloads once and a hung metadata recovery times out", async () => {
  const f = fixture({ scheduled: true }); f.ready(); f.player.play(); await flush();
  f.media.seeking = true; f.media.event("seeking"); await f.advance(8000);
  assert.equal(f.media.loads, 2); assert.equal(f.media.plays, 1);
  await f.advance(15_000);
  assert.equal(f.player.state.failed, true); assert.match(f.player.state.status, /recovery timed out/);
  assert.equal(f.timers.size, 0); f.player.dispose();
});

test("decoded callbacks keep one outstanding request and mark each new frame, not duplicate times", async () => {
  const f = fixture(); f.ready(); f.player.play(); await flush();
  assert.equal(f.frames.size, 1);
  assert.equal(f.rafs.size, 0);
  f.fire(f.frames, 0);
  assert.equal(f.marks, 1);
  for (let i = 1; i <= 300; i++) { f.fire(f.frames, i / 29.97); assert.equal(f.frames.size, 1); }
  assert.equal(f.marks, 301);
  f.player.pause();
  assert.equal(f.frames.size, 0);
  assert.equal(f.player.state.playing, false);
  f.player.dispose();
});

test("RAF fallback polls currentTime but never dirties unchanged 60 Hz ticks; partial feature falls back", async () => {
  const f = fixture({ videoFrames: false });
  f.media.requestVideoFrameCallback = () => { throw new Error("uncancellable API must not be used"); };
  f.ready(); f.player.play(); await flush();
  for (let i = 0; i < 60; i++) f.fire(f.rafs, 0);
  assert.equal(f.marks, 1);
  f.fire(f.rafs, 1 / 29.97);
  assert.equal(f.marks, 2);
  assert.equal(f.rafs.size, 1);
  f.player.pause(); assert.equal(f.rafs.size, 0);
  f.player.dispose();
});

test("paused seeks update only after data is available and do not restart playback", () => {
  const f = fixture(); f.ready();
  f.player.seek(500); assert.equal(f.media.currentTime, 10);
  f.media.seeking = true; f.media.event("seeking"); f.media.event("timeupdate");
  assert.equal(f.marks, 1);
  f.media.seeking = false; f.media.event("seeked");
  assert.equal(f.marks, 2);
  f.player.seek(-1); f.media.event("seeked");
  assert.equal(f.media.currentTime, 0);
  assert.equal(f.marks, 3);
  f.player.seek(NaN); assert.equal(f.media.currentTime, 0);
  assert.equal(f.media.plays, 0);
  assert.equal(f.frames.size, 0);
  f.player.dispose();
});

test("hidden cancels frames including late cancelled callbacks; restoration requires Play", async () => {
  const f = fixture(); f.ready(); f.player.play(); await flush();
  const stale = [...f.frames.values()][0];
  f.player.setVisible(false);
  assert.equal(f.frames.size, 0);
  stale(0, { mediaTime: 2 });
  assert.equal(f.marks, 1);
  f.player.play(); assert.equal(f.media.plays, 1);
  f.player.setVisible(true);
  assert.equal(f.frames.size, 0);
  assert.equal(f.media.paused, true);
  f.player.play(); await flush(); assert.equal(f.media.plays, 2);
  f.player.dispose();
});

test("a queued pause event after fulfilled replay cannot cancel the current run or frame scheduling", async () => {
  for (const videoFrames of [true, false]) {
    const f = fixture({ autoplay: true, videoFrames }); f.ready(); await flush();
    assert.equal(f.media.plays, 1);
    let deliverPause;
    f.media.pause = () => {
      f.media.pauses++;
      f.media.paused = true;
      deliverPause = () => f.media.event("pause");
    };
    f.player.pause();
    assert.equal(f.media.paused, true);
    assert.equal(f.frames.size + f.rafs.size, 0);
    f.player.play(); await flush();
    assert.equal(f.media.plays, 2);
    assert.equal(f.media.paused, false);
    const queue = videoFrames ? f.frames : f.rafs;
    assert.equal(queue.size, 1);
    const emissions = f.states.length;
    deliverPause();
    assert.equal(f.media.paused, false);
    assert.equal(f.player.state.playing, true);
    assert.match(f.player.state.status, /Playing · muted/);
    assert.equal(f.states.length, emissions);
    assert.equal(queue.size, 1);
    const marks = f.marks;
    f.fire(queue, 1 / 30);
    assert.equal(f.marks, marks + 1);
    assert.equal(queue.size, 1);
    // A later playing event must not stop the resumed run either.
    f.media.event("playing");
    assert.equal(f.media.paused, false);
    assert.equal(f.media.plays, 2);
    assert.equal(queue.size, 1);
    // A real interruption stops frame scheduling but remains recoverable.
    f.media.pause(); deliverPause();
    assert.equal(f.player.state.playing, false);
    assert.equal(queue.size, 0);
    assert.equal(f.player.state.status, "Playback interrupted · recovering…");
    await f.advance(1000);
    assert.equal(f.media.plays, 3); assert.equal(queue.size, 1);
    f.player.dispose();
  }
});

test("a queued playing event while currently paused cannot change status or schedule frames", async () => {
  const f = fixture({ autoplay: true }); f.ready(); await flush();
  f.player.pause();
  const emissions = f.states.length, pauses = f.media.pauses;
  f.media.event("playing");
  assert.equal(f.player.state.status, "Paused · press Play");
  assert.equal(f.player.state.playing, false);
  assert.equal(f.media.paused, true);
  assert.equal(f.media.pauses, pauses);
  assert.equal(f.states.length, emissions);
  assert.equal(f.frames.size, 0);
  f.player.dispose();
});

test("a browser-originated pause remains immediately retryable with explicit Play", async () => {
  const f = fixture(); f.ready(); f.player.play(); await flush();
  f.media.pause();
  assert.equal(f.frames.size, 0); assert.equal(f.player.state.playing, false);
  f.player.play(); await flush(); assert.equal(f.media.plays, 2);
  f.player.dispose();
});

test("initially hidden waits to attach, then remains paused", () => {
  const f = fixture({ visible: false }); f.ready();
  assert.equal(f.attaches, 0);
  f.player.setVisible(true);
  assert.equal(f.attaches, 1);
  assert.equal(f.media.plays, 0);
  f.player.dispose();
});

test("one pending play is bounded, pause invalidates success, source changes invalidate rejection", async () => {
  const f = fixture(); f.ready();
  f.media.pending = deferred();
  f.player.play(); f.player.play();
  assert.equal(f.media.plays, 1);
  f.player.pause();
  f.media.pending.resolve(); await flush();
  assert.equal(f.media.paused, true);
  assert.equal(f.frames.size, 0);
  f.media.pending = deferred(); f.player.play();
  f.player.setSource("/replacement.mp4");
  f.media.pending.reject(new Error("old source")); await flush();
  assert.equal(f.detaches, 1);
  assert.equal(f.player.state.status, "Loading movie metadata…");
  f.metadata(); assert.equal(f.attaches, 2);
  f.player.dispose();
});

test("disposal is idempotent, releases source/decoder and ignores late play and media events", async () => {
  const f = fixture(); f.ready(); f.media.pending = deferred(); f.player.play();
  f.player.dispose(); f.player.dispose();
  assert.equal(f.detaches, 1);
  assert.equal(f.media.src, "");
  assert.equal(f.frames.size, 0);
  const loads = f.media.loads;
  f.media.pending.resolve(); await flush();
  f.media.event("loadedmetadata"); f.media.event("error"); f.player.setSource("/another.mp4");
  assert.equal(f.attaches, 1);
  assert.equal(f.media.loads, loads);
  assert.equal(f.media.paused, true);
  const muted = f.media.muted, volume = f.media.volume;
  f.player.setMuted(!muted); f.player.setVolume(0.9);
  assert.equal(f.media.muted, muted); assert.equal(f.media.volume, volume);
  assert.equal(f.player.state.status, "Movie stopped");
});

test("play rejection is retryable without losing attachment; media failure detaches and releases source", async () => {
  const f = fixture(); f.ready(); f.media.pending = deferred(); f.player.play();
  f.media.pending.reject(new Error("NotAllowedError")); await flush();
  assert.match(f.player.state.status, /Play failed/);
  assert.equal(f.detaches, 0);
  f.media.pending = null; f.player.play(); await flush();
  assert.equal(f.media.plays, 2);
  assert.equal(f.media.muted, true);
  assert.match(f.player.state.status, /muted/);
  f.media.error = { code: 4 }; f.media.event("error");
  assert.match(f.player.state.status, /media error 4/);
  assert.equal(f.detaches, 1);
  assert.equal(f.media.src, "");
  assert.equal(f.frames.size, 0);
  f.player.dispose(); assert.equal(f.detaches, 1);
});

test("source configuration stays same-origin and bounded", () => {
  for (const source of ["https://remote.test/movie.mp4", "//remote.test/movie.mp4", "relative.mp4", "/\\\\remote.test/movie.mp4", "/ movie.mp4", "/" + "x".repeat(2048)]) {
    const f = fixture(); f.player.setSource(source);
    assert.match(f.player.state.status, /same-origin root-absolute/);
    assert.equal(f.media.src, ""); f.player.dispose();
  }
});

test("disposal ignores a late play rejection as well as late success", async () => {
  const f = fixture(); f.ready(); f.media.pending = deferred(); f.player.play();
  f.player.dispose(); f.media.pending.reject(new Error("old rejection")); await flush();
  assert.equal(f.player.state.status, "Movie stopped"); assert.equal(f.detaches, 1);
});

test("attachment and dirty rejection are visible and never become a scheduling loop", () => {
  const attach = fixture({ attachResult: false }); attach.ready();
  assert.match(attach.player.state.status, /attachment rejected/);
  attach.player.play(); assert.equal(attach.media.plays, 0); attach.player.dispose();
  const dirty = fixture({ markResult: false }); dirty.ready();
  assert.match(dirty.player.state.status, /rejected the movie frame/);
  assert.equal(dirty.detaches, 1); dirty.player.dispose();
});
