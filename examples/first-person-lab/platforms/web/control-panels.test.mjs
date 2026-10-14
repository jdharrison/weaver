import assert from "node:assert/strict";
import test from "node:test";
import { FullscreenControl, fullscreenState, fullscreenPanelState, soundPanelState, installCanvasPanels } from "./control-panels.ts";
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

test("structured panel states do not infer audio from desired playing or unmute", () => {
  const state = { ready: true, playing: true, muted: false, volume: 0.4, audible: false, soundPending: false, soundError: false };
  assert.equal(soundPanelState(state), 1);
  assert.equal(soundPanelState({ ...state, audible: true }), 3);
  assert.equal(soundPanelState({ ...state, soundPending: true, audible: true }), 2);
  assert.equal(soundPanelState({ ...state, soundError: true }), 4);
  assert.equal(soundPanelState({ ...state, ready: false }), 0);
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
  const listeners = new Map(), picks = [], actions = [], doc = { pointerLockElement: null };
  const child = { addEventListener: (type, listener, options) => {
    assert.equal(options.capture, true); assert.equal(options.passive, false); listeners.set(type, listener);
  }, removeEventListener: (type) => listeners.delete(type) };
  const canvas = { getBoundingClientRect: () => ({ left: 10, top: 20, right: 210, bottom: 120, width: 200, height: 100 }) };
  let hit = 1, enabled = true;
  const remove = installCanvasPanels({ window: child, document: doc, canvas, enabled: () => enabled,
    pick: (x, y) => { picks.push([x, y]); return hit; }, activate: (panel) => actions.push(panel) });
  const send = (type, values = {}) => {
    const event = { target: canvas, isTrusted: true, button: 0, pointerId: 1, pointerType: "mouse", clientX: 110, clientY: 70,
      consumed: false, preventDefault() { this.consumed = true; }, stopImmediatePropagation() { this.stopped = true; }, ...values };
    listeners.get(type)?.(event); return event;
  };
  return { send, doc, canvas, picks, actions, remove, listeners, setHit: (value) => { hit = value; }, disable: () => { enabled = false; } };
}

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
