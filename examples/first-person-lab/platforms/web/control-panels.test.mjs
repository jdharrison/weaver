import assert from "node:assert/strict";
import test from "node:test";
import { FullscreenControl, fullscreenState, fullscreenPanelState, soundPanelState, installCanvasInteractions } from "./control-panels.ts";
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

test("structured panel states do not infer audio from desired playing or unmute", () => {
  const state = { ready: true, playing: true, muted: false, volume: 0.4, audible: false, soundPending: false, soundError: false };
  assert.equal(soundPanelState(state), 1);
  assert.equal(soundPanelState({ ...state, audible: true }), 3);
  assert.equal(soundPanelState({ ...state, soundPending: true, audible: true }), 2);
  assert.equal(soundPanelState({ ...state, soundError: true }), 4);
  assert.equal(soundPanelState({ ...state, ready: false }), 5);
  assert.equal(soundPanelState({ ...state, buffering: true }), 2);
  assert.equal(soundPanelState({ ...state, ready: false, failed: true }), 4);
  assert.equal(soundPanelState({ ...state, disposed: true }), 0);
  assert.equal(fullscreenState({ available: true, active: false, pending: false, error: "false" }), false);
  assert.equal(fullscreenPanelState({ available: true, active: true, pending: true, error: false }), 2);
});

test("child bridge toggle remains synchronous and state/event validation is observed", async () => {
  const child = new EventTarget(), doc = new EventTarget(), states = [];
  let state = { available: true, active: false, pending: false, error: false }, gesture = false, toggles = 0;
  child.jdharrisonGetFullscreenState = () => state;
  child.jdharrisonToggleFullscreen = () => { assert.ok(gesture); toggles++; state = { ...state, pending: true }; };
  const control = new FullscreenControl(child, doc, (value) => states.push(value));
  gesture = true; control.toggle(); gesture = false;
  assert.equal(toggles, 1); assert.equal(states.at(-1).active, false); assert.equal(states.at(-1).pending, true);
  control.toggle(); assert.equal(toggles, 1);
  const notify = (detail) => { const event = new Event("jdharrison-fullscreen-state"); event.detail = detail; child.dispatchEvent(event); };
  await flush(); const count = states.length;
  notify({ active: true }); assert.equal(states.length, count);
  state = { ...state, pending: false, error: true }; notify(state);
  assert.equal(fullscreenPanelState(states.at(-1)), 4);
  gesture = true; control.toggle(); gesture = false; assert.equal(toggles, 2);
  state = { ...state, pending: false, active: true, error: false }; notify(state);
  assert.equal(fullscreenPanelState(states.at(-1)), 3);
  await flush(); control.dispose(); const disposedCount = states.length;
  notify(state); control.toggle(); assert.equal(states.length, disposedCount);
});

test("embedded child without its parent bridge does not fullscreen the iframe document", () => {
  const child = new EventTarget(), doc = new EventTarget();
  child.parent = {};
  Object.assign(doc, { fullscreenEnabled: true, documentElement: { requestFullscreen: () => assert.fail("child fallback") }, exitFullscreen() {} });
  const control = new FullscreenControl(child, doc, () => {});
  assert.equal(control.state.available, false); control.toggle(); control.dispose();
});

test("standalone fullscreen has pending, rejection retry, observed entry/exit and cleanup", async () => {
  const child = new EventTarget(), doc = new EventTarget(), states = [];
  let reject, resolve, calls = 0;
  Object.assign(doc, { fullscreenEnabled: true, fullscreenElement: null,
    documentElement: { requestFullscreen: () => { calls++; return new Promise((yes, no) => { resolve = yes; reject = no; }); } },
    exitFullscreen: () => { doc.fullscreenElement = null; doc.dispatchEvent(new Event("fullscreenchange")); return Promise.resolve(); } });
  const control = new FullscreenControl(child, doc, (state) => states.push(state));
  control.toggle(); assert.equal(calls, 1); assert.equal(fullscreenPanelState(states.at(-1)), 2);
  control.toggle(); assert.equal(calls, 1);
  reject(new Error("denied")); await flush(); assert.equal(fullscreenPanelState(states.at(-1)), 4);
  control.toggle(); doc.fullscreenElement = doc.documentElement; doc.dispatchEvent(new Event("fullscreenchange"));
  resolve(); await flush(); assert.equal(fullscreenPanelState(states.at(-1)), 3);
  control.toggle(); await flush(); assert.equal(fullscreenPanelState(states.at(-1)), 1);
  control.dispose(); const count = states.length;
  doc.dispatchEvent(new Event("fullscreenerror")); assert.equal(states.length, count);
});

function inputFixture() {
  const listeners = new Map(), picks = [], actions = [];
  const owner = () => ({ addEventListener: (type, listener, options) => {
    assert.equal(options.capture, true); assert.equal(options.passive, false); listeners.set(type, listener);
  }, removeEventListener: (type) => listeners.delete(type) });
  const child = owner(), doc = { ...owner(), pointerLockElement: null };
  const canvas = { getBoundingClientRect: () => ({ left: 10, top: 20, right: 210, bottom: 120, width: 200, height: 100 }) };
  let hit = 1, enabled = true, invalidations = 0, onActivate;
  const interactions = installCanvasInteractions({ window: child, document: doc, canvas, enabled: () => enabled,
    pick: (x, y) => { picks.push([x, y]); return hit; }, activate: (target) => { actions.push(target); onActivate?.(target); },
    invalidate: () => invalidations++ });
  const send = (type, values = {}) => {
    const event = { target: canvas, isTrusted: true, button: 0, pointerId: 1, pointerType: "mouse", isPrimary: true, timeStamp: 0, clientX: 110, clientY: 70,
      consumed: false, preventDefault() { this.consumed = true; }, stopImmediatePropagation() { this.stopped = true; }, ...values };
    listeners.get(type)?.(event); return event;
  };
  return { send, doc, canvas, picks, actions, listeners, reset: interactions.reset, remove: interactions.dispose,
    invalidations: () => invalidations, setHit: (value) => { hit = value; }, disable: () => { enabled = false; }, enable: () => { enabled = true; },
    onActivate: (callback) => { onActivate = callback; } };
}

test("subtitle panel and both portfolio halves accept mouse and touch activation", () => {
  for (const target of [3, 4, 5]) for (const pointerType of ["mouse", "touch"]) {
    const f = inputFixture();
    f.setHit(target);
    f.send("pointerdown", { pointerType });
    f.send("pointerup", { pointerType });
    f.send("click");
    assert.deepEqual(f.actions, [target]);
    f.remove();
  }
});

test("trusted canvas hits use Y-up NDC or locked center and consume before compatibility pointer lock", () => {
  const f = inputFixture();
  assert.equal(f.send("pointerdown", { clientX: 60, clientY: 45 }).consumed, true);
  assert.deepEqual(f.picks.at(-1), [-0.5, 0.5]); assert.deepEqual(f.actions, [1]);
  assert.equal(f.send("mousedown").consumed, true); assert.equal(f.send("click").consumed, true);
  f.doc.pointerLockElement = f.canvas; f.setHit(2); f.send("pointerdown");
  assert.deepEqual(f.picks.at(-1), [0, 0]); assert.deepEqual(f.actions, [1, 2]);
  f.setHit(0); assert.equal(f.send("pointerdown").consumed, false); assert.equal(f.send("mousedown").consumed, false);
  f.doc.pointerLockElement = null; f.setHit(1);
  assert.equal(f.send("pointerdown", { isTrusted: false }).consumed, false);
  assert.equal(f.send("pointerdown", { clientX: 0 }).consumed, false);
  f.disable(); assert.equal(f.send("pointerdown").consumed, false);
  f.remove(); assert.equal(f.listeners.size, 0);
});

test("touch activates once at tap release, never on drag, cancelled input or a different panel", () => {
  const f = inputFixture(), touch = { pointerType: "touch" };
  f.send("pointerdown", touch); assert.deepEqual(f.actions, []);
  f.send("pointerup", touch); assert.deepEqual(f.actions, [1]);
  f.send("click"); assert.deepEqual(f.actions, [1]);
  f.send("pointerdown", touch); f.send("pointermove", { ...touch, clientX: 130 }); f.send("pointerup", touch);
  assert.deepEqual(f.actions, [1]);
  f.send("pointerdown", touch); f.send("pointercancel", touch); f.send("pointerup", touch);
  assert.deepEqual(f.actions, [1]);
  f.send("pointerdown", touch); f.setHit(2); f.send("pointerup", touch); assert.deepEqual(f.actions, [1]);
  f.setHit(1); f.send("pointerdown", touch);
  f.send("pointerdown", { ...touch, pointerId: 2, isPrimary: false });
  f.send("pointerup", touch); assert.deepEqual(f.actions, [1]);
});

test("panel-origin touch drags reach the engine from down through release on either half", () => {
  for (const [panel, clientX] of [[1, 50], [2, 170]]) {
    const f = inputFixture(), touch = { pointerType: "touch", clientX };
    f.setHit(panel);
    const events = [
      f.send("pointerdown", touch), f.send("touchstart"),
      f.send("pointermove", { ...touch, clientY: 40 }), f.send("touchmove"),
      f.send("pointerup", { ...touch, clientY: 40 }), f.send("touchend"),
    ];
    for (const event of events) {
      assert.equal(event.consumed, false, "movement/look must retain the complete touch sequence");
      assert.equal(event.stopped, undefined);
    }
    assert.deepEqual(f.actions, [], "a drag must not become a panel click");
    f.remove();
  }
});

test("single touch taps activate on release without stealing touch input or repeating via mouse", () => {
  const f = inputFixture(), touch = { pointerType: "touch" };
  assert.equal(f.send("pointerdown", touch).consumed, false);
  assert.equal(f.send("touchstart").consumed, false);
  assert.deepEqual(f.actions, []);
  assert.equal(f.send("pointerup", { ...touch, timeStamp: 100 }).consumed, false);
  assert.equal(f.send("touchend").consumed, false);
  assert.deepEqual(f.actions, [1]);
  for (const type of ["mousedown", "mouseup", "click"]) assert.equal(f.send(type).consumed, true);
  assert.deepEqual(f.actions, [1]);
  f.setHit(0);
  assert.equal(f.send("pointerdown", touch).consumed, false);
  assert.equal(f.send("touchmove").consumed, false);
  assert.equal(f.send("mousedown").consumed, false);
});

test("two thumbs remain independent even when the first lands on a panel", () => {
  const f = inputFixture();
  const left = { pointerType: "touch", clientX: 50 };
  const right = { pointerType: "touch", pointerId: 2, isPrimary: false, clientX: 170 };
  for (const [type, values] of [
    ["pointerdown", left], ["touchstart"], ["pointerdown", right], ["touchstart"],
    ["pointermove", { ...left, clientY: 40 }], ["pointermove", { ...right, clientY: 90 }],
    ["touchmove"], ["pointerup", left], ["touchend"], ["pointerup", right], ["touchend"],
  ]) assert.equal(f.send(type, values).consumed, false, type);
  assert.deepEqual(f.actions, []);
  // A fresh gesture is a tap again, not permanently cancelled by the second thumb.
  f.send("pointerdown", left); f.send("pointerup", left);
  assert.deepEqual(f.actions, [1]);
});

test("long holds, cancellation, focus loss and hidden pages cannot complete a panel tap", () => {
  const touch = { pointerType: "touch" };
  for (const [interrupt, releaseTime] of [
    [() => {}, 600],
    [f => { assert.equal(f.send("pointercancel", touch).consumed, false); assert.equal(f.send("touchcancel").consumed, false); }, 100],
    [f => f.send("lostpointercapture", touch), 100],
    [f => f.send("blur"), 100],
    [f => { f.doc.hidden = true; f.send("visibilitychange"); f.doc.hidden = false; }, 100],
    [f => f.disable(), 100],
  ]) {
    const f = inputFixture();
    f.send("pointerdown", touch);
    interrupt(f);
    assert.equal(f.send("pointerup", { ...touch, timeStamp: releaseTime }).consumed, false);
    assert.deepEqual(f.actions, []);
  }
});

test("a second finger outside the canvas cancels a pending tap, not engine input", () => {
  const f = inputFixture(), touch = { pointerType: "touch" };
  f.send("pointerdown", touch);
  assert.equal(f.send("pointerdown", { ...touch, pointerId: 2, isPrimary: false, target: {} }).consumed, false);
  f.send("pointerup", touch);
  assert.deepEqual(f.actions, []);
});

test("all seat targets dispatch once through the same trusted mouse/touch path as panels", () => {
  for (let target = 100; target <= 123; target++) {
    for (const pointerType of ["mouse", "touch"]) {
      const f = inputFixture();
      f.setHit(target);
      assert.equal(f.send("pointerdown", { pointerType }).consumed, pointerType === "mouse");
      assert.deepEqual(f.actions, pointerType === "touch" ? [] : [target]);
      f.send("pointerup", { pointerType, timeStamp: 100 });
      for (const type of ["mousedown", "mouseup", "click", "dblclick"]) assert.equal(f.send(type).consumed, true);
      assert.deepEqual(f.actions, [target]);
      f.remove();
    }
  }
});

test("seated exit allows touch and locked mouse, but an unlocked mouse click remains capture-only", () => {
  const f = inputFixture();
  f.setHit(200);
  for (const type of ["pointerdown", "pointerup", "mousedown", "mouseup", "click"]) {
    assert.equal(f.send(type).consumed, false, type);
  }
  assert.deepEqual(f.actions, []);
  f.doc.pointerLockElement = {};
  assert.equal(f.send("pointerdown").consumed, false, "another element's lock is not scene capture");
  f.doc.pointerLockElement = f.canvas;
  assert.equal(f.send("pointerdown", { clientX: 0, clientY: 0 }).consumed, true);
  assert.deepEqual(f.picks.at(-1), [0, 0]);
  f.send("pointerup");
  assert.deepEqual(f.actions, [200]);
  f.doc.pointerLockElement = null;
  const touch = { pointerType: "touch" };
  assert.equal(f.send("pointerdown", touch).consumed, false);
  assert.equal(f.send("pointerup", { ...touch, timeStamp: 100 }).consumed, false);
  assert.deepEqual(f.actions, [200, 200]);
});

test("unknown targets, untrusted input, inert canvases and invalid coordinates fail closed", () => {
  const f = inputFixture();
  for (const target of [0, 6, 99, 124, 199, 201, 100.5, NaN, Infinity]) {
    f.setHit(target);
    assert.equal(f.send("pointerdown").consumed, false);
  }
  f.setHit(100);
  for (const values of [{ isTrusted: false }, { button: 1 }, { button: 2 }, { target: {} }, { clientX: NaN }, { clientY: Infinity }]) {
    assert.equal(f.send("pointerdown", values).consumed, false);
  }
  f.canvas.inert = true;
  assert.equal(f.send("pointerdown").consumed, false);
  f.canvas.inert = false;
  f.send("pointerdown", { pointerType: "touch" });
  f.send("pointerup", { pointerType: "touch", isTrusted: false });
  assert.deepEqual(f.actions, []);
});

test("touch release must retain the exact seat or panel target, never merely the target kind", () => {
  for (const [start, end] of [[100, 101], [100, 200], [200, 100], [100, 1], [1, 100], [4, 5], [5, 4], [4, 100]]) {
    const f = inputFixture(), touch = { pointerType: "touch" };
    f.setHit(start); f.send("pointerdown", touch);
    f.setHit(end); f.send("pointerup", { ...touch, timeStamp: 100 });
    assert.deepEqual(f.actions, []);
  }
});

test("seat drags on either half and multi-finger gestures preserve the entire native touch stream", () => {
  for (const [target, clientX] of [[100, 50], [123, 170], [200, 170]]) {
    const f = inputFixture(), touch = { pointerType: "touch", clientX };
    f.setHit(target);
    for (const [type, values] of [
      ["pointerdown", touch], ["touchstart"], ["pointermove", { ...touch, clientY: 90 }], ["touchmove"],
      ["pointerup", { ...touch, clientY: 90 }], ["touchend"],
      ["pointerdown", touch], ["touchstart"],
      ["pointerdown", { ...touch, pointerId: 2, isPrimary: false, target: {} }], ["touchstart"],
      ["pointerup", touch], ["touchend"], ["pointerup", { ...touch, pointerId: 2 }], ["touchend"],
    ]) {
      const event = f.send(type, values);
      assert.equal(event.consumed, false, type);
      assert.equal(event.stopped, undefined, type);
    }
    assert.deepEqual(f.actions, []);
    f.send("pointerdown", touch); f.send("pointerup", touch);
    assert.deepEqual(f.actions, [target], "a fresh single tap works after the gesture ends");
  }
});

test("seat tap candidates cannot survive lifecycle, focus, capture, resize or explicit phase resets", () => {
  const touch = { pointerType: "touch" };
  for (const target of [100, 200]) {
    for (const interrupt of [
      f => f.send("blur"), f => f.send("focusin", { target: {} }),
      f => f.send("pointerlockchange"), f => f.send("resize"), f => f.send("orientationchange"), f => f.send("pagehide"),
      f => { f.doc.hidden = true; f.send("visibilitychange"); f.doc.hidden = false; },
      f => f.send("pointercancel", touch), f => f.send("lostpointercapture", touch), f => f.send("touchcancel"),
      f => { f.disable(); f.reset(); f.enable(); }, f => f.remove(),
    ]) {
      const f = inputFixture();
      f.setHit(target); f.send("pointerdown", touch);
      interrupt(f);
      f.send("pointerup", { ...touch, timeStamp: 100 });
      assert.deepEqual(f.actions, []);
      assert.ok(f.invalidations() > 0, "Rust pending intent/snapshot is invalidated too");
      f.remove(); const count = f.invalidations(); f.remove();
      assert.equal(f.invalidations(), count);
      assert.equal(f.listeners.size, 0);
    }
  }
});

test("normal canvas focus and implicit capture release do not cancel an accepted touch tap", () => {
  const f = inputFixture(), touch = { pointerType: "touch" };
  f.setHit(100);
  f.send("pointerdown", touch);
  f.send("focusin");
  f.send("pointerup", { ...touch, timeStamp: 100 });
  f.send("lostpointercapture", touch);
  assert.deepEqual(f.actions, [100]);
  assert.equal(f.invalidations(), 0);
});

test("seat taps reject long or invalid durations and movement even when the finger returns", () => {
  for (const duration of [501, -1, NaN]) {
    const f = inputFixture(), touch = { pointerType: "touch" };
    f.setHit(100); f.send("pointerdown", touch); f.send("pointerup", { ...touch, timeStamp: duration });
    assert.deepEqual(f.actions, []);
  }
  const f = inputFixture(), touch = { pointerType: "touch" };
  f.setHit(100); f.send("pointerdown", touch);
  f.send("pointermove", { ...touch, clientX: 119 }); f.send("pointerup", { ...touch, timeStamp: 100 });
  assert.deepEqual(f.actions, []);
  f.send("pointerdown", touch); f.send("pointerup", { ...touch, clientX: 118, timeStamp: 500 });
  assert.deepEqual(f.actions, [100]);
});

test("compatibility suppression survives synchronous focus changes without swallowing middle click or wheel", () => {
  const f = inputFixture();
  f.onActivate(() => f.reset());
  f.send("pointerdown");
  assert.deepEqual(f.actions, [1]);
  for (const type of ["mousedown", "mouseup", "click", "dblclick"]) {
    assert.equal(f.send(type).consumed, true, type);
    assert.equal(f.send(type, { button: 1 }).consumed, false, type);
    assert.equal(f.send(type, { button: 2 }).consumed, false, type);
  }
  for (const type of ["wheel", "auxclick"]) assert.equal(f.send(type).consumed, false);
  f.setHit(0); f.send("pointerdown");
  assert.equal(f.send("mousedown").consumed, false, "a fresh blank click can capture");
});
