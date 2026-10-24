import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ControllerHud } from "./controller-hud.ts";

class Element extends EventTarget {
  tagName = "BUTTON";
  isContentEditable = false;
  hidden = false;
  disabled = false;
  inert = false;
  dataset = {};
  children = [];
  attributes = new Map();
  append(...children) { this.children.push(...children); }
  contains(target) { return target === this || this.children.some(child => child.contains(target)); }
  setAttribute(name, value) { this.attributes.set(name, value); }
  focus() { this.doc.activeElement = this; }
}
function trustedClick(element) {
  const event = new Event("click", { cancelable: true });
  Object.defineProperty(event, "isTrusted", { value: true });
  element.dispatchEvent(event);
}
function fixture(t, coarse = false) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "HTMLElement");
  globalThis.HTMLElement = Element;
  t.after(() => { if (original) Object.defineProperty(globalThis, "HTMLElement", original); else delete globalThis.HTMLElement; });
  const child = new EventTarget(); child.matchMedia = () => ({ matches: coarse });
  const doc = new EventTarget();
  doc.head = new Element();
  doc.createElement = () => new Element();
  const elements = Object.fromEntries(["launcher", "overlay", "dialog", "audio", "subtitles", "fullscreen", "status", "scene", "background"].map(key => [key, new Element()]));
  for (const element of Object.values(elements)) element.doc = doc;
  elements.dialog.children = [elements.audio, elements.subtitles, elements.fullscreen];
  elements.overlay.children = [elements.dialog];
  elements.background.inert = true;
  doc.getElementById = () => elements.scene;
  doc.activeElement = elements.scene;
  doc.pointerLockElement = elements.scene;
  doc.exitPointerLock = () => { doc.pointerLockElement = null; };
  const events = [], resumes = [], transitions = [];
  let allowed = true;
  child.addEventListener("weaver-controller-change", event => transitions.push(event.detail));
  const hud = new ControllerHud({
    window: child, document: doc, ...elements, background: [elements.scene, elements.background],
    canOpen: () => allowed, canResume: () => allowed,
    clearInputs: () => events.push(["clear", hud.openState, elements.scene.inert]),
    ownership: open => events.push(["owner", open]), resume: lock => resumes.push(lock),
    audioAction: () => events.push("audio"), subtitlesAction: () => events.push("subtitles"), fullscreenAction: () => events.push("fullscreen"),
  });
  t.after(() => hud.dispose());
  return { hud, child, doc, elements, events, resumes, transitions, block: () => { allowed = false; } };
}

test("ownership clears first, preserves existing inertness, emits only transitions and never recaptures during disposal", t => {
  const f = fixture(t);
  f.hud.open(); f.hud.open();
  assert.deepEqual(f.events, [["clear", false, false], ["owner", true]]);
  assert.equal(f.doc.activeElement, f.elements.audio);
  assert.equal(f.doc.pointerLockElement, null);
  f.hud.close(true); f.hud.close(true);
  assert.deepEqual(f.resumes, [true]);
  assert.equal(f.elements.background.inert, true);
  assert.equal(f.elements.scene.inert, false);
  f.doc.pointerLockElement = f.elements.scene;
  f.hud.open(); f.hud.dispose(); f.hud.dispose();
  assert.deepEqual(f.transitions, [{ open: true }, { open: false }, { open: true }, { open: false }]);
  assert.deepEqual(f.resumes, [true]);
  assert.equal(f.elements.launcher.hidden, true);
  const count = f.events.length;
  trustedClick(f.elements.audio); f.hud.open();
  assert.equal(f.events.length, count);
});

test("coarse pointer and other input owners cannot recapture; lifecycle close does not focus", t => {
  const f = fixture(t, true);
  f.hud.open(); f.hud.close(true);
  assert.deepEqual(f.resumes, [false]);
  f.hud.open(); f.block(); f.hud.close(true);
  assert.deepEqual(f.resumes, [false]);
  f.hud.open(); assert.equal(f.hud.openState, false);
});

test("ready controls leave the footer empty instead of showing a close hint", t => {
  const f = fixture(t);
  f.hud.update({ audioEnabled: true, audioAudible: true, audioPending: false, audioFailed: false, audioReady: true,
    subtitles: true, fullscreen: false, fullscreenAvailable: true, fullscreenPending: false, fullscreenError: false });
  assert.equal(f.elements.status.textContent, "");
});

test("desired preference selects audio artwork while pending and errors remain separate", t => {
  const f = fixture(t);
  const state = { audioEnabled: false, audioAudible: false, audioPending: true, audioFailed: false, audioReady: false,
    subtitles: true, fullscreen: false, fullscreenAvailable: true, fullscreenPending: true, fullscreenError: false };
  f.hud.update(state);
  assert.equal(f.elements.audio.dataset.state, "off");
  assert.equal(f.elements.audio.attributes.get("aria-busy"), "true");
  assert.equal(f.elements.audio.disabled, true);
  assert.equal(f.elements.subtitles.attributes.get("aria-pressed"), "true");
  assert.equal(f.elements.fullscreen.disabled, true);
  assert.match(f.elements.status.textContent, /Preparing audio/);
  f.hud.open(); trustedClick(f.elements.audio);
  assert.equal(f.hud.openState, true); assert.equal(f.elements.audio.dataset.state, "off");
  f.hud.update({ ...state, audioEnabled: true, audioAudible: false, audioPending: false, audioReady: true, fullscreenPending: false, fullscreenError: true });
  assert.equal(f.elements.audio.dataset.state, "on");
  assert.match(f.elements.status.textContent, /Fullscreen failed/);
  f.hud.update({ ...state, audioFailed: true });
  assert.match(f.elements.status.textContent, /retry/);
});

test("launcher-focused explicit outside dismissal resumes gameplay rather than refocusing the launcher", t => {
  const f = fixture(t);
  for (const priorLock of [null, f.elements.scene]) {
    f.doc.pointerLockElement = priorLock;
    f.elements.launcher.focus();
    f.elements.launcher.dispatchEvent(new Event("click"));
    assert.equal(f.hud.openState, true);
    for (const type of ["pointerdown", "pointerup", "click"]) {
      const event = new Event(type, { cancelable: true });
      Object.defineProperty(event, "target", { value: f.elements.overlay });
      f.child.dispatchEvent(event);
      assert.equal(event.defaultPrevented, true);
    }
    assert.equal(f.hud.openState, false);
    assert.notEqual(f.doc.activeElement, f.elements.launcher);
  }
  assert.deepEqual(f.resumes, [false, true]);
});

test("initialization preloads all eight sprites without waiting for image readiness or adding requests on toggle", t => {
  const f = fixture(t);
  const links = f.doc.head.children;
  assert.equal(links.length, 8);
  assert.deepEqual(links.map(link => link.href), [
    "collapsed_state.png", "expanded_base_background_x3.png", "audio_on.png", "audio_off.png",
    "subtitles_on.png", "subtitles_off.png", "fullscreen_on.png", "fullscreen_off.png",
  ].map(file => `/weaver/assets/controller/${file}`));
  assert.ok(links.every(link => link.rel === "preload" && link.as === "image"));
  // No load/error event is dispatched: opening and row actions must remain synchronous.
  f.elements.launcher.dispatchEvent(new Event("click"));
  assert.equal(f.hud.openState, true);
  trustedClick(f.elements.audio);
  assert.equal(f.events.at(-1), "audio");
  f.hud.close(true); f.hud.open();
  assert.equal(links.length, 8);
});

function controllerCssRule(selector) {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]+)\\}`).exec(css);
  assert.ok(match, `Missing CSS rule: ${selector}`);
  return new Map(match[1].split(";").filter(value => value.trim()).map(value => {
    const colon = value.indexOf(":");
    return [value.slice(0, colon).trim(), value.slice(colon + 1).trim()];
  }));
}

test("source-derived HUD geometry anchors bottom-right inside safe areas, including short landscape", () => {
  const overlay = controllerCssRule(".controller-overlay"), dialog = controllerCssRule(".controller-dialog");
  assert.equal(overlay.get("position"), "fixed"); assert.equal(overlay.get("inset"), "0");
  assert.equal(overlay.get("display"), "grid"); assert.equal(overlay.get("place-items"), "end end");
  assert.equal(overlay.get("background"), "rgb(0 0 0 / 70%)");
  assert.match(overlay.get("padding"), /^max\(12px, env\(safe-area-inset-top\)\) max\(12px, env\(safe-area-inset-right\)\)\s+max\(12px, env\(safe-area-inset-bottom\)\) max\(12px, env\(safe-area-inset-left\)\)$/);
  assert.equal(dialog.get("width"), "min(498px, 100%, calc((100dvh - max(12px, env(safe-area-inset-top)) - max(12px, env(safe-area-inset-bottom))) * 498 / 424))");
  assert.equal(dialog.get("aspect-ratio"), "498 / 424");
  const [baseWidth, baseHeight] = dialog.get("aspect-ratio").split(" / ").map(Number);
  const maxWidth = Number(dialog.get("width").match(/^min\((\d+)px/)[1]);
  const [align, justify] = overlay.get("place-items").split(" ");
  // Compute the declared grid/card geometry, not a substitute for browser computed-style QA.
  for (const [viewportWidth, viewportHeight, insets] of [
    [1440, 900, [0, 0, 0, 0]], [390, 844, [47, 0, 34, 0]],
    [844, 390, [0, 44, 21, 44]], [568, 200, [0, 0, 21, 0]],
  ]) {
    const [top, right, bottom, left] = insets.map(value => Math.max(12, value));
    const innerWidth = viewportWidth - left - right, innerHeight = viewportHeight - top - bottom;
    const width = Math.min(maxWidth, innerWidth, innerHeight * baseWidth / baseHeight), height = width * baseHeight / baseWidth;
    const x = left + (innerWidth - width) * (justify === "end" ? 1 : 0.5);
    const y = top + (innerHeight - height) * (align === "end" ? 1 : 0.5);
    assert.ok(Math.abs(x + width - (viewportWidth - right)) < 1e-8, "Card right edge must align to safe area");
    assert.ok(Math.abs(y + height - (viewportHeight - bottom)) < 1e-8, "Card bottom edge must align to safe area");
    assert.ok(x >= left - 1e-8 && y >= top - 1e-8); assert.ok(width <= 498);
  }
});

test("expanded backing artwork is inset about 20px per side without resizing the button layout", () => {
  const dialog = controllerCssRule(".controller-dialog");
  assert.equal(dialog.get("background"), "url('/weaver/assets/controller/expanded_base_background_x3.png') center / 92% 100% no-repeat");
  const maxWidth = Number(dialog.get("width").match(/^min\((\d+)px/)[1]);
  assert.ok(Math.abs(maxWidth * (1 - 0.92) / 2 - 20) < 0.1);
});

test("sprite rows stay below the title inside the three shortened-base recesses and launcher stays compact", () => {
  const row = controllerCssRule(".controller-row");
  assert.equal(row.get("background"), "transparent center / contain no-repeat");
  const left = parseFloat(row.get("left")) / 100 * 498;
  const width = parseFloat(row.get("width")) / 100 * 498;
  const height = parseFloat(row.get("height")) / 100 * 424;
  assert.equal(row.get("left"), "18%"); assert.equal(row.get("width"), "74%"); assert.equal(row.get("height"), "15%");
  assert.ok(left >= 86 && left + width <= 459);
  for (const [selector, expectedTop, slotTop, slotBottom] of [
    [".controller-audio", "30%", 127, 191], [".controller-subtitles", "45%", 190, 255],
    [".controller-fullscreen", "60%", 254, 323],
  ]) {
    const rule = controllerCssRule(selector), y = parseFloat(rule.get("top")) / 100 * 424;
    assert.equal(rule.get("top"), expectedTop);
    assert.ok(y > 125, "Rows must not overlap the title"); assert.ok(y >= slotTop && y + height <= slotBottom);
  }
  assert.equal(controllerCssRule(".controller-launcher").get("width"), "min(110px, calc(100vw - 2rem))");
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(css, /@media \(max-width: 40rem\), \(pointer: coarse\)\s*\{\s*\.controller-launcher\s*\{\s*width: min\(96px, calc\(100vw - 2rem\)\);/);
  const status = controllerCssRule(".controller-status");
  assert.equal(status.get("max-height"), "14%"); assert.equal(status.get("overflow"), "auto");
});

test("ON remains selectable while inaudible or pending so OFF is always available", t => {
  const f = fixture(t);
  const state = { audioEnabled: true, audioAudible: false, audioReady: true, audioPending: false, audioFailed: false,
    subtitles: true, fullscreen: false, fullscreenAvailable: false, fullscreenPending: false, fullscreenError: false };
  f.hud.update(state);
  assert.equal(f.elements.audio.dataset.state, "on"); assert.equal(f.elements.audio.disabled, false);
  assert.equal(f.elements.status.textContent, "");
  f.hud.update({ ...state, audioFailed: true });
  assert.match(f.elements.status.textContent, /unavailable.*OFF.*retry\/turn ON.*fresh press/);
  f.hud.update({ ...state, audioPending: true });
  assert.equal(f.elements.audio.disabled, false); assert.equal(f.elements.audio.attributes.get("aria-busy"), "true");
  assert.match(f.elements.status.textContent, /Preparing audio.*OFF.*cancel/);
  f.hud.open(); trustedClick(f.elements.audio); assert.equal(f.events.at(-1), "audio");
  f.hud.update({ ...state, audioEnabled: false, audioPending: true });
  assert.equal(f.elements.audio.disabled, true); assert.equal(f.doc.activeElement, f.elements.subtitles);
  assert.match(f.elements.status.textContent, /Preparing audio.*again when ready/);
  const count = f.events.length; trustedClick(f.elements.audio); assert.equal(f.events.length, count);
});

function launcherPointer(f, type, values = {}) {
  const { target = f.elements.launcher, timeStamp = 100, ...fields } = values;
  const event = new Event(type, { cancelable: true });
  Object.defineProperties(event, { target: { value: target }, timeStamp: { value: timeStamp } });
  Object.assign(event, { pointerType: "touch", pointerId: 2, isPrimary: false, clientX: 20, clientY: 30, ...fields });
  f.child.dispatchEvent(event);
  return event;
}

test("non-primary completed launcher touch opens synchronously without a compatibility click", t => {
  const f = fixture(t, true);
  launcherPointer(f, "pointerdown", { target: f.elements.scene, pointerId: 1, isPrimary: true });
  assert.equal(launcherPointer(f, "pointerdown").defaultPrevented, true);
  assert.equal(f.hud.openState, false);
  assert.equal(launcherPointer(f, "pointerup", { timeStamp: 200 }).defaultPrevented, true);
  assert.equal(f.hud.openState, true);
  assert.deepEqual(f.events, [["clear", false, false], ["owner", true]]);
  assert.deepEqual(f.transitions, [{ open: true }]);
  assert.equal(f.elements.scene.inert, true); assert.equal(f.doc.pointerLockElement, null);
  // Retargeting the touch-generated click to the newly visible backdrop must not dismiss it.
  assert.equal(launcherPointer(f, "click", { target: f.elements.overlay }).defaultPrevented, true);
  assert.equal(f.hud.openState, true);
  // A different, fresh backdrop gesture may dismiss normally.
  launcherPointer(f, "pointerdown", { target: f.elements.overlay, pointerId: 3 });
  launcherPointer(f, "pointerup", { target: f.elements.overlay, pointerId: 3, timeStamp: 200 });
  launcherPointer(f, "click", { target: f.elements.overlay, pointerId: 3 });
  assert.equal(f.hud.openState, false);
  assert.equal(launcherPointer(f, "click").defaultPrevented, true);
  // Older browsers emit a MouseEvent with no pointer identity.
  const lateClick = new Event("click", { cancelable: true }); f.elements.launcher.dispatchEvent(lateClick);
  assert.equal(lateClick.defaultPrevented, true);
  assert.equal(f.hud.openState, false);
  assert.deepEqual(f.transitions, [{ open: true }, { open: false }]);
});

for (const scenario of ["cancel", "drag", "long press", "different pointer", "release outside", "blur"]) {
  test(`launcher ${scenario} cannot open via pointerup or a subsequent compatibility click`, t => {
    const f = fixture(t, true);
    launcherPointer(f, "pointerdown");
    if (scenario === "cancel") launcherPointer(f, "pointercancel");
    if (scenario === "drag") {
      launcherPointer(f, "pointermove", { clientX: 29 });
      launcherPointer(f, "pointermove", { clientX: 20 });
    }
    if (scenario === "blur") f.child.dispatchEvent(new Event("blur"));
    const values = { timeStamp: scenario === "long press" ? 601 : 200 };
    if (scenario === "different pointer") values.pointerId = 9;
    if (scenario === "release outside") values.target = f.elements.scene;
    launcherPointer(f, "pointerup", values);
    assert.equal(f.hud.openState, false);
    assert.equal(launcherPointer(f, "click").defaultPrevented, true);
    assert.equal(f.hud.openState, false); assert.deepEqual(f.events, []);
  });
}

test("a canceled touch does not steal subsequent native keyboard or mouse launcher activation", t => {
  const f = fixture(t);
  launcherPointer(f, "pointerdown"); launcherPointer(f, "pointercancel");
  const keyboardClick = new Event("click", { cancelable: true });
  Object.defineProperty(keyboardClick, "detail", { value: 0 });
  f.elements.launcher.dispatchEvent(keyboardClick);
  assert.equal(keyboardClick.defaultPrevented, false); assert.equal(f.hud.openState, true);
  f.hud.close(true);
  launcherPointer(f, "pointerdown", { pointerType: "mouse", pointerId: 5 });
  launcherPointer(f, "pointerup", { pointerType: "mouse", pointerId: 5 });
  assert.equal(f.hud.openState, false);
  const mouseClick = new Event("click", { cancelable: true });
  Object.defineProperty(mouseClick, "detail", { value: 1 });
  f.elements.launcher.dispatchEvent(mouseClick);
  assert.equal(mouseClick.defaultPrevented, false); assert.equal(f.hud.openState, true);
});

test("launcher touch candidate cannot survive lifecycle closure or disposal", t => {
  const f = fixture(t);
  launcherPointer(f, "pointerdown"); f.hud.close(false);
  launcherPointer(f, "pointerup", { timeStamp: 200 }); assert.equal(f.hud.openState, false);
  launcherPointer(f, "pointerdown"); f.hud.dispose();
  launcherPointer(f, "pointerup", { timeStamp: 200 }); assert.equal(f.hud.openState, false);
  assert.deepEqual(f.events, []);
});

test("startup dismissal resumes keyboard only even with prior lock and unrelated focus; manual reopening retains prior-lock behavior", t => {
  const f = fixture(t);
  f.elements.background.focus();
  f.hud.open(true); f.hud.open(true);
  f.hud.close(true);
  assert.deepEqual(f.resumes, [false]);
  assert.equal(f.elements.scene.inert, false);
  f.doc.pointerLockElement = f.elements.scene; f.elements.scene.focus();
  f.hud.open(); f.hud.close(true);
  assert.deepEqual(f.resumes, [false, true]);
});

test("startup lifecycle close and blocked explicit dismissal never restore focus or capture", t => {
  const f = fixture(t);
  f.hud.open(true); f.hud.close(false);
  assert.deepEqual(f.resumes, []);
  f.hud.open(true); f.block(); f.hud.close(true);
  assert.deepEqual(f.resumes, []);
  f.hud.open(true); assert.equal(f.hud.openState, false);
});

test("Audio rejects synthetic activation for ready, failed and pending preferences but accepts trusted enabled presses", t => {
  const f = fixture(t);
  const base = { audioEnabled: false, audioAudible: false, audioReady: true, audioPending: false, audioFailed: false,
    subtitles: true, fullscreen: false, fullscreenAvailable: false, fullscreenPending: false, fullscreenError: false };
  f.hud.open();
  for (const state of [base, { ...base, audioFailed: true }, { ...base, audioEnabled: true, audioFailed: true },
    { ...base, audioEnabled: true, audioPending: true }, { ...base, audioPending: true }]) {
    f.hud.update(state);
    const count = f.events.length;
    f.elements.audio.dispatchEvent(new Event("click"));
    assert.equal(f.events.length, count, "untrusted click must not enable, retry or cancel");
    trustedClick(f.elements.audio);
    assert.equal(f.events.length, count + (f.elements.audio.disabled ? 0 : 1));
  }
});
