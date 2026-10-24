import assert from "node:assert/strict";
import test from "node:test";
import { TheaterStartup } from "./theater-startup.ts";

class Element extends EventTarget {
  hidden = false;
  disabled = false;
  inert = false;
  dataset = {};
  textContent = "";
  children = [];
  contains(target) { return target === this || this.children.includes(target); }
  focus() { this.document.activeElement = this; }
}

function click(element, trusted = true) {
  const event = new Event("click", { cancelable: true });
  Object.defineProperty(event, "isTrusted", { value: trusted });
  element.dispatchEvent(event);
}

function fixture(t) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const previousNode = globalThis.Node;
  globalThis.Node = Element;
  t.after(() => { if (previousNode === undefined) delete globalThis.Node; else globalThis.Node = previousNode; });
  const document = Object.assign(new EventTarget(), { visibilityState: "visible", activeElement: null });
  const window = new EventTarget();
  const [root, status, play, retry, continueMuted, canvas, other] = Array.from({ length: 7 }, () => {
    const element = new Element(); element.document = document; return element;
  });
  root.children = [status, play, retry, continueMuted];
  other.inert = true;
  const calls = [];
  const state = { ready: false, failed: false, frameReady: false, buffering: false, soundError: false,
    playing: false, audible: false, muted: true, duration: 0, disposed: false, soundPending: false };
  const player = { state, setMuted: (muted) => { calls.push(["muted", muted]); state.muted = muted; } };
  const schedule = { state: { loading: false, failed: false },
    play: (sound) => calls.push(["play", sound]),
    retry: (...args) => calls.push(["retry", ...args]),
    continueMuted: () => { assert.equal(state.muted, true); calls.push(["continue"]); } };
  const ui = new TheaterStartup({ window, document, root, status, play, retry, continueMuted,
    background: [canvas, other], player, schedule,
    onOpenChange: (open) => calls.push(["open", open]), onDismiss: () => { calls.push(["dismiss"]); canvas.focus(); } });
  t.after(() => ui.dispose());
  const ready = () => { state.ready = true; state.duration = 60; ui.update(); };
  const key = (key, target = document.activeElement, shiftKey = false, type = "keydown") => {
    const event = new Event(type, { cancelable: true });
    Object.defineProperties(event, { target: { value: target }, key: { value: key }, shiftKey: { value: shiftKey } });
    window.dispatchEvent(event); return event;
  };
  return { ui, state, calls, schedule, root, status, play, retry, continueMuted, canvas, other, document, window, ready, key };
}

test("confirmation opens only at combined scene readiness; metadata suffices for Ready", (t) => {
  const f = fixture(t);
  f.ui.update(); f.ui.setSceneReady(false);
  assert.equal(f.root.hidden, true);
  assert.deepEqual(f.calls, []);
  f.ui.setSceneReady(true);
  assert.equal(f.root.hidden, false); assert.equal(f.root.dataset.state, "preparing");
  assert.equal(f.play.disabled, true); assert.equal(f.continueMuted.disabled, false);
  assert.equal(f.document.activeElement, f.continueMuted);
  f.schedule.state.loading = true;
  f.ready(); assert.equal(f.play.disabled, true);
  f.schedule.state.loading = false; f.ui.update();
  assert.equal(f.state.frameReady, false);
  assert.equal(f.status.textContent, "Ready"); assert.equal(f.play.disabled, false);
  assert.deepEqual(f.calls, [["open", true]]);
});

test("trusted Play is synchronous and dismissal requires audible playback AND a frame, not a promise", async (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true); f.ready();
  let inGesture = false;
  f.schedule.play = (sound) => { assert.equal(inGesture, true); f.calls.push(["play", sound]); return Promise.resolve(); };
  click(f.play, false); assert.deepEqual(f.calls, [["open", true]]);
  inGesture = true; click(f.play); inGesture = false;
  assert.deepEqual(f.calls, [["open", true], ["play", true]]);
  assert.equal(f.root.dataset.state, "starting"); assert.equal(f.play.disabled, true);
  await Promise.resolve(); f.state.playing = true; f.ui.update();
  assert.equal(f.root.hidden, false);
  f.state.frameReady = true; f.ui.update(); assert.equal(f.root.hidden, false);
  f.state.frameReady = false; f.state.audible = true; f.ui.update(); assert.equal(f.root.hidden, false);
  f.state.frameReady = true; f.ui.update();
  assert.equal(f.root.hidden, true); assert.equal(f.canvas.inert, false); assert.equal(f.other.inert, true);
  assert.equal(f.document.activeElement, f.canvas);
  f.ui.setSceneReady(true); assert.equal(f.root.hidden, true);
});

test("initial muted autoplay pending disables Play and shows starting until it settles", (t) => {
  const f = fixture(t); f.ui.setSceneReady(true); f.ready();
  f.state.soundPending = true; f.state.playing = true; f.ui.update();
  assert.equal(f.play.disabled, true);
  assert.equal(f.root.dataset.state, "starting");
  assert.match(f.status.textContent, /Starting playback/);
  click(f.play); assert.equal(f.calls.some(([kind]) => kind === "play"), false);
  f.state.soundPending = false; f.ui.update();
  assert.equal(f.play.disabled, false);
  assert.equal(f.root.dataset.state, "ready");
  click(f.play); assert.deepEqual(f.calls.at(-1), ["play", true]);
});

test("denied sound dismisses into the room only after muted fallback settles with a decoded frame", (t) => {
  const f = fixture(t); f.ui.setSceneReady(true); f.ready(); click(f.play);
  Object.assign(f.state, { soundError: true, muted: true, audible: false, playing: true, soundPending: true, frameReady: true });
  f.ui.update();
  assert.equal(f.root.hidden, false);
  assert.equal(f.root.dataset.state, "starting");
  assert.doesNotMatch(f.status.textContent, /couldn’t start/);
  f.state.frameReady = false; f.state.soundPending = false; f.ui.update();
  assert.equal(f.root.hidden, false);
  f.state.frameReady = true; f.ui.update();
  assert.equal(f.root.hidden, true);
  assert.equal(f.canvas.inert, false);
  assert.equal(f.calls.filter(([kind]) => kind === "play").length, 1);
  assert.equal(f.calls.filter(([kind]) => kind === "dismiss").length, 1);
});

test("a UTC source transition during Play prepares the replacement and requires a fresh gesture", (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true); f.ready();
  f.schedule.play = (sound) => {
    f.calls.push(["play", sound]); f.state.ready = false; f.state.duration = 0; f.ui.update();
  };
  click(f.play);
  assert.equal(f.root.dataset.state, "preparing"); assert.equal(f.play.disabled, true);
  f.ready();
  assert.equal(f.root.dataset.state, "ready"); assert.equal(f.play.disabled, false);
  assert.equal(f.calls.filter(([kind]) => kind === "play").length, 1);
});

test("manifest/media failures offer Retry; an audio denial retries play in the trusted stack", (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true);
  for (const failed of [f.schedule.state, f.state]) {
    failed.failed = true; f.ui.update();
    assert.equal(f.root.dataset.state, "failed"); assert.equal(f.retry.hidden, false);
    assert.equal(f.continueMuted.disabled, false);
    f.schedule.retry = (...args) => { assert.deepEqual(args, []); f.calls.push(["retry"]); failed.failed = false; f.ui.update(); };
    click(f.retry);
    assert.equal(f.root.dataset.state, "preparing");
  }
  assert.equal(f.calls.filter(([kind]) => kind === "retry").length, 2);
  f.ready(); f.state.soundError = true; f.ui.update();
  assert.equal(f.root.dataset.state, "failed");
  f.schedule.play = (sound) => { f.state.soundError = false; f.calls.push(["play", sound]); f.ui.update(); };
  click(f.retry);
  assert.equal(f.root.dataset.state, "starting"); assert.deepEqual(f.calls.at(-1), ["play", true]);
});

for (const phase of ["preparing", "starting"]) {
  test(`slow ${phase} Retry reloads once and grants a fresh bounded attempt`, (t) => {
    const f = fixture(t);
    f.ui.setSceneReady(true);
    if (phase === "starting") {
      f.ready(); click(f.play); f.state.buffering = f.state.soundPending = true; f.ui.update();
      assert.match(f.status.textContent, /Buffering/);
    }
    f.schedule.retry = (reloadCurrent) => {
      assert.equal(reloadCurrent, true);
      f.calls.push(["retry", reloadCurrent]);
      f.state.ready = f.state.soundPending = f.state.buffering = false;
      f.state.duration = 0;
      f.ui.update();
    };
    t.mock.timers.tick(11_999); f.ui.update(); assert.equal(f.root.dataset.state, phase);
    click(f.retry); assert.equal(f.calls.some(([kind]) => kind === "retry"), false);
    t.mock.timers.tick(1); assert.equal(f.root.dataset.state, "slow");
    assert.equal(f.retry.hidden, false); assert.match(f.status.textContent, /Retry.*continue without sound/);
    click(f.retry); assert.deepEqual(f.calls.at(-1), ["retry", true]);
    assert.equal(f.root.dataset.state, "preparing"); assert.equal(f.retry.hidden, true);
    click(f.retry); click(f.retry);
    assert.equal(f.calls.filter(([kind]) => kind === "retry").length, 1);
    t.mock.timers.tick(11_999); assert.equal(f.root.dataset.state, "preparing");
    t.mock.timers.tick(1); assert.equal(f.root.dataset.state, "slow");
    click(f.retry); assert.equal(f.calls.filter(([kind]) => kind === "retry").length, 2);
    assert.equal(f.root.dataset.state, "preparing");
    click(f.continueMuted);
    assert.equal(f.root.hidden, true);
    assert.deepEqual(f.calls.slice(-4), [["open", false], ["dismiss"], ["muted", true], ["continue"]]);
    const calls = f.calls.length; t.mock.timers.tick(60_000);
    assert.equal(f.calls.length, calls); assert.equal(f.root.hidden, true);
  });
}

test("a failed retry clears its action error before the next normal bounded attempt", (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true); t.mock.timers.tick(12_000);
  f.schedule.retry = (reloadCurrent) => { assert.equal(reloadCurrent, true); throw new Error("fixture retry failure"); };
  click(f.retry); assert.equal(f.root.dataset.state, "failed");
  f.schedule.retry = (...args) => { assert.deepEqual(args, []); f.calls.push(["retry"]); f.ui.update(); };
  click(f.retry);
  assert.equal(f.root.dataset.state, "preparing"); assert.equal(f.retry.hidden, true);
  t.mock.timers.tick(11_999); assert.equal(f.root.dataset.state, "preparing");
  t.mock.timers.tick(1); assert.equal(f.root.dataset.state, "slow");
});

test("Continue dismisses immediately during loading, errors, and pending sound; mute precedes release", (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true); f.schedule.state.loading = true;
  click(f.continueMuted);
  assert.equal(f.root.hidden, true); assert.deepEqual(f.calls.at(-1), ["continue"]);
  f.ui.requestSound(); assert.equal(f.root.hidden, false); assert.equal(f.play.disabled, true);
  f.schedule.state.loading = false; f.state.failed = true; f.ui.update();
  click(f.continueMuted); assert.equal(f.root.hidden, true);
  f.state.failed = false; f.ready(); f.ui.requestSound();
  f.state.soundPending = true; f.state.muted = false; f.ui.update();
  click(f.continueMuted);
  assert.equal(f.root.hidden, true); assert.equal(f.state.muted, true);
  assert.equal(f.calls.filter(([kind]) => kind === "continue").length, 3);
});

test("hidden and disposed confirmations cancel timers and never complete from stale media events", (t) => {
  const f = fixture(t);
  f.ui.setVisible(false); f.ui.setSceneReady(true);
  assert.equal(f.root.hidden, true); t.mock.timers.tick(60_000);
  f.ui.setVisible(true); assert.equal(f.root.dataset.state, "preparing");
  t.mock.timers.tick(6_000); f.ui.setVisible(false); t.mock.timers.tick(60_000);
  f.ui.setVisible(true); t.mock.timers.tick(6_000); assert.equal(f.root.dataset.state, "preparing");
  f.ready(); click(f.play); f.ui.setVisible(false);
  f.state.audible = f.state.frameReady = true; f.ui.update();
  f.ui.setVisible(true); assert.equal(f.root.hidden, false); // A return requires a new gesture.
  assert.equal(f.root.dataset.state, "ready");
  f.ui.dispose(); const calls = f.calls.length;
  t.mock.timers.tick(60_000); f.ui.update(); f.ui.setVisible(true); f.ui.setSceneReady(true);
  click(f.play); click(f.retry); click(f.continueMuted); f.ui.requestSound();
  assert.equal(f.calls.length, calls); assert.equal(f.root.hidden, true); assert.equal(f.canvas.inert, false);
});

test("Tab traps enabled actions, native button keys remain uncanceled, Escape continues muted, focus cannot escape", (t) => {
  const f = fixture(t);
  f.ui.setSceneReady(true);
  assert.equal(f.canvas.inert, true); assert.equal(f.document.activeElement, f.continueMuted);
  f.key("Tab"); assert.equal(f.document.activeElement, f.continueMuted);
  f.ready(); f.key("Tab"); assert.equal(f.document.activeElement, f.play);
  f.key("Tab", f.play, true); assert.equal(f.document.activeElement, f.continueMuted);
  f.key("Tab", f.continueMuted, true); assert.equal(f.document.activeElement, f.play);
  f.canvas.focus(); f.document.dispatchEvent(new Event("focusin")); assert.equal(f.document.activeElement, f.play);
  let nativeInput = 0;
  for (const type of ["keydown", "keyup"]) {
    f.window.addEventListener(type, () => nativeInput++);
    for (const key of ["Enter", " "]) {
      assert.equal(f.key(key, f.play, false, type).defaultPrevented, false, `${key} ${type} retains native activation`);
    }
  }
  assert.equal(f.key("w", f.canvas).defaultPrevented, true); assert.equal(nativeInput, 0);
  f.key("Escape"); assert.equal(f.root.hidden, true); assert.equal(f.document.activeElement, f.canvas);
  f.key("w", f.canvas); assert.equal(nativeInput, 1);
});

test("World Sound opens loading/failure UI and retries only the schedule-owned current source", (t) => {
  const f = fixture(t);
  f.ui.requestSound(); assert.equal(f.root.hidden, true);
  f.ui.setSceneReady(true); click(f.continueMuted);
  f.ui.requestSound(); assert.equal(f.root.hidden, false); assert.equal(f.root.dataset.state, "preparing");
  click(f.continueMuted); f.state.failed = true;
  f.ui.requestSound(); assert.equal(f.root.hidden, false); assert.deepEqual(f.calls.at(-1), ["retry"]);
  f.state.failed = false; f.ready(); click(f.continueMuted);
  f.ui.requestSound(); assert.deepEqual(f.calls.at(-1), ["play", true]);
  assert.equal(f.root.dataset.state, "starting");
});
