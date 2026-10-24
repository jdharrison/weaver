import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { build } from "esbuild";

// Strict contract for this static shell's markup, not a replacement for browser DOM QA.
// Unlike the event fixture, this rejects malformed tags and verifies actual nesting.
function parseShellMarkup(html) {
  const root = { tag: "#document", children: [] }, stack = [root], byId = new Map();
  const voidTags = new Set(["meta", "link", "img", "input", "br", "hr", "source", "track"]);
  const tokens = /<!--[\s\S]*?-->|<!doctype[^>]*>|<\/?[a-z][^<>]*>/gi;
  let end = 0;
  for (const match of html.matchAll(tokens)) {
    assert.doesNotMatch(html.slice(end, match.index), /[<>]/, "Malformed tag in shell HTML");
    end = match.index + match[0].length;
    const token = match[0];
    if (token.startsWith("<!")) continue;
    if (token.startsWith("</")) {
      const closing = /^<\/([a-z][\w-]*)\s*>$/i.exec(token);
      assert.ok(closing, `Malformed closing tag: ${token}`);
      assert.equal(stack.at(-1).tag, closing[1], `Unbalanced tag: ${token}`);
      stack.pop();
      continue;
    }
    const opening = /^<([a-z][\w-]*)([\s\S]*?)>$/i.exec(token);
    assert.ok(opening, `Malformed opening tag: ${token}`);
    const attributes = new Map();
    let rest = opening[2];
    while (rest.trim()) {
      const attr = /^\s+([a-z][\w:-]*)(?:\s*=\s*"([^"]*)")?/i.exec(rest);
      assert.ok(attr, `Malformed attributes: ${token}`);
      assert.equal(attributes.has(attr[1]), false, `Duplicate attribute: ${token}`);
      attributes.set(attr[1], attr[2] ?? "");
      rest = rest.slice(attr[0].length);
    }
    const node = { tag: opening[1], attributes, children: [], parent: stack.at(-1) };
    node.parent.children.push(node);
    const id = attributes.get("id");
    if (id) { assert.equal(byId.has(id), false, `Duplicate id: ${id}`); byId.set(id, node); }
    if (!voidTags.has(node.tag)) stack.push(node);
  }
  assert.doesNotMatch(html.slice(end), /[<>]/, "Malformed trailing shell HTML");
  assert.equal(stack.length, 1, "Unclosed shell tags");
  return { root, byId };
}
const shellHtml = readFileSync(new URL("./index.html", import.meta.url), "utf8");
const shellMarkup = parseShellMarkup(shellHtml);

test("actual HUD markup contains a valid launcher image, nested dialog and three accessible native rows", () => {
  const node = id => { const value = shellMarkup.byId.get(id); assert.ok(value, `Missing HTML id: ${id}`); return value; };
  const launcher = node("controller-launcher"), overlay = node("controller-overlay"), dialog = node("controller-dialog");
  assert.equal(launcher.tag, "button");
  assert.equal(launcher.attributes.get("type"), "button");
  assert.equal(launcher.attributes.get("aria-controls"), "controller-dialog");
  assert.equal(launcher.children.length, 1);
  assert.equal(launcher.children[0].tag, "img");
  assert.equal(launcher.children[0].attributes.get("src"), "/weaver/assets/controller/collapsed_state.png");
  assert.equal(launcher.children[0].attributes.get("alt"), "");
  assert.equal(overlay.tag, "div"); assert.equal(dialog.parent, overlay);
  assert.equal(dialog.tag, "section"); assert.equal(dialog.attributes.get("role"), "dialog");
  assert.equal(dialog.attributes.get("aria-modal"), "true");
  assert.deepEqual(dialog.children.filter(child => child.tag === "button").map(child => child.attributes.get("id")),
    ["controller-audio", "controller-subtitles", "controller-fullscreen"]);
  for (const id of ["controller-audio", "controller-subtitles", "controller-fullscreen"]) {
    const button = node(id);
    assert.equal(button.parent, dialog); assert.equal(button.attributes.get("type"), "button");
    assert.ok(button.attributes.get("aria-label"));
    assert.match(button.attributes.get("aria-pressed"), /^(true|false)$/);
  }
  for (const element of shellMarkup.byId.values()) {
    for (const attribute of ["aria-controls", "aria-describedby", "aria-labelledby"]) {
      for (const id of (element.attributes.get(attribute) ?? "").split(/\s+/).filter(Boolean)) node(id);
    }
  }
  assert.doesNotMatch(shellHtml, /Subtitles: on \(required\)|Subtitles stay on|using Sound in the room/);
});

test("markup contract rejects the reported broken image and section tags", () => {
  assert.throws(() => parseShellMarkup(shellHtml.replace('<img src=', '<img</button> src=')), /Malformed tag/);
  assert.throws(() => parseShellMarkup(shellHtml.replace('<section id="controller-dialog"', '<se</div>ction id="controller-dialog"')), /Malformed tag/);
});

// A unit-test bundle only: no real Woven client, generated WASM, or output files.
const bundled = await build({ entryPoints: ["main.ts"], bundle: true, write: false,
  format: "esm", platform: "browser", target: "es2022",
  define: { "import.meta.url": JSON.stringify("https://portfolio.test/weaver/first-person-lab/main.js") }, plugins: [{
    name: "offline-browser-seams",
    setup(builder) {
      builder.onResolve({ filter: /^@signalweave\/woven-client$/ }, () => ({ path: "woven", namespace: "fixture" }));
      builder.onResolve({ filter: /\/pkg\/first_person_lab\.js$/ }, () => ({ path: "wasm", namespace: "fixture" }));
      builder.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({ contents: path === "woven" ? `
        export const AdmissionStatus = { Admitted: 1 };
        export const QueueState = { Admitted: 1 };
        export const AuthenticationScheme = { Bearer: 1, Development: 0 };
        export const DeliveryClass = { UnreliableSequenced: 3 };
        export const MessageKind = { SubscriptionAccepted: 9, SubscriptionRejected: 10,
          EntityEntered: 11, EntityLeft: 12, ReliableEvent: 20, EntityState: 21, ProtocolError: 40, ClientLog: 46 };
        export class WovenClient {
          static connect(config) { return globalThis.__weaverHarness.connect(config); }
        }
      ` : `
        export default function init(options) { return globalThis.__weaverHarness.init(options); }
        export function GenerateGuestName() { return "Bright Otter"; }
        export function IsLegacyUserName(name) { return name === "BrightCalm"; }
        export function GenerateUserID() { return "a8712050-f10b-4d38-93b1-556fab8633d2"; }
        export function set_display_name(value) { globalThis.__weaverHarness.bootEvents.push(["name", value]); return true; }
        export function set_minimal_ui(value) { globalThis.__weaverHarness.bootEvents.push(["minimal", value]); }
        export function start() { globalThis.__weaverHarness.start(); }
        export function attach_theater_video(video) { globalThis.__weaverHarness.videoEvents.push(["attach", video]); return true; }
        export function mark_theater_video_frame() { globalThis.__weaverHarness.videoEvents.push(["frame"]); return true; }
        export function detach_theater_video() { globalThis.__weaverHarness.videoEvents.push(["detach"]); }
        export function set_room_lighting(lobby, auditorium) { globalThis.__weaverHarness.videoEvents.push(["light", lobby, auditorium]); return true; }
        export function pick_control_panel() { globalThis.__weaverHarness.legacyPanelPicks++; return 0; }
        export function pick_scene_interaction(x, y) { const h = globalThis.__weaverHarness; if (h.interactionError) throw new Error("stale interaction binding"); h.panelPicks.push([x, y]); return h.panelHit; }
        export function activate_seat_interaction(target) { const h = globalThis.__weaverHarness; h.seatTargets.push(target); if (h.interactionError) throw new Error("retired interaction"); if (h.seatAccept) h.pendingSeatTarget = target; return h.seatAccept; }
        export function invalidate_scene_interactions() { const h = globalThis.__weaverHarness; h.sceneInvalidations++; h.pendingSeatTarget = null; }
        export function set_control_panel_state(panel, state) { globalThis.__weaverHarness.panelStates.push([panel, state]); return true; }
        export function invalidate_control_panels() { globalThis.__weaverHarness.panelInvalidations++; }
        export function get_npc_interaction_context() { const h = globalThis.__weaverHarness; return h.npcActive ? h.npcContext : undefined; }
        export function submit_npc_question(npcId, requestId, text) { const h = globalThis.__weaverHarness; h.npcQuestions.push([npcId, requestId, text]); return h.npcAccept; }
        export function present_npc_response(npcId, requestId, answer, gesture) { const h = globalThis.__weaverHarness; h.npcResponses.push([npcId, requestId, answer, gesture]); return h.npcAccept; }
        export function clear_npc_dialogue() { globalThis.__weaverHarness.npcClears++; }
        export function get_npc_definitions() { return JSON.stringify(globalThis.__weaverHarness.npcDefinitions); }
        export function set_npc_interaction_active(active) { const h = globalThis.__weaverHarness; h.npcActive = active; h.npcActivations.push(active); if (!active) h.npcContext = undefined; }
        export function set_npc_show_cycle(anchor, cycle) { globalThis.__weaverHarness.npcCycles.push([anchor, cycle]); return true; }
        export function send_host_chat(text) { return globalThis.__weaverHarness.sendChat(text); }
        export function set_host_chat_focus(active) { globalThis.__weaverHarness.chatFocus.push(active); }
        export function reset_host_chat() { globalThis.__weaverHarness.pendingChat.length = 0; }
        export function realtime_connected(id) { globalThis.__weaverHarness.events.push(["connected", id]); return globalThis.__weaverHarness.acceptConnection; }
        export function realtime_disconnected(reason) { globalThis.__weaverHarness.events.push(["disconnected", reason]); return true; }
        export function realtime_entity_entered(id) { globalThis.__weaverHarness.events.push(["entered", id]); return true; }
        export function realtime_entity_left(id) { globalThis.__weaverHarness.events.push(["left", id]); return true; }
        export function realtime_payload(id, seq, text) { globalThis.__weaverHarness.events.push(["payload", id, seq, text]); return true; }
        export function realtime_unreliable_payload(id, seq, payload) { globalThis.__weaverHarness.events.push(["pose", id, seq, payload.slice()]); return true; }
      `, loader: "js" }));
    },
  }] });
const source = bundled.outputFiles[0].text;
const TARGET = { version: 1, url: "https://realtime.test:4434/webtransport", namespaceId: "2",
  sessionId: "2", spaceId: "3", spaceEpoch: "1" };
const TOKEN = "a".repeat(64); // Synthetic fixture, never an environment credential.
const jsonResponse = () => new Response(JSON.stringify({ ...TARGET, token: TOKEN }),
  { headers: { "content-type": "application/json" } });
const flush = async () => { for (let i = 0; i < 60; i += 1) await Promise.resolve(); };
const deferred = () => { let resolve; const promise = new Promise((yes) => { resolve = yes; }); return { promise, resolve }; };
const SCHEDULED_VIDEO = "/weaver/assets/theater/trailers/fixture.mp4";
const LOCAL_VIDEO = "/weaver/assets/theater/Music_SpaceNoirJazz.mp4";
const scheduleResponse = () => new Response(JSON.stringify({
  version: 1, revision: "fixture", anchorUnixSeconds: 0, cycleSeconds: 3600, driftToleranceSeconds: 5,
  entries: [{ id: "fixture", title: "Scheduled fixture", startSeconds: 0, durationSeconds: 3600,
    source: SCHEDULED_VIDEO, sourceStartSeconds: 0, loop: false }],
}), { headers: { "content-type": "application/json" } });
let moduleId = 0;

function trustedClick(element) {
  const event = new Event("click", { cancelable: true });
  Object.defineProperty(event, "isTrusted", { value: true });
  element.dispatchEvent(event);
}

function scenePointer(f, type, values = {}) {
  const { target = f.elements.get("scene"), isTrusted = true, timeStamp = 0, ...fields } = values;
  const event = new Event(type, { cancelable: true });
  Object.defineProperties(event, { isTrusted: { value: isTrusted }, target: { value: target }, timeStamp: { value: timeStamp } });
  Object.assign(event, { button: 0, pointerId: 1, pointerType: "mouse", isPrimary: true, clientX: 100, clientY: 50, ...fields });
  f.window.dispatchEvent(event);
  return event;
}

function soundPanelClick(f) {
  assert.equal(scenePointer(f, "pointerdown").defaultPrevented, true);
}

class KeyboardFixtureEvent extends Event {
  constructor(type, options = {}) {
    super(type, { cancelable: true, ...options });
    for (const [name, fallback] of Object.entries({ key: "", code: "", repeat: false, isComposing: false, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false })) {
      this[name] = options[name] ?? fallback;
    }
  }
}

class Element extends EventTarget {
  tagName = "DIV";
  isContentEditable = false;
  blurCount = 0;
  focusCount = 0;
  focus() { this.focusCount += 1; if (this.ownerDocument) this.ownerDocument.activeElement = this; }
  blur() { this.blurCount += 1; if (this.ownerDocument?.activeElement === this) this.ownerDocument.activeElement = null; this.dispatchEvent(new Event("blur")); }
  value = "";
  textContent = "";
  disabled = true;
  hidden = false;
  open = false;
  inert = false;
  dataset = {};
  attributes = new Map();
  setAttribute(name, value) { this.attributes.set(name, value); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  children = [];
  classes = new Set();
  classList = { add: (value) => this.classes.add(value), toggle: (value, enabled) => {
    if (enabled) this.classes.add(value); else this.classes.delete(value);
  } };
  contains(target) { return target === this || this.children.some(child => child.contains(target)); }
  getBoundingClientRect() { return { left: 0, top: 0, right: 200, bottom: 100, width: 200, height: 100 }; }
  append(...children) { this.children.push(...children); }
  remove() { this.removed = true; }
  setCustomValidity() {}
  checkValidity() { return true; }
  requestSubmit() { this.dispatchEvent(new Event("submit", { cancelable: true })); }
}

function clientFixture() {
  const publishes = [];
  const pending = [];
  const inbox = [];
  const pendingDatagrams = [];
  const datagrams = [];
  let closed = false;
  const startup = [{ messageKind: 9 }, { messageKind: 11, entityId: 7n }];
  const client = {
    publishes,
    closed: () => closed,
    supportsPositionedState: () => true,
    admitWithCancellation: async () => ({ kind: "admission", result: { status: 1 } }),
    joinSession: async () => {},
    subscribeSpace: async () => {},
    recvTimeout: async () => startup.shift(),
    recv: () => inbox.length ? Promise.resolve(inbox.shift()) : new Promise((resolve, reject) => pending.push({ resolve, reject })),
    recvDatagram: () => datagrams.length ? Promise.resolve(datagrams.shift()) : new Promise((resolve, reject) => pendingDatagrams.push({ resolve, reject })),
    publishEvent: async (...args) => {
      publishes.push({ kind: "event", at: performance.now(), channel: args[4], sequence: args[6], payload: args[8].slice() });
    },
    publishUnreliablePositionedState: async (...args) => {
      publishes.push({ kind: "pose", at: performance.now(), channel: args[4], sequence: args[6],
        position: { ...args[8] }, payload: args[9].slice() });
    },
    logger: { info: async () => { publishes.push({ kind: "log", at: performance.now() }); } },
    close: () => {
      closed = true;
      for (const waiter of [...pending.splice(0), ...pendingDatagrams.splice(0)]) waiter.reject(new Error("fixture closed"));
    },
    send: (value) => {
      const index = pending.findIndex((waiter) => waiter.resolve);
      if (index === -1) inbox.push(value);
      else pending.splice(index, 1)[0].resolve(value);
    },
    sendDatagram: (value) => {
      const waiter = pendingDatagrams.shift();
      if (waiter) waiter.resolve(value); else datagrams.push(value);
    },
  };
  return client;
}

class VideoElement extends Element {
  _src = "";
  sources = [];
  loads = [];
  getAttribute(name) { return name === "src" ? this._src || null : super.getAttribute(name); }
  get src() { return this._src; }
  set src(value) { this._src = value; this.sources.push(value); }
  _muted = false;
  _volume = 1;
  get muted() { return this._muted; }
  set muted(value) { this._muted = value; this.dispatchEvent(new Event("volumechange")); }
  get volume() { return this._volume; }
  set volume(value) { this._volume = value; this.dispatchEvent(new Event("volumechange")); }
  readyState = 0;
  currentTime = 0;
  duration = 3981.386;
  paused = true;
  seeking = false;
  error = null;
  pauseCount = 0;
  playCount = 0;
  playError = null;
  playPromise = null;
  load() { this.loads.push(this.src); this.readyState = 0; }
  removeAttribute(name) { if (name === "src") this._src = ""; }
  pause() { this.paused = true; this.pauseCount += 1; }
  play() {
    this.playCount += 1;
    if (this.playError) return Promise.reject(this.playError);
    this.paused = false; this.dispatchEvent(new Event("playing")); return this.playPromise ?? Promise.resolve();
  }
}

async function shell(t, options = {}) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0, unixNow = options.unixNow ?? 0;
  t.mock.method(Date, "now", () => unixNow);
  t.mock.method(performance, "now", () => now);
  const elements = new Map(["network-form", "connect-network", "disconnect-network", "network-status", "connection-badge", "connection-label", "woven-auth",
    "woven-token", "display-name", "join-network", "reload-scene", "multiplayer-settings", "woven-url",
    "woven-namespace", "woven-session", "woven-space", "woven-epoch", "woven-certificate-hash", "status", "scene",
        "subtitle-controls", "theater-subtitle-hud", "theater-subtitles", "theater-subtitle-explanation",
        "theater-controls", "theater-play", "theater-restart", "theater-mute", "theater-volume", "theater-seek", "theater-time", "theater-status",
        "theater-lobby-light", "theater-auditorium-light", "theater-follow-schedule", "theater-schedule-status",
        "scene-instructions", "connection-controls", "theater-startup", "theater-startup-status",
        "theater-startup-play", "theater-startup-retry", "theater-startup-continue",
        "controller-launcher", "controller-overlay", "controller-dialog", "controller-audio", "controller-subtitles", "controller-fullscreen", "controller-status"]
    .map((id) => {
      const markup = shellMarkup.byId.get(id);
      assert.ok(markup, `Fixture id must exist in actual HTML: ${id}`);
      const element = new Element(); element.tagName = markup.tag.toUpperCase();
      element.hidden = markup.attributes.has("hidden");
      return [id, element];
    }));
  for (const [id, value] of Object.entries({ "woven-auth": "bearer", "woven-url": TARGET.url,
    "woven-namespace": "2", "woven-session": "2", "woven-space": "3", "woven-epoch": "1" })) elements.get(id).value = value;
  for (const id of ["theater-lobby-light", "theater-auditorium-light"]) {
    elements.get(id).value = shellMarkup.byId.get(id).attributes.get("value");
  }
  const startupListeners = [];
  for (const id of ["theater-startup", "theater-startup-play", "theater-startup-retry", "theater-startup-continue"]) {
    const element = elements.get(id), listen = element.addEventListener;
    t.mock.method(element, "addEventListener", function (...args) {
      startupListeners.push([id, args[0]]);
      return listen.apply(this, args);
    });
  }
  const video = new VideoElement(), videoFrames = new Map();
  let nextVideoFrame = 0;
  if (options.videoFrames) {
    video.requestVideoFrameCallback = (callback) => { videoFrames.set(++nextVideoFrame, callback); return nextVideoFrame; };
    video.cancelVideoFrameCallback = (id) => videoFrames.delete(id);
  }
  elements.set("theater-video", video);
  const rafs = new Map();
  let nextRaf = 0;
  const document = Object.assign(new EventTarget(), {
    head: new Element(),
    createElement: tag => { const element = new Element(); element.tagName = tag.toUpperCase(); return element; },
    visibilityState: options.visibility ?? "visible",
    activeElement: null,
    pointerLockElement: null,
    pointerLockExits: 0,
    exitPointerLock() { this.pointerLockExits += 1; this.pointerLockElement = null; },
    documentElement: { dataset: {
      ...(options.presentation === undefined ? {} : { weaverPresentation: options.presentation }),
      ...(options.publishRateHz === undefined ? {} : { weaverPublishRateHz: options.publishRateHz }),
    } },
    getElementById: (id) => elements.get(id) ?? null,
    querySelector: (selector) => selector.includes("weaver-theater-schedule") ? { content: "/weaver/theater-schedule.json" } : selector.includes("weaver-theater-video") ? { content: options.localSource ?? "" } : ({ content: selector.includes("weaver-lobby-bootstrap") ?
      (options.visitor === false ? "" : "/api/lobby") : JSON.stringify(TARGET) }),
  });
  for (const element of elements.values()) element.ownerDocument = document;
  elements.get("theater-startup").children = ["theater-startup-play", "theater-startup-retry", "theater-startup-continue"].map(id => elements.get(id));
  elements.get("controller-dialog").children = ["controller-audio", "controller-subtitles", "controller-fullscreen", "controller-status"].map(id => elements.get(id));
    elements.get("controller-overlay").children = [elements.get("controller-dialog")];
    for (const id of ["controller-launcher", "controller-audio", "controller-subtitles", "controller-fullscreen"]) elements.get(id).tagName = "BUTTON";
    const storage = new Map();
  const window = Object.assign(new EventTarget(), { navigator: { onLine: options.onLine ?? true }, isSecureContext: true, location: { origin: options.origin ?? "https://portfolio.test" },
    requestAnimationFrame: (callback) => { rafs.set(++nextRaf, callback); return nextRaf; },
    cancelAnimationFrame: (id) => rafs.delete(id),
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) } });
  const client = clientFixture();
  const clients = [];
  const harness = { acceptConnection: options.acceptConnection !== false, events: [], connects: [], fetches: [], wasmFetches: [], scheduleFetches: [], bootEvents: [], startState: null,
    panelPicks: [], panelStates: [], panelHit: 1, panelInvalidations: 0, legacyPanelPicks: 0,
    sceneInvalidations: 0, seatTargets: [], seatAccept: true, pendingSeatTarget: null, interactionError: false,
    npcActive: false, npcContext: undefined, npcAccept: true, npcQuestions: [], npcResponses: [],
    npcClears: 0, npcActivations: [], npcCycles: [], npcDefinitions: [{ id: "concession_support" }],
    videoEvents: [], chatFocus: [], chatSends: [], pendingChat: [], nextChatSequence: 100n,
    sendChat: (text) => {
      harness.chatSends.push(text);
      if (options.acceptChat === false) return false;
      harness.pendingChat.push(text);
      return true;
    },
    flushChat: () => {
      for (const text of harness.pendingChat.splice(0)) {
        globalThis.weaverChatMessage("7", "Bright Otter", text, true);
        globalThis.weaverRealtimePublishReliable(harness.nextChatSequence++, JSON.stringify({
          kind: "chat", version: 1, display_name: "Bright Otter", text,
        }));
      }
    },
    init: async ({ module_or_path }) => {
      harness.bootEvents.push(["init"]);
      // Model the real WASM initializer consuming the monitored response stream.
      await module_or_path.arrayBuffer();
      assert.equal(typeof globalThis.weaverSceneReady, "function");
      assert.equal(typeof globalThis.weaverSceneFatal, "function");
      if (options.initError) throw new Error("fixture WASM assets missing");
      if (options.init) await options.init(harness);
      harness.bootEvents.push(["initialized"]);
    },
    start: () => {
      harness.bootEvents.push(["start"]);
      if (options.startError) throw new Error("fixture renderer startup failed");
      if (options.readyOnStart) globalThis.weaverSceneReady();
      harness.startState = { joinDisabled: elements.get("join-network").disabled,
        fetchCount: harness.fetches.length, connectionCount: harness.connects.length };
    },
    connect: async (config) => {
      harness.connects.push(config);
      const active = clients.length === 0 ? client : clientFixture();
      clients.push(active);
      return options.connect ? options.connect(active, clients.length) : active;
    },
  };
  harness.progress = [];
  harness.loadingMessages = [];
  window.parent = { postMessage: (data, origin) => {
    const message = { ...JSON.parse(data), origin };
    harness.loadingMessages.push(message);
    if (message.event === "progress") harness.progress.push(message);
  } };
  const globals = { window, document, HTMLElement: Element, Node: Element, KeyboardEvent: KeyboardFixtureEvent, WebTransport: options.noWebTransport ? undefined : function () {},
    __weaverHarness: harness, fetch: async (url, init) => {
      url = String(url);
      if (url.endsWith("/pkg/first_person_lab_bg.wasm")) {
        harness.wasmFetches.push({ url, init });
        return options.wasmFetch ? options.wasmFetch(url, init, harness) : new Response(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]), {
          headers: { "content-type": "application/wasm", "content-length": "8" },
        });
      }
      if (url.endsWith("/weaver/theater-schedule.json")) {
        harness.scheduleFetches.push({ url, init });
        return options.scheduleFetch ? options.scheduleFetch(url, init) : scheduleResponse();
      }
      harness.fetches.push({ url, init });
      return options.fetch ? options.fetch(url, init, harness.fetches.length) : jsonResponse();
    } };
  const names = [...Object.keys(globals), "weaverSceneReady", "weaverSceneFatal", "weaverDisplayNameChanged",
    "weaverRealtimeFatal", "weaverRealtimePublishUnreliable", "weaverRealtimePublishPositionedUnreliable",
    "weaverRealtimePublishLatest", "weaverRealtimePublishReliable", "weaverChatMessage", "weaverSendChat",
    "weaverSetChatFocus", "weaverResumeGameInput", "weaverHostRequestChat", "weaverChatSendCapacity", "weaverDisposeTheaterVideo",
    "weaverRequestChatOpen", "weaverNpcInteraction", "weaverNpcInteractionFrame", "weaverGetNpcInteractionContext",
    "weaverSubmitNpcQuestion", "weaverPresentNpcResponse", "weaverClearNpcDialogue", "weaverGetNpcDefinitions"];
  const originals = new Map(names.map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  Object.assign(globalThis, globals);
  if (options.chatCallback !== undefined) globalThis.weaverChatMessage = options.chatCallback;
  else delete globalThis.weaverChatMessage;
  if (options.requestChat !== undefined) globalThis.weaverHostRequestChat = options.requestChat;
  else delete globalThis.weaverHostRequestChat;
  if (options.npcCallback !== undefined) globalThis.weaverNpcInteraction = options.npcCallback;
  else delete globalThis.weaverNpcInteraction;
  delete globalThis.weaverRequestChatOpen;
  t.after(async () => {
    window.dispatchEvent(new Event("pagehide"));
    globalThis.weaverDisposeTheaterVideo();
    await flush();
    for (const [key, original] of originals) {
      if (original) Object.defineProperty(globalThis, key, original); else delete globalThis[key];
    }
  });
  await import("data:text/javascript;base64," + Buffer.from(source + `\n// fixture ${moduleId++}`).toString("base64"));
  const visible = async (state) => {
    document.visibilityState = state;
    document.dispatchEvent(new Event("visibilitychange"));
    await flush();
  };
  const advance = async (ms) => {
    now += ms;
    t.mock.timers.tick(ms);
    await flush();
  };
  const metadata = () => {
    const video = elements.get("theater-video");
    video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata")); video.dispatchEvent(new Event("loadeddata"));
  };
  return { elements, harness, client, clients, storage, window, document, visible, advance, rafs, videoFrames, metadata, startupListeners,
    dismissStartup: () => {
      if (options.presentation === "minimal") key({ window }, "keydown", "Escape");
      else elements.get("theater-startup-continue").dispatchEvent(new Event("click"));
    },
    setUnixNow: (value) => { unixNow = value; } };
}

test("room light controls match scene defaults and preserve the other room on first adjustment", async (t) => {
  const rust = readFileSync(new URL("../../src/theater.rs", import.meta.url), "utf8");
  const defaults = new Map();
  for (const [id, constant] of [["theater-lobby-light", "LOBBY_LIGHT_LEVEL"],
    ["theater-auditorium-light", "AUDITORIUM_LIGHT_LEVEL"]]) {
    const match = rust.match(new RegExp(constant + ": f32 = ([0-9.]+);"));
    assert.ok(match, `Missing Rust room-light default: ${constant}`);
    const value = Number(match[1]);
    defaults.set(id, value);
    assert.equal(Number(shellMarkup.byId.get(id).attributes.get("value")), value);
  }
  const f = await shell(t, { visitor: false, noWebTransport: true, readyOnStart: true });
  await f.advance(0);
  const lobby = f.elements.get("theater-lobby-light");
  const auditorium = f.elements.get("theater-auditorium-light");
  assert.equal(f.harness.videoEvents.some(([event]) => event === "light"), false);
  lobby.value = "0.95";
  lobby.dispatchEvent(new Event("input"));
  assert.deepEqual(f.harness.videoEvents.filter(([event]) => event === "light"),
    [["light", 0.95, defaults.get("theater-auditorium-light")]]);
  auditorium.value = "0.45";
  auditorium.dispatchEvent(new Event("input"));
  assert.deepEqual(f.harness.videoEvents.filter(([event]) => event === "light").at(-1),
    ["light", 0.95, 0.45]);
});

const npcContext = (sequence = "1") => ({
  version: 1, npcId: "concession_support", profile: "npc_concession_support", displayName: "Concession Support",
  sceneGeneration: "scene_fixture", frameSequence: sequence, distanceSquared: 6.25, radiusWorldUnits: 4,
  room: "lobby", showState: "showing", nearbyProject: null, playerHasSeenIntro: false,
});

test("NPC bridge is renderer-gated but works offline without chat, replication, or inference", async (t) => {
  const snapshots = [];
  const callback = (snapshot) => snapshots.push(snapshot);
  const f = await shell(t, { visitor: false, noWebTransport: true, npcCallback: callback });
  assert.equal(globalThis.weaverNpcInteraction, callback);
  assert.equal(globalThis.weaverGetNpcInteractionContext(), null);
  assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "request_1", "hello"), false);
  assert.deepEqual(f.harness.npcActivations, []);
  globalThis.weaverSceneReady(); await flush();
  assert.deepEqual(f.harness.npcActivations, [true]);
  await f.advance(0);
  assert.deepEqual(f.harness.npcCycles, [[0, 3600]]);
  f.harness.npcContext = JSON.stringify(npcContext());
  globalThis.weaverNpcInteractionFrame(f.harness.npcContext);
  assert.deepEqual(snapshots, [npcContext()]);
  assert.equal(globalThis.weaverGetNpcInteractionContext(), f.harness.npcContext);
  assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "request_1", "What is Woven?"), true);
  assert.equal(globalThis.weaverPresentNpcResponse("concession_support", "request_1", "A local answer", "shrug"), true);
  assert.deepEqual(f.harness.npcQuestions, [["concession_support", "request_1", "What is Woven?"]]);
  assert.deepEqual(f.harness.npcResponses, [["concession_support", "request_1", "A local answer", "shrug"]]);
  assert.deepEqual(globalThis.weaverGetNpcDefinitions(), [{ id: "concession_support" }]);
  assert.deepEqual(f.harness.chatSends, []);
  assert.deepEqual(f.client.publishes, []);
  assert.deepEqual(f.harness.fetches, []);
  assert.deepEqual(f.harness.connects, []);
  const clears = f.harness.npcClears;
  globalThis.weaverClearNpcDialogue();
  assert.equal(f.harness.npcClears, clears + 1);
  assert.equal(f.harness.npcActive, true);
});

test("NPC schedule waits for initialized WASM and renderer readiness never authorizes pre-start context", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true, init: async (harness) => {
    globalThis.weaverSceneReady();
    t.mock.timers.tick(0); await flush();
    assert.deepEqual(harness.scheduleFetches, []);
    assert.deepEqual(harness.npcCycles, []);
    assert.deepEqual(harness.npcActivations, []);
    assert.equal(globalThis.weaverGetNpcInteractionContext(), null);
    assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "too_early", "hello"), false);
  } });
  await f.advance(0);
  assert.deepEqual(f.harness.npcCycles, [[0, 3600]]);
  assert.deepEqual(f.harness.npcActivations, [true]);
  globalThis.weaverSceneReady();
  await f.visible("hidden");
  await f.visible("visible");
  assert.deepEqual(f.harness.npcCycles, [[0, 3600]]);
  assert.deepEqual(f.harness.npcQuestions, []);
});

test("NPC hook errors cannot break the scene; UTF-16 errors and lifecycle retirement fail closed", async (t) => {
  const f = await shell(t, { visitor: false, npcCallback: () => { throw new Error("host failure"); } });
  globalThis.weaverSceneReady(); await flush();
  f.harness.npcContext = JSON.stringify(npcContext());
  assert.doesNotThrow(() => globalThis.weaverNpcInteractionFrame(f.harness.npcContext));
  assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "request_1", "\ud800"), false);
  assert.equal(globalThis.weaverPresentNpcResponse("concession_support", "request_1", "\udfff", "idle"), false);
  assert.equal(f.harness.npcQuestions.length, 0);
  assert.equal(f.harness.npcResponses.length, 0);
  await f.visible("hidden");
  assert.equal(f.harness.npcActive, false);
  assert.equal(globalThis.weaverGetNpcInteractionContext(), null);
  assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "request_2", "hello"), false);
  await f.visible("visible");
  assert.equal(f.harness.npcActive, true);
  assert.equal(globalThis.weaverGetNpcInteractionContext(), null); // Needs a fresh engine frame.
  globalThis.weaverSceneFatal("fixture fatal");
  assert.equal(f.harness.npcActive, false);
  assert.equal(globalThis.weaverGetNpcDefinitions(), null);
  globalThis.weaverSceneReady();
  assert.equal(globalThis.weaverPresentNpcResponse("concession_support", "request_1", "late", "idle"), false);
});

test("E opens the existing composer for a selected offline NPC and never submits on range entry", async (t) => {
  let opens = 0;
  const f = await shell(t, { visitor: false, presentation: "minimal", requestChat: () => { opens++; return true; } });
  globalThis.weaverSceneReady(); f.dismissStartup(); await flush();
  const press = () => f.window.dispatchEvent(new KeyboardFixtureEvent("keydown", { code: "KeyE", key: "e" }));
  press(); assert.equal(opens, 0);
  f.harness.npcContext = JSON.stringify(npcContext());
  press(); assert.equal(opens, 1);
  assert.deepEqual(f.harness.npcQuestions, []);
  f.window.dispatchEvent(new KeyboardFixtureEvent("keydown", { code: "KeyE", key: "e", repeat: true }));
  assert.equal(opens, 1);
  globalThis.weaverSetChatFocus(true);
  press(); assert.equal(opens, 1);
});

test("startup confirmation is accessible while minimal presentation hides diagnostic controls", () => {
  const html = readFileSync("index.html", "utf8");
  const css = readFileSync("styles.css", "utf8");
  assert.doesNotMatch(html + css, /theater-enable-sound|theater-sound-access/);
  assert.match(html, /id="theater-controls"/);
  assert.match(html, /id="theater-startup"[^>]+role="dialog"[^>]+aria-modal="true"/);
  assert.match(html, /aria-labelledby="theater-startup-title"/);
  assert.match(html, /id="theater-startup-status"[^>]+role="status"/);
  assert.match(html, /id="theater-startup-play"[^>]*>Play with sound</);
  assert.match(html, /id="theater-startup-continue"[^>]*>Continue without sound</);
  assert.match(css, /html\[data-weaver-presentation="minimal"\] \.theater-controls \{ display: none; \}/);
  assert.match(css, /html\[data-weaver-presentation="minimal"\] :is\(\.instructions, \.connection-panel\) \{ display: none; \}/);
});

test("diagnostic theater focus still suppresses game input and releases pointer lock", async (t) => {
  const f = await shell(t, { presentation: "minimal", visitor: false });
  const sound = f.elements.get("theater-controls");
  f.document.pointerLockElement = f.elements.get("scene");
  sound.dispatchEvent(new Event("focusin"));
  assert.equal(f.document.pointerLockExits, 1);
  assert.equal(f.harness.chatFocus.at(-1), true);
  sound.dispatchEvent(new Event("focusout"));
  assert.equal(f.harness.chatFocus.at(-1), false);
});

test("load milestones are bounded, monotonic and scene-gated, including synchronous ready", async (t) => {
  const f = await shell(t, { readyOnStart: true, visitor: false });
  assert.deepEqual(f.harness.progress.map(message => message.completed), [1, 3, 7, 10, 12, 14]);
  for (const message of f.harness.progress) {
    assert.equal(message.type, "weaver-host"); assert.equal(message.version, 1);
    assert.equal(message.event, "progress"); assert.equal(message.total, 14);
    assert.equal(message.origin, "https://portfolio.test");
  }
  globalThis.weaverSceneReady();
  assert.equal(f.harness.progress.length, 6);
});

test("download reports real bytes, then initializing, without granting renderer readiness", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true });
  assert.equal(f.harness.wasmFetches.length, 1);
  const updates = f.harness.loadingMessages.filter(message => message.event !== "progress");
  assert.deepEqual(updates.map(({ event, loadedBytes, totalBytes }) => ({ event, loadedBytes, totalBytes })), [
    { event: "download", loadedBytes: 0, totalBytes: 8 },
    { event: "download", loadedBytes: 8, totalBytes: 8 },
    { event: "initializing", loadedBytes: undefined, totalBytes: undefined },
  ]);
  assert.equal(f.harness.progress.at(-1).stage, "started");
  assert.equal(f.harness.scheduleFetches.length, 0);
  assert.equal(f.elements.get("join-network").disabled, true);
});

test("a healthy download can exceed twenty seconds while a stalled download aborts", async (t) => {
  let controller, active, signal;
  const started = deferred();
  const boot = shell(t, { visitor: false, noWebTransport: true, wasmFetch: (_url, init, harness) => {
    active = harness; signal = init.signal;
    return new Response(new ReadableStream({ start(source) { controller = source; started.resolve(); } }));
  } });
  await started.promise; await flush();
  for (let i = 0; i < 4; i++) {
    t.mock.timers.tick(15000);
    controller.enqueue(new Uint8Array([i])); await flush();
    assert.equal(signal.aborted, false);
  }
  assert.equal(active.loadingMessages.some(message => message.event === "initializing"), false);
  t.mock.timers.tick(20000);
  await boot;
  assert.equal(signal.aborted, true);
  assert.equal(active.loadingMessages.some(message => message.event === "initializing"), false);
  assert.deepEqual(active.progress.map(message => message.completed), [1, 3]);
});

test("failed initialization never emits completion and late ready cannot advance progress", async (t) => {
  const f = await shell(t, { initError: true });
  assert.deepEqual(f.harness.progress.map(message => message.completed), [1, 3]);
  globalThis.weaverSceneReady(); await f.advance(20000);
  assert.deepEqual(f.harness.progress.map(message => message.completed), [1, 3]);
});

test("elapsed time cannot advance progress while awaiting the renderer", async (t) => {
  const f = await shell(t, { visitor: false });
  assert.deepEqual(f.harness.progress.map(message => message.completed), [1, 3, 7, 10, 12]);
  await f.advance(1000);
  assert.equal(f.harness.progress.at(-1).completed, 12);
  globalThis.weaverSceneReady();
  assert.equal(f.harness.progress.at(-1).completed, 14);
});

test("theater starts once in a later task, never in the synchronous ready callback or its microtasks", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true });
  const video = f.elements.get("theater-video");
  await f.advance(0);
  assert.equal(f.harness.scheduleFetches.length, 0); assert.deepEqual(video.loads, []);
  globalThis.weaverSceneReady();
  assert.equal(f.harness.progress.at(-1).stage, "scene");
  assert.equal(f.elements.get("reload-scene").hidden, true);
  assert.equal(f.harness.scheduleFetches.length, 0); assert.deepEqual(video.sources, []);
  assert.equal(f.elements.get("theater-startup").hidden, false);
  assert.equal(f.elements.get("theater-startup").dataset.state, "preparing");
  await flush();
  assert.equal(f.harness.scheduleFetches.length, 0); assert.deepEqual(video.loads, []);
  globalThis.weaverSceneReady();
  await f.advance(0);
  assert.equal(f.harness.scheduleFetches.length, 1);
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO]); assert.deepEqual(video.loads, [SCHEDULED_VIDEO]);
  globalThis.weaverSceneReady(); f.metadata(); await f.advance(0);
  assert.equal(f.harness.scheduleFetches.length, 1); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  assert.equal(f.elements.get("theater-startup").dataset.state, "ready");
  f.dismissStartup();
  assert.equal(video.playCount, 1);
});

test("slow manifest cannot block ready and selects the current UTC slot only when it arrives", async (t) => {
  const manifest = deferred();
  const f = await shell(t, { visitor: false, noWebTransport: true, unixNow: 7000, scheduleFetch: () => manifest.promise });
  const video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0);
  assert.equal(f.harness.progress.at(-1).completed, 14);
  assert.equal(f.harness.scheduleFetches.length, 1); assert.deepEqual(video.sources, []);
  await f.advance(9000);
  assert.equal(f.elements.get("reload-scene").hidden, true); assert.deepEqual(video.loads, []);
  f.setUnixNow(75000);
  manifest.resolve(new Response(JSON.stringify({
    version: 1, revision: "late", anchorUnixSeconds: 0, cycleSeconds: 120, driftToleranceSeconds: 5,
    entries: [
      { id: "first", title: "First", startSeconds: 0, durationSeconds: 60, source: "/first.mp4", sourceStartSeconds: 0, loop: false },
      { id: "second", title: "Second", startSeconds: 60, durationSeconds: 60, source: "/second.mp4", sourceStartSeconds: 10, loop: false },
    ],
  }), { headers: { "content-type": "application/json" } }));
  await flush();
  assert.deepEqual(video.sources, ["/second.mp4"]); assert.equal(video.playCount, 0);
  assert.deepEqual(f.harness.npcCycles, [[0, 120]]);
  f.metadata(); await flush();
  assert.equal(video.currentTime, 25); assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  f.dismissStartup();
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
});

test("startup Play accepts metadata, stays synchronous, and waits for observed sound plus a frame", async (t) => {
  const f = await shell(t, { visitor: false, presentation: "full" });
  const video = f.elements.get("theater-video"), dialog = f.elements.get("theater-startup");
  const play = f.elements.get("theater-startup-play");
  assert.equal(dialog.hidden, true);
  globalThis.weaverSceneReady();
  assert.equal(dialog.hidden, false); assert.equal(f.harness.progress.at(-1).completed, 14);
  assert.equal(f.harness.scheduleFetches.length, 0);
  await f.advance(0);
  video.readyState = 1; video.dispatchEvent(new Event("loadedmetadata"));
  await flush();
  assert.equal(dialog.dataset.state, "ready"); assert.equal(play.disabled, false);
  assert.equal(video.playCount, 1); assert.equal(video.muted, true); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  await flush(); // Settle the independent silent autoplay before the sound gesture.
  let gesture = false;
  video.play = () => {
    assert.equal(gesture, true); assert.equal(video.muted, false);
    video.playCount++; video.paused = false;
    return Promise.resolve(); // Neither a playing notification nor a decoded frame.
  };
  f.setUnixNow(17000);
  play.dispatchEvent(new Event("click")); assert.equal(video.playCount, 1);
  gesture = true; trustedClick(play); gesture = false;
  assert.equal(video.playCount, 2); assert.equal(video.currentTime, 17);
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "starting");
  await flush(); assert.equal(dialog.hidden, false);
  video.dispatchEvent(new Event("playing")); assert.equal(dialog.hidden, false);
  video.readyState = 2; video.dispatchEvent(new Event("loadeddata"));
  assert.equal(dialog.hidden, true); assert.equal(f.elements.get("scene").inert, false);
  assert.equal(f.harness.chatFocus.at(-1), false);
  globalThis.weaverSceneReady(); assert.equal(dialog.hidden, true);
});

test("startup Retry reloads only on failure or an explicit slow retry and bounds repeated attempts", async (t) => {
  let available = false;
  const f = await shell(t, { visitor: false, scheduleFetch: () => available ? scheduleResponse() : jsonResponse() });
  const dialog = f.elements.get("theater-startup"), retry = f.elements.get("theater-startup-retry");
  const video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0);
  assert.equal(dialog.dataset.state, "failed"); assert.equal(retry.hidden, false);
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 4);
  assert.deepEqual(video.sources, []); assert.equal(f.elements.get("theater-startup-continue").disabled, false);
  available = true; trustedClick(retry);
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 5);
  trustedClick(retry); await flush();
  assert.equal(f.harness.scheduleFetches.length, 2); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  assert.equal(dialog.dataset.state, "preparing");
  await f.advance(12_000);
  assert.equal(dialog.dataset.state, "slow"); assert.equal(retry.hidden, false);
  trustedClick(retry); await flush();
  assert.equal(f.harness.scheduleFetches.length, 2);
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  assert.equal(dialog.dataset.state, "preparing"); assert.equal(retry.hidden, true);
  trustedClick(retry); trustedClick(retry); await flush();
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  await f.advance(11_999); assert.equal(dialog.dataset.state, "preparing");
  await f.advance(1); assert.equal(dialog.dataset.state, "slow");
  video.error = { code: 4 }; video.dispatchEvent(new Event("error"));
  assert.equal(dialog.dataset.state, "failed");
  video.error = null; trustedClick(retry); await flush();
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO, SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  f.metadata(); await flush(); assert.equal(dialog.dataset.state, "ready"); assert.equal(video.playCount, 1);
  video.pause(); // Exercise a blocked restart, not an already-observed muted playback unmute.
  video.playError = new Error("NotAllowedError");
  trustedClick(f.elements.get("theater-startup-play")); await flush();
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "failed");
  video.playError = null; trustedClick(retry);
  assert.equal(video.playCount, 4); assert.equal(video.muted, false);
  assert.equal(dialog.hidden, true);
});

test("Continue while preparing starts only the current and later selected UTC clips muted", async (t) => {
  const f = await shell(t, { visitor: false, scheduleFetch: () => new Response(JSON.stringify({
    version: 1, revision: "two", anchorUnixSeconds: 0, cycleSeconds: 20, driftToleranceSeconds: 5,
    entries: [
      { id: "first", title: "First", startSeconds: 0, durationSeconds: 10, source: "/first.mp4", sourceStartSeconds: 0, loop: false },
      { id: "second", title: "Second", startSeconds: 10, durationSeconds: 10, source: "/second.mp4", sourceStartSeconds: 0, loop: false },
    ],
  }), { headers: { "content-type": "application/json" } }) });
  const dialog = f.elements.get("theater-startup"), video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); f.dismissStartup();
  assert.equal(dialog.hidden, true); assert.equal(video.muted, true); assert.equal(video.playCount, 0);
  await f.advance(0);
  assert.deepEqual(video.sources, ["/first.mp4"]);
  f.metadata(); await flush(); assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  f.setUnixNow(12_000); await f.advance(1000);
  assert.deepEqual(video.sources, ["/first.mp4", "/second.mp4"]); assert.equal(video.playCount, 1);
  f.metadata(); await flush();
  assert.equal(video.playCount, 2); assert.equal(video.muted, true); assert.equal(dialog.hidden, true);
});

test("World Sound reopens preparation and retries a failed current source instead of silently returning", async (t) => {
  const f = await shell(t, { visitor: false });
  const dialog = f.elements.get("theater-startup"), video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup();
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 5);
  soundPanelClick(f);
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "preparing");
  assert.equal(video.playCount, 0); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  f.dismissStartup(); video.error = { code: 4 }; video.dispatchEvent(new Event("error"));
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 4);
  video.error = null; soundPanelClick(f);
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "preparing");
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  f.metadata(); await flush();
  assert.equal(dialog.hidden, false);
  trustedClick(f.elements.get("theater-startup-play")); await flush();
  assert.equal(dialog.hidden, true); assert.equal(video.muted, false);
});

test("pending Sound reopens confirmation and slow Retry reloads once without replaying audible intent", async (t) => {
  const f = await shell(t, { visitor: false });
  const dialog = f.elements.get("theater-startup"), video = f.elements.get("theater-video");
  const retry = f.elements.get("theater-startup-retry"), pending = deferred();
  video.playPromise = pending.promise;
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata();
  assert.equal(video.playCount, 1); assert.equal(dialog.hidden, true);
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 2);
  soundPanelClick(f);
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "starting");
  assert.equal(video.playCount, 1);
  await f.advance(12_000); assert.equal(dialog.dataset.state, "slow");
  trustedClick(retry);
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  assert.equal(dialog.dataset.state, "starting"); assert.equal(retry.hidden, true);
  trustedClick(retry); assert.equal(video.sources.length, 2);
  video.playPromise = null; pending.resolve(); await flush(); f.metadata(); await flush();
  assert.equal(dialog.hidden, false); assert.equal(dialog.dataset.state, "ready");
  assert.equal(video.playCount, 2); assert.equal(video.muted, true);
  trustedClick(f.elements.get("theater-startup-play"));
  assert.equal(video.playCount, 3); assert.equal(video.muted, false); assert.equal(dialog.hidden, true);
});

test("startup visibility pauses the slow indication and disposal cancels late UI work", async (t) => {
  const f = await shell(t, { visitor: false });
  const dialog = f.elements.get("theater-startup"), video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0);
  await f.advance(6000); await f.visible("hidden"); await f.advance(60_000);
  assert.equal(dialog.hidden, true);
  await f.visible("visible"); assert.equal(dialog.dataset.state, "preparing");
  await f.advance(12_000); assert.equal(dialog.dataset.state, "slow");
  assert.equal(f.elements.get("theater-startup-continue").disabled, false);
  globalThis.weaverDisposeTheaterVideo();
  assert.equal(dialog.hidden, true); assert.equal(f.elements.get("scene").inert, false);
  await f.advance(60_000); await f.visible("hidden"); await f.visible("visible");
  f.metadata(); trustedClick(f.elements.get("theater-startup-play"));
  trustedClick(f.elements.get("theater-startup-retry")); f.dismissStartup();
  assert.equal(dialog.hidden, true); assert.equal(video.playCount, 0);
});

test("startup traps focus, releases held input and pointer lock, blocks chat/game input, and Escape restores canvas", async (t) => {
  let requests = 0;
  const f = await shell(t, { visitor: false, presentation: "full", requestChat: () => { requests++; return true; } });
  const canvas = f.elements.get("scene"), play = f.elements.get("theater-startup-play");
  const dialog = f.elements.get("theater-startup"), continued = f.elements.get("theater-startup-continue");
  const released = [];
  canvas.addEventListener("keyup", (event) => released.push(event.code));
  canvas.focus(); key(f, "keydown", "w"); f.document.pointerLockElement = canvas;
  globalThis.weaverSceneReady();
  assert.deepEqual(released, ["KeyW"]); assert.equal(f.document.pointerLockExits, 1);
  assert.equal(f.document.activeElement, continued); assert.equal(canvas.inert, true);
  assert.equal(f.harness.chatFocus.at(-1), true); assert.equal(globalThis.weaverResumeGameInput(), false);
  globalThis.weaverSetChatFocus(true); globalThis.weaverSetChatFocus(false);
  assert.equal(f.harness.chatFocus.at(-1), true);
  let nativeInput = 0;
  f.window.addEventListener("keydown", () => nativeInput++);
  f.window.addEventListener("keyup", () => nativeInput++);
  f.window.addEventListener("beforeinput", () => nativeInput++);
  for (const pressed of ["w", "e", "Enter"]) assert.equal(key(f, "keydown", pressed).defaultPrevented, true);
  f.window.dispatchEvent(new Event("beforeinput", { cancelable: true }));
  assert.equal(nativeInput, 0); assert.equal(requests, 0);
  await f.advance(0); f.metadata(); await flush();
  key(f, "keydown", "Tab"); assert.equal(f.document.activeElement, play);
  key(f, "keydown", "Tab", { shiftKey: true }); assert.equal(f.document.activeElement, continued);
  for (const type of ["keydown", "keyup"]) {
    for (const pressed of ["Enter", " "]) {
      assert.equal(key(f, type, pressed, { target: continued }).defaultPrevented, false, `${pressed} ${type} retains native activation`);
    }
  }
  assert.equal(nativeInput, 0);
  key(f, "keydown", "Escape");
  assert.equal(dialog.hidden, true); assert.equal(f.document.activeElement, canvas);
  assert.equal(canvas.inert, false); assert.equal(f.harness.chatFocus.at(-1), false);
  assert.equal(key(f, "keydown", "w", { repeat: true }).defaultPrevented, true);
  key(f, "keydown", "w"); assert.equal(nativeInput, 1);
});

test("stalled metadata and unavailable media never withhold ready, time out the scene, or load fallback", async (t) => {
  const f = await shell(t);
  const video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0);
  assert.equal(f.harness.progress.at(-1).stage, "scene"); assert.equal(video.readyState, 0);
  await f.advance(25000);
  assert.equal(video.playCount, 0); assert.deepEqual(f.harness.videoEvents, []);
  assert.equal(f.elements.get("reload-scene").hidden, true);
  assert.equal(f.elements.get("status").classes.has("failed"), false);
  assert.match(f.elements.get("network-status").textContent, /Online/);
  video.error = { code: 4 }; video.dispatchEvent(new Event("error"));
  await f.advance(1000);
  assert.match(f.elements.get("theater-status").textContent, /media error 4/);
  assert.deepEqual(video.sources, [SCHEDULED_VIDEO]); assert.equal(video.src, "");
  assert.equal(f.elements.get("reload-scene").hidden, true); assert.equal(f.client.closed(), false);
});

test("a pending autoplay promise cannot block ready and cannot revive disposed media", async (t) => {
  const f = await shell(t, { visitor: false });
  const video = f.elements.get("theater-video"), play = deferred();
  video.playPromise = play.promise;
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata();
  assert.equal(video.playCount, 1); assert.equal(f.harness.progress.at(-1).completed, 14);
  await f.advance(25000);
  assert.equal(f.elements.get("reload-scene").hidden, true);
  assert.equal(f.elements.get("status").classes.has("failed"), false);
  globalThis.weaverDisposeTheaterVideo(); play.resolve(); await flush();
  assert.equal(video.paused, true); assert.equal(video.src, ""); assert.equal(video.playCount, 1);
});

for (const queued of [false, true]) {
  for (const teardown of ["dispose", "fatal"]) {
    test(`${teardown} ${queued ? "after ready before its task" : "before ready"} prevents all theater requests`, async (t) => {
      const f = await shell(t, { visitor: false, noWebTransport: true });
      const video = f.elements.get("theater-video");
      if (queued) globalThis.weaverSceneReady();
      if (teardown === "fatal") globalThis.weaverSceneFatal("fixture shutdown");
      else globalThis.weaverDisposeTheaterVideo();
      globalThis.weaverSceneReady();
      await f.visible("hidden"); await f.visible("visible"); f.metadata(); await f.advance(25000);
      assert.equal(f.harness.scheduleFetches.length, 0);
      assert.deepEqual(video.sources, []); assert.deepEqual(video.loads, []);
      assert.deepEqual(f.harness.videoEvents, []); assert.equal(video.playCount, 0); assert.equal(video.removed, true);
    });
  }
}

for (const initiallyHidden of [false, true]) {
  test(`${initiallyHidden ? "initially hidden scene" : "hide before the deferred task"} postpones loading and restore still requires Play`, async (t) => {
    const f = await shell(t, { visitor: false, visibility: initiallyHidden ? "hidden" : "visible" });
    const video = f.elements.get("theater-video");
    globalThis.weaverSceneReady();
    if (!initiallyHidden) await f.visible("hidden");
    await f.advance(0);
    assert.equal(f.harness.scheduleFetches.length, 0); assert.deepEqual(video.sources, []);
    await f.visible("visible");
    assert.equal(f.harness.scheduleFetches.length, 0);
    await f.advance(0); f.metadata(); await flush();
    assert.equal(f.harness.scheduleFetches.length, 1); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
    assert.equal(video.playCount, 0);
    f.elements.get("theater-play").dispatchEvent(new Event("click")); await flush();
    assert.equal(video.playCount, 1);
  });
}

test("shell onRecover wiring realigns and replays a watchdog decoder reload while schedule polling continues without video callbacks", async (t) => {
  const f = await shell(t, { visitor: false, unixNow: 123_000, videoFrames: true });
  const video = f.elements.get("theater-video"), starts = [];
  const load = video.load.bind(video), play = video.play.bind(video);
  video.load = () => { load(); video.currentTime = 0; video.paused = true; };
  video.play = () => { starts.push({ source: video.src, time: video.currentTime }); return play(); };
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  assert.equal(f.videoFrames.size, 1); assert.equal(video.playCount, 1);
  for (let second = 1; second <= 16; second++) {
    f.setUnixNow((123 + second) * 1000); await f.advance(1000);
    video.dispatchEvent(new Event("timeupdate"));
    if (second === 8) { assert.equal(video.playCount, 2); assert.equal(video.loads.length, 1); }
  }
  assert.deepEqual(video.loads, [SCHEDULED_VIDEO, SCHEDULED_VIDEO]);
  assert.equal(video.readyState, 0); assert.equal(video.playCount, 2); assert.equal(f.videoFrames.size, 0);
  assert.equal(f.elements.get("theater-play").disabled, true);
  assert.match(f.elements.get("theater-schedule-status").textContent, /waiting for media/);
  f.setUnixNow(140_000); f.metadata(); await flush();
  assert.deepEqual(starts, [{ source: SCHEDULED_VIDEO, time: 123 }, { source: SCHEDULED_VIDEO, time: 129 }, { source: SCHEDULED_VIDEO, time: 140 }]);
  assert.equal(video.paused, false); assert.equal(video.muted, true); assert.equal(f.videoFrames.size, 1);
  assert.equal(f.harness.scheduleFetches.length, 1); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  assert.equal(f.elements.get("theater-startup").hidden, true);
});

test("established playback resumes once across hidden/pagehide/BFCache events without network rejoin or input recapture", async (t) => {
  const f = await shell(t, { unixNow: 9000 });
  const video = f.elements.get("theater-video"), canvas = f.elements.get("scene");
  let locks = 0; canvas.requestPointerLock = () => { locks++; };
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  assert.equal(video.playCount, 1); assert.equal(video.currentTime, 9);
  await f.visible("hidden"); f.window.dispatchEvent(new Event("pagehide")); await f.advance(10_000);
  assert.equal(video.paused, true); assert.equal(video.playCount, 1); assert.equal(f.rafs.size, 0);
  f.setUnixNow(17_000); f.document.visibilityState = "visible";
  const restored = new Event("pageshow"); Object.defineProperty(restored, "persisted", { value: true });
  f.window.dispatchEvent(restored); await flush(); await f.visible("visible");
  assert.equal(video.playCount, 2); assert.equal(video.paused, false); assert.equal(video.currentTime, 17);
  assert.equal(video.muted, true); assert.equal(f.rafs.size, 1);
  assert.equal(f.harness.scheduleFetches.length, 1); assert.deepEqual(video.sources, [SCHEDULED_VIDEO]);
  assert.equal(f.client.closed(), true); assert.equal(f.clients.length, 1); assert.equal(f.harness.fetches.length, 1);
  assert.equal(locks, 0); assert.equal(f.elements.get("theater-startup").hidden, true);
  f.elements.get("theater-play").dispatchEvent(new Event("click"));
  await f.visible("hidden"); f.window.dispatchEvent(new Event("pagehide")); await f.visible("visible");
  assert.equal(video.paused, true); assert.equal(video.playCount, 2);
  globalThis.weaverDisposeTheaterVideo(); await f.visible("hidden"); await f.visible("visible");
  f.metadata(); await f.advance(30_000); assert.equal(video.playCount, 2); assert.equal(video.src, "");
});

test("visibility restoration waits for asynchronous live alignment before resuming muted playback", async (t) => {
  const f = await shell(t, { visitor: false, unixNow: 12_000 });
  const video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  await f.visible("hidden"); f.setUnixNow(14_000);
  let position = video.currentTime;
  Object.defineProperty(video, "currentTime", { configurable: true, get: () => position,
    set: (value) => { position = value; video.seeking = true; video.dispatchEvent(new Event("seeking")); } });
  await f.visible("visible");
  assert.equal(video.currentTime, 14); assert.equal(video.seeking, true);
  assert.equal(video.playCount, 1); assert.equal(video.paused, true);
  video.seeking = false; video.dispatchEvent(new Event("seeked")); await flush();
  assert.equal(video.playCount, 2); assert.equal(video.paused, false); assert.equal(video.muted, true);
  assert.equal(f.harness.scheduleFetches.length, 1);
});

test("explicit local selection before ready remains lazy, honors its configured source, and can rejoin the schedule", async (t) => {
  const f = await shell(t, { visitor: false, localSource: "/diagnostics.mp4", unixNow: 17000 });
  const video = f.elements.get("theater-video"), follow = f.elements.get("theater-follow-schedule");
  follow.checked = false; follow.dispatchEvent(new Event("change"));
  await f.advance(0);
  assert.equal(f.harness.scheduleFetches.length, 0); assert.deepEqual(video.sources, []);
  globalThis.weaverSceneReady(); assert.deepEqual(video.sources, []);
  await f.advance(0); f.metadata(); await flush();
  assert.deepEqual(video.sources, ["/diagnostics.mp4"]); assert.equal(video.playCount, 0);
  follow.checked = true; follow.dispatchEvent(new Event("change"));
  assert.deepEqual(video.sources, ["/diagnostics.mp4", SCHEDULED_VIDEO]);
  f.metadata(); await flush(); assert.equal(video.currentTime, 17); assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  f.dismissStartup(); assert.equal(video.playCount, 1);
});

test("unavailable schedule cannot block ready or load fallback until explicit local diagnostics", async (t) => {
  const f = await shell(t, { scheduleFetch: async () => jsonResponse() });
  const video = f.elements.get("theater-video");
  globalThis.weaverSceneReady();
  assert.equal(f.harness.progress.at(-1).stage, "scene");
  assert.deepEqual(video.sources, []); assert.equal(f.harness.scheduleFetches.length, 0);
  await f.advance(0);
  assert.match(f.elements.get("theater-schedule-status").textContent, /Schedule unavailable/);
  assert.equal(video.playCount, 0); assert.equal(f.elements.get("theater-follow-schedule").checked, true);
  assert.equal(f.elements.get("theater-restart").disabled, true);
  assert.equal(f.elements.get("theater-seek").disabled, true);
  assert.equal(f.elements.get("theater-play").disabled, true);
  f.elements.get("theater-play").dispatchEvent(new Event("click")); assert.equal(video.playCount, 0);
  assert.deepEqual(video.sources, []);
  const follow = f.elements.get("theater-follow-schedule"); follow.checked = false; follow.dispatchEvent(new Event("change"));
  assert.deepEqual(video.sources, [LOCAL_VIDEO]); f.metadata();
  assert.match(f.elements.get("theater-schedule-status").textContent, /not synchronized/);
  assert.equal(f.elements.get("theater-restart").disabled, false);
  assert.equal(f.elements.get("theater-seek").disabled, false);
  f.elements.get("theater-play").dispatchEvent(new Event("click")); await flush(); assert.equal(video.playCount, 1);
  assert.equal(f.client.closed(), false);
});

test("movie attachment waits for metadata plus initialized renderer; controls pause without disconnect", async (t) => {
  const f = await shell(t);
  const video = f.elements.get("theater-video");
  video.readyState = 2;
  video.dispatchEvent(new Event("loadedmetadata"));
  assert.deepEqual(f.harness.videoEvents, []);
  globalThis.weaverSceneReady(); await f.advance(0);
  assert.deepEqual(f.harness.videoEvents, []);
  f.dismissStartup(); f.metadata(); await flush();
  assert.deepEqual(f.harness.videoEvents.map(([event]) => event), ["attach", "frame"]);
  globalThis.weaverSceneReady();
  assert.equal(f.harness.videoEvents.filter(([event]) => event === "attach").length, 1);
  assert.equal(video.playCount, 1);
  assert.equal(video.autoplay, false);
  assert.equal(video.src, SCHEDULED_VIDEO);
  assert.equal(f.elements.get("theater-time").textContent, "0:00 / 66:21");
  assert.equal(video.paused, false); assert.equal(f.rafs.size, 1);
  assert.equal(video.muted, true); assert.equal(video.volume, 0.4);
  assert.equal(f.elements.get("theater-mute").textContent, "Unmute");
  assert.match(f.elements.get("theater-status").textContent, /muted/);
  f.elements.get("theater-play").dispatchEvent(new Event("click"));
  assert.equal(video.paused, true); assert.equal(f.rafs.size, 0);
  assert.equal(f.client.closed(), false);
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(video.paused, true);
  globalThis.weaverDisposeTheaterVideo(); globalThis.weaverDisposeTheaterVideo();
  assert.equal(video.src, ""); assert.equal(video.removed, true);
  assert.equal(f.harness.videoEvents.filter(([event]) => event === "detach").length, 1);
});

test("one combined picker dispatches offline seats and preserves synchronous, exclusive wall controls", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true, presentation: "full" });
  f.harness.panelHit = 100;
  scenePointer(f, "pointerdown");
  assert.deepEqual(f.harness.seatTargets, [], "renderer readiness is required");
  globalThis.weaverSceneReady();
  scenePointer(f, "pointerdown");
  assert.deepEqual(f.harness.seatTargets, [], "startup confirmation owns input");
  f.dismissStartup();
  for (let target = 100; target <= 123; target++) {
    f.harness.panelHit = target;
    assert.equal(scenePointer(f, "pointerdown").defaultPrevented, true);
    scenePointer(f, "pointerup");
    scenePointer(f, "click");
  }
  assert.deepEqual(f.harness.seatTargets, Array.from({ length: 24 }, (_, index) => 100 + index));
  assert.equal(f.harness.connects.length, 0);
  let fullscreen = 0, gesture = false;
  f.window.jdharrisonGetFullscreenState = () => ({ available: true, active: false, pending: false, error: false });
  f.window.jdharrisonToggleFullscreen = () => { assert.ok(gesture); fullscreen++; };
  f.harness.panelHit = 2;
  gesture = true; scenePointer(f, "pointerdown"); gesture = false;
  scenePointer(f, "pointerup"); scenePointer(f, "click");
  assert.equal(fullscreen, 1);
  f.harness.panelHit = 3; scenePointer(f, "pointerdown"); scenePointer(f, "pointerup");
  f.harness.panelHit = 1; soundPanelClick(f);
  assert.equal(f.elements.get("theater-startup").hidden, false);
  assert.deepEqual(f.harness.seatTargets, Array.from({ length: 24 }, (_, index) => 100 + index));
  assert.equal(f.harness.legacyPanelPicks, 0, "no second panel-only pick or fallback dispatcher");
});

test("exit target is capture-only for an unlocked mouse and activates once for a locked mouse or touch", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true });
  globalThis.weaverSceneReady(); f.dismissStartup();
  f.harness.panelHit = 200;
  for (const type of ["pointerdown", "pointerup", "mousedown", "mouseup", "click"]) {
    assert.equal(scenePointer(f, type).defaultPrevented, false);
  }
  assert.deepEqual(f.harness.seatTargets, []);
  f.document.pointerLockElement = f.elements.get("scene");
  f.document.dispatchEvent(new Event("pointerlockchange"));
  assert.equal(scenePointer(f, "pointerdown", { clientX: 10, clientY: 10 }).defaultPrevented, true);
  scenePointer(f, "pointerup"); scenePointer(f, "click");
  assert.deepEqual(f.harness.panelPicks.at(-1), [0, 0]);
  assert.deepEqual(f.harness.seatTargets, [200]);
  f.document.pointerLockElement = null;
  f.document.dispatchEvent(new Event("pointerlockchange"));
  const touch = { pointerType: "touch" };
  assert.equal(scenePointer(f, "pointerdown", touch).defaultPrevented, false);
  assert.equal(scenePointer(f, "pointerup", { ...touch, timeStamp: 100 }).defaultPrevented, false);
  scenePointer(f, "click");
  assert.deepEqual(f.harness.seatTargets, [200, 200]);
});

test("seat rejection, invalid targets and unavailable interaction bindings never activate a fallback", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true });
  globalThis.weaverSceneReady(); f.dismissStartup();
  f.harness.panelHit = 100; f.harness.seatAccept = false;
  assert.equal(scenePointer(f, "pointerdown").defaultPrevented, true);
  scenePointer(f, "pointerup"); scenePointer(f, "click");
  assert.deepEqual(f.harness.seatTargets, [100]);
  assert.equal(f.harness.pendingSeatTarget, null);
  assert.equal(f.elements.get("theater-startup").hidden, true);
  for (const target of [99, 124, 201, NaN]) {
    f.harness.panelHit = target;
    assert.equal(scenePointer(f, "pointerdown").defaultPrevented, false);
  }
  f.harness.interactionError = true;
  assert.equal(scenePointer(f, "pointerdown").defaultPrevented, false);
  assert.equal(f.harness.legacyPanelPicks, 0);
  assert.deepEqual(f.harness.seatTargets, [100]);
});

test("focus and viewport/capture phase changes clear both seat tap candidates and queued Rust intent", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true, presentation: "minimal" });
  globalThis.weaverSceneReady(); f.dismissStartup(); f.harness.panelHit = 100;
  const touch = { pointerType: "touch" };
  for (const interrupt of [
    () => { globalThis.weaverSetChatFocus(true); globalThis.weaverSetChatFocus(false); },
    () => { const panel = f.elements.get("theater-controls"); panel.dispatchEvent(new Event("focusin")); panel.dispatchEvent(new Event("focusout")); },
    () => f.window.dispatchEvent(new Event("blur")),
    () => f.window.dispatchEvent(new Event("resize")),
    () => f.window.dispatchEvent(new Event("orientationchange")),
    () => f.document.dispatchEvent(new Event("pointerlockchange")),
  ]) {
    const count = f.harness.seatTargets.length, invalidations = f.harness.sceneInvalidations;
    scenePointer(f, "pointerdown", touch);
    interrupt();
    scenePointer(f, "pointerup", { ...touch, timeStamp: 100 });
    assert.equal(f.harness.seatTargets.length, count);
    assert.ok(f.harness.sceneInvalidations > invalidations);
    scenePointer(f, "pointerdown"); scenePointer(f, "pointerup");
    assert.equal(f.harness.pendingSeatTarget, 100);
    interrupt();
    assert.equal(f.harness.pendingSeatTarget, null);
  }
  const editable = f.elements.get("display-name");
  editable.tagName = "INPUT"; editable.focus();
  const count = f.harness.seatTargets.length;
  assert.equal(scenePointer(f, "pointerdown").defaultPrevented, false);
  assert.equal(f.harness.seatTargets.length, count);
});

test("hidden/pagehide, fatal and shutdown phases never replay an old seat gesture", async (t) => {
  const f = await shell(t, { visitor: false, noWebTransport: true });
  globalThis.weaverSceneReady(); f.dismissStartup(); f.harness.panelHit = 100;
  const touch = { pointerType: "touch" };
  scenePointer(f, "pointerdown", touch);
  await f.visible("hidden"); await f.visible("visible");
  scenePointer(f, "pointerup", { ...touch, timeStamp: 100 });
  assert.deepEqual(f.harness.seatTargets, []);
  scenePointer(f, "pointerdown", touch);
  f.window.dispatchEvent(new Event("pagehide"));
  const restored = new Event("pageshow"); Object.defineProperty(restored, "persisted", { value: true });
  f.window.dispatchEvent(restored);
  scenePointer(f, "pointerup", { ...touch, timeStamp: 100 });
  assert.deepEqual(f.harness.seatTargets, []);
  scenePointer(f, "pointerdown"); scenePointer(f, "pointerup");
  assert.equal(f.harness.pendingSeatTarget, 100, "fresh input works after a nonpermanent reset");
  globalThis.weaverSceneFatal("fixture fatal");
  assert.equal(f.harness.pendingSeatTarget, null);
  assert.equal(f.harness.panelInvalidations, 1);
  const count = f.harness.seatTargets.length;
  globalThis.weaverSceneReady(); globalThis.weaverDisposeTheaterVideo();
  scenePointer(f, "pointerdown");
  assert.equal(f.harness.seatTargets.length, count);
  assert.equal(f.harness.panelInvalidations, 1);
});

test("sound panel invokes schedule audible play synchronously, observes state and invalidates on shutdown", async (t) => {
  const f = await shell(t);
  const video = f.elements.get("theater-video");
  const activate = () => {
    const event = new Event("pointerdown", { cancelable: true });
    Object.defineProperties(event, { isTrusted: { value: true }, target: { value: f.elements.get("scene") } });
    Object.assign(event, { button: 0, pointerId: 1, pointerType: "mouse", clientX: 100, clientY: 50 });
    f.window.dispatchEvent(event);
    assert.equal(event.defaultPrevented, true);
  };
  let gesture = false;
  const play = video.play.bind(video);
  video.play = () => {
    assert.ok(video.muted || gesture, "audible autoplay denied");
    return play();
  };
  video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 2);
  video.readyState = 3; video.dispatchEvent(new Event("canplay"));
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 1);
  gesture = true; activate(); gesture = false;
  assert.equal(video.playCount, 2); assert.equal(video.muted, false);
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 2);
  await flush();
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 3);
  f.elements.get("theater-mute").dispatchEvent(new Event("click"));
  assert.equal(f.harness.panelStates.filter(([panel]) => panel === 1).at(-1)[1], 1);
  f.elements.get("theater-play").dispatchEvent(new Event("click"));
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(video.playCount, 2); assert.equal(video.paused, true);
  gesture = true; activate(); gesture = false;
  assert.equal(video.playCount, 3); assert.equal(video.paused, false);
  await flush(); globalThis.weaverDisposeTheaterVideo();
  globalThis.weaverDisposeTheaterVideo();
  assert.equal(f.harness.panelInvalidations, 1);
});

test("shell surfaces blocked muted autoplay and permits one manual Play retry", async (t) => {
  const f = await shell(t);
  const video = f.elements.get("theater-video");
  video.playError = new Error("NotAllowedError");
  video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  const play = f.elements.get("theater-play");
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  assert.equal(play.disabled, false); assert.equal(play.textContent, "Play");
  assert.equal(f.elements.get("theater-status").textContent, "Autoplay blocked/unavailable · press Play");
  globalThis.weaverSceneReady();
  for (const event of ["loadedmetadata", "loadeddata", "canplay", "timeupdate"]) video.dispatchEvent(new Event(event));
  await flush(); assert.equal(video.playCount, 1);
  video.playError = null;
  await f.visible("hidden"); await f.visible("visible"); await f.advance(1000);
  assert.equal(video.playCount, 1); assert.equal(video.paused, true);
  play.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.playCount, 2); assert.equal(video.paused, false); assert.equal(video.muted, true);
  assert.equal(play.textContent, "Pause");
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(video.playCount, 3); assert.equal(video.paused, false);
  globalThis.weaverDisposeTheaterVideo();
});

test("movie audio controls synchronize external changes and preserve explicit mute across lifecycle pauses", async (t) => {
  const f = await shell(t);
  const video = f.elements.get("theater-video");
  const mute = f.elements.get("theater-mute"), volume = f.elements.get("theater-volume");
  const play = f.elements.get("theater-play");
  assert.equal(video.muted, true);
  assert.equal(volume.value, "0.4");
  assert.equal(mute.disabled, true);
  video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
  globalThis.weaverSceneReady(); await f.advance(0); f.dismissStartup(); f.metadata(); await flush();
  assert.equal(mute.disabled, false); assert.equal(volume.disabled, false);
  assert.equal(video.playCount, 1);
  mute.dispatchEvent(new Event("click"));
  assert.equal(video.muted, false);
  mute.dispatchEvent(new Event("click"));
  assert.equal(video.muted, true); assert.equal(mute.textContent, "Unmute");
  volume.value = "0.25"; volume.dispatchEvent(new Event("input"));
  assert.equal(video.volume, 0.25); assert.equal(video.muted, true);
  play.dispatchEvent(new Event("click")); play.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.muted, true); assert.equal(video.paused, false);
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(video.paused, false); assert.equal(video.volume, 0.25);
  assert.equal(video.playCount, 3); assert.equal(video.muted, true);
  mute.dispatchEvent(new Event("click"));
  assert.equal(video.paused, false); assert.equal(video.muted, false);
  video.volume = 0.6; video.muted = true;
  assert.equal(volume.value, "0.6"); assert.equal(mute.textContent, "Unmute");
  assert.match(f.elements.get("theater-status").textContent, /Playing · muted/);
  video.muted = false;
  assert.equal(mute.textContent, "Mute");
  assert.match(f.elements.get("theater-status").textContent, /audio enabled/);
  f.window.dispatchEvent(new Event("pagehide"));
  assert.equal(video.paused, true);
  globalThis.weaverDisposeTheaterVideo();
  assert.equal(video.paused, true); assert.equal(video.src, "");
  assert.equal(mute.disabled, true); assert.equal(volume.disabled, true);
  const status = f.elements.get("theater-status").textContent;
  video.volume = 0.9; video.muted = true;
  assert.equal(f.elements.get("theater-status").textContent, status);
  assert.equal(volume.value, "0.6");
});

test("early metadata and synchronous ready cannot attach until WASM startup completes", async (t) => {
  let duringInit;
  const f = await shell(t, { readyOnStart: true, init: async (harness) => {
    const video = document.getElementById("theater-video");
    video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
    globalThis.weaverSceneReady();
    assert.equal(document.getElementById("theater-startup").hidden, true);
    duringInit = [...harness.videoEvents];
  } });
  assert.deepEqual(duringInit, []);
  assert.equal(f.elements.get("theater-startup").hidden, false);
  assert.equal(f.elements.get("theater-startup").dataset.state, "preparing");
  assert.deepEqual(f.harness.videoEvents, []); assert.equal(f.harness.scheduleFetches.length, 0);
  await f.advance(0);
  assert.deepEqual(f.elements.get("theater-video").sources, [SCHEDULED_VIDEO]);
  assert.deepEqual(f.harness.videoEvents, []);
  f.metadata();
  assert.deepEqual(f.harness.videoEvents.map(([event]) => event), ["attach", "frame"]);
});

test("theater focus composes with host chat focus without reviving game input", async (t) => {
  const f = await shell(t, { presentation: "minimal" }); globalThis.weaverSceneReady(); f.dismissStartup(); await flush();
  const panel = f.elements.get("theater-controls");
  panel.dispatchEvent(new Event("focusin"));
  assert.equal(f.harness.chatFocus.at(-1), true);
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  panel.dispatchEvent(new Event("focusout"));
  assert.equal(f.harness.chatFocus.at(-1), true);
  assert.equal(globalThis.weaverSetChatFocus(false), true);
  assert.equal(f.harness.chatFocus.at(-1), false);
  panel.dispatchEvent(new Event("focusin"));
  globalThis.weaverSetChatFocus(false);
  assert.equal(f.harness.chatFocus.at(-1), true);
  assert.equal(globalThis.weaverResumeGameInput(), false);
});

test("fatal scene disposes video and a late ready cannot reattach", async (t) => {
  const f = await shell(t); const video = f.elements.get("theater-video");
  video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
  globalThis.weaverSceneReady(); await f.advance(0); f.metadata(); await flush();
  globalThis.weaverSceneFatal("fixture graphics failure");
  globalThis.weaverSceneReady(); video.dispatchEvent(new Event("loadedmetadata"));
  assert.deepEqual(f.harness.videoEvents.map(([event]) => event), ["attach", "frame", "detach"]);
  assert.equal(video.src, ""); assert.equal(video.removed, true);
});

test("shell waits for renderer readiness and keeps anonymous credentials out of DOM/storage", async (t) => {
  const f = await shell(t);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.equal(f.harness.fetches.length, 0);
  assert.equal(f.harness.connects.length, 0);
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 1);
  assert.equal(f.harness.connects.length, 1);
  assert.equal(f.elements.get("woven-token").value, "");
  assert.equal([...f.storage.values()].some((value) => value.includes(TOKEN)), false);
  assert.match(f.elements.get("network-status").textContent, /Online/);
});

test("shell drops malformed and oversize reliable messages without losing the healthy connection", async (t) => {
  const f = await shell(t);
  globalThis.weaverSceneReady();
  await flush();
  const envelope = { namespaceId: 2n, sessionId: 2n, spaceId: 3n, spaceEpoch: 1n,
    channelId: 1n, payloadTypeId: 1n, entityId: 8n, senderSequence: 1n, messageKind: 20 };
  for (const payload of [new Uint8Array([0xff]), new Uint8Array(2049), new TextEncoder().encode('{"kind":"profile"}')]) {
    f.client.send({ ...envelope, payload });
    await flush();
  }
  assert.equal(f.harness.events.filter(([kind]) => kind === "payload").length, 1);
  assert.equal(f.client.closed(), false);
});

// Exercise the actual shell bridge/outboxes, not a reimplementation of the send policy.
function badge(f, label) {
  assert.equal(f.elements.get("connection-label").textContent, label);
  assert.equal(f.elements.get("connection-badge").dataset.state, label.toLowerCase());
}

test("passive badge is outside diagnostics, accessible, safe-area anchored and pointer transparent", async () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  const markup = html.slice(html.indexOf('<div id="connection-badge"'), html.indexOf('<section id="scene-instructions"'));
  assert.match(markup, /role="status" aria-live="polite" aria-atomic="true"/);
  assert.match(markup, /POWERED BY WOVEN.HOST/);
  assert.doesNotMatch(markup, /<(button|a|input)\b|tabindex|hidden>/);
  assert.ok(html.indexOf('id="connection-badge"') < html.indexOf('id="connection-controls"'));
  assert.match(html, /id="subtitle-controls"[^>]* hidden/);
  const rule = css.match(/\.connection-badge \{([^}]+)\}/)[1];
  assert.match(rule, /position: fixed/);
  assert.match(rule, /top:.*env\(safe-area-inset-top\)/);
  assert.match(rule, /right:.*env\(safe-area-inset-right\)/);
  assert.match(rule, /pointer-events: none/);
  assert.match(css, /\.connection-badge \* \{ pointer-events: none; \}/);
  assert.match(css, /\.connection-dot \{ width: 8px; height: 8px/);
  assert.match(css, /\.connection-brand \{[^}]*letter-spacing:[^}]*text-transform: uppercase/s);
  assert.doesNotMatch(css, /html\[data-weaver-presentation="minimal"\][^{]*connection-badge/);
});

test("badge waits through boot, admission and subscription for accepted assigned entity", async (t) => {
  const admission = deferred(), assignment = deferred();
  const f = await shell(t, { connect: (client) => {
    client.admitWithCancellation = () => admission.promise;
    const recv = client.recvTimeout;
    client.recvTimeout = async () => { await assignment.promise; return recv(); };
    return client;
  } });
  badge(f, "Offline");
  globalThis.weaverSceneReady(); await flush();
  badge(f, "Connecting");
  admission.resolve({ kind: "admission", result: { status: 1 } }); await flush();
  badge(f, "Connecting");
  assignment.resolve(); await flush();
  badge(f, "CONNECTED");
  await f.advance(120_000);
  badge(f, "CONNECTED"); // A quiet room is not a transport failure.
});

for (const [name, options, label] of [
  ["unsupported", { noWebTransport: true }, "Unavailable"],
  ["failed setup", { connect: async () => { throw new Error("setup failed"); } }, "Offline"],
  ["rejected realtime inbox", { acceptConnection: false }, "Offline"],
  ["initially offline", { onLine: false }, "Offline"],
]) test(`badge is honest for ${name}`, async (t) => {
  const f = await shell(t, options);
  globalThis.weaverSceneReady(); await flush();
  badge(f, label);
});

for (const loss of ["recv null", "datagram null", "recv error", "publish error", "manual", "hidden", "pagehide", "fatal", "offline"]) {
  test(`badge leaves green on ${loss} and only reconnects explicitly`, async (t) => {
    const f = await online(t);
    badge(f, "CONNECTED");
    if (loss === "recv null") f.client.send(null);
    if (loss === "datagram null") f.client.sendDatagram(null);
    if (loss === "recv error") f.client.close();
    if (loss === "publish error") {
      f.client.publishEvent = async () => { throw new Error("publish closed"); };
      globalThis.weaverRealtimePublishReliable(99n, '{"kind":"chat","text":"test"}');
    }
    if (loss === "manual") f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
    if (loss === "hidden") await f.visible("hidden");
    if (loss === "pagehide") f.window.dispatchEvent(new Event("pagehide"));
    if (loss === "fatal") globalThis.weaverSceneFatal("device lost");
    if (loss === "offline") {
      f.window.navigator.onLine = false;
      f.window.dispatchEvent(new Event("offline"));
      badge(f, "Disconnected");
      f.window.navigator.onLine = true;
      f.window.dispatchEvent(new Event("online"));
    }
    await flush();
    badge(f, loss === "fatal" ? "Unavailable" : "Disconnected");
    assert.equal(f.client.closed(), true);
    if (loss === "hidden") await f.visible("visible");
    if (loss === "pagehide") {
      const event = new Event("pageshow"); Object.defineProperty(event, "persisted", { value: true });
      f.window.dispatchEvent(event);
    }
    assert.equal(f.harness.connects.length, 1);
    if (loss === "fatal") { globalThis.weaverSceneReady(); badge(f, "Unavailable"); return; }
    f.elements.get("join-network").dispatchEvent(new Event("click"));
    badge(f, "Connecting");
    await flush(); badge(f, "CONNECTED");
  });
}

test("offline cancels pending transport and late completion cannot turn the badge green", async (t) => {
  const handshake = deferred();
  const f = await shell(t, { connect: () => handshake.promise });
  globalThis.weaverSceneReady(); await flush(); badge(f, "Connecting");
  f.window.navigator.onLine = false; f.window.dispatchEvent(new Event("offline"));
  badge(f, "Offline");
  f.window.navigator.onLine = true; f.window.dispatchEvent(new Event("online"));
  handshake.resolve(f.client); await flush();
  badge(f, "Offline"); assert.equal(f.client.closed(), true);
  assert.equal(f.harness.connects.length, 1);
});

async function online(t, options = {}) {
  const f = await shell(t, options);
  globalThis.weaverSceneReady();
  await flush();
  assert.match(f.elements.get("network-status").textContent, /Online/);
  return f;
}

function posePayload(version = 1, seat = 0) {
  const payload = new Uint8Array(version === 1 ? 25 : 26);
  payload[0] = version;
  if (version === 2) payload[1] = seat;
  const view = new DataView(payload.buffer);
  [1, 2, 3, 0, 0, -1].forEach((value, index) => view.setFloat32(version + index * 4, value, true));
  return payload;
}

function pose(sequence, payload = posePayload()) {
  globalThis.weaverRealtimePublishPositionedUnreliable(BigInt(sequence), 1, 2, 3, payload);
}

function malformedPosePayloads() {
  const unknown = posePayload(); unknown[0] = 3;
  const wrongLegacyLength = new Uint8Array(26); wrongLegacyLength[0] = 1;
  const wrongSeatedLength = new Uint8Array(25); wrongSeatedLength[0] = 2;
  return [null, undefined, [], new ArrayBuffer(25), new Uint8Array(), new Uint8Array(25),
    posePayload().slice(0, 24), unknown, wrongLegacyLength, wrongSeatedLength, posePayload(2, 25), posePayload(2, 255)];
}

test("both pose versions publish unchanged copied bytes on the positioned datagram channel", async (t) => {
  const f = await online(t);
  const payloads = [posePayload(), ...Array.from({ length: 25 }, (_, seat) => posePayload(2, seat))];
  for (const [index, payload] of payloads.entries()) {
    const expected = payload.slice();
    const backing = new Uint8Array(payload.length + 8);
    backing.set(payload, 4);
    const wasmView = backing.subarray(4, 4 + payload.length);
    pose(index + 1, wasmView);
    wasmView.fill(255);
    await flush();
    const published = f.client.publishes.at(-1);
    assert.equal(published.kind, "pose"); assert.equal(published.channel, 4n);
    assert.equal(published.sequence, BigInt(index + 1));
    assert.deepEqual(published.position, { x: 1, y: 2, z: 3 });
    assert.deepEqual(published.payload, expected);
  }
  assert.equal(f.client.publishes.length, payloads.length);
  assert.throws(() => globalThis.weaverRealtimePublishUnreliable(99n, posePayload(2, 1)), /rejects unpositioned/);
});

test("outgoing pose headers reject unknown versions, wrong lengths and invalid seat codes without a fallback", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  pose(1); await flush();
  pose(2, posePayload(2, 8));
  for (const payload of malformedPosePayloads()) {
    assert.throws(() => globalThis.weaverRealtimePublishPositionedUnreliable(3n, 1, 2, 3, payload), /pose payload must be version 1/);
  }
  assert.throws(() => globalThis.weaverRealtimePublishPositionedUnreliable(4n, NaN, 2, 3, posePayload(2)), /finite coordinates/);
  await f.advance(260);
  assert.deepEqual(f.client.publishes.map(call => call.sequence), [1n, 2n]);
  assert.deepEqual(f.client.publishes.at(-1).payload, posePayload(2, 8));
  assert.equal(f.client.closed(), false);
});

test("incoming legacy and seated poses retain scope/delivery gates and forward opaque bytes to Rust", async (t) => {
  const f = await online(t);
  const envelope = { namespaceId: 2n, sessionId: 2n, spaceId: 3n, spaceEpoch: 1n,
    channelId: 4n, payloadTypeId: 1n, entityId: 8n, senderSequence: 1n, messageKind: 21, deliveryClass: 3 };
  for (const payload of malformedPosePayloads()) { f.client.sendDatagram({ ...envelope, payload }); await flush(); }
  for (const invalid of [{ namespaceId: 99n }, { sessionId: 99n }, { spaceId: 99n }, { spaceEpoch: 99n },
    { channelId: 1n }, { payloadTypeId: 2n }, { entityId: null }, { entityId: 7n }, { messageKind: 20 }, { deliveryClass: 1 }]) {
    f.client.sendDatagram({ ...envelope, payload: posePayload(2, 1), ...invalid }); await flush();
  }
  assert.deepEqual(f.harness.events.filter(event => event[0] === "pose"), []);
  const payloads = [posePayload(), ...Array.from({ length: 25 }, (_, seat) => posePayload(2, seat))];
  for (const [index, payload] of payloads.entries()) {
    f.client.sendDatagram({ ...envelope, senderSequence: BigInt(index + 1), payload }); await flush();
  }
  assert.deepEqual(f.harness.events.filter(event => event[0] === "pose"),
    payloads.map((payload, index) => ["pose", 8n, BigInt(index + 1), payload]));
  assert.equal(f.client.closed(), false);
  assert.deepEqual(f.client.publishes, []);
});

function assertSharedSpacing(publishes, spacing = 260) {
  for (let i = 1; i < publishes.length; i += 1) {
    assert.ok(publishes[i].at - publishes[i - 1].at >= spacing, "every lane must share the minimum send spacing");
  }
  for (const first of publishes) {
    assert.ok(publishes.filter((item) => item.at >= first.at && item.at < first.at + 1_000).length <= 4,
      "at most four attempts in any half-open one-second window");
  }
}

test("4 Hz shares one budget between an initial reliable profile, 10 Hz poses, and chat", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  await flush();
  pose(2);
  for (let i = 1; i <= 20; i += 1) {
    await f.advance(100);
    pose(i + 2);
    if (i === 5) globalThis.weaverRealtimePublishReliable(100n, "fixture chat");
    await flush();
  }
  await f.advance(260);
  const calls = f.client.publishes;
  assert.deepEqual(calls.filter((call) => call.kind === "event").map((call) => call.sequence), [1n, 100n]);
  const poses = calls.filter((call) => call.kind === "pose");
  assert.ok(poses.length > 1 && poses.length < 21);
  assert.equal(poses.at(-1).sequence, 22n);
  assert.ok(poses.every((call) => call.channel === 4n));
  assert.ok(calls.filter((call) => call.kind === "event").every((call) => call.channel === 1n));
  assertSharedSpacing(calls);
  assert.equal(f.client.closed(), false);
});

test("paced reliable FIFO keeps latest replacement ordering ahead of the newest copied pose", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  await flush();
  pose(2);
  globalThis.weaverRealtimePublishLatest(3n, "fixture old latest");
  globalThis.weaverRealtimePublishReliable(4n, "fixture chat one");
  globalThis.weaverRealtimePublishLatest(5n, "fixture new latest");
  globalThis.weaverRealtimePublishReliable(6n, "fixture chat two");
  pose(7);
  const payload = posePayload(2, 8);
  pose(8, payload);
  payload.fill(99); // The bridge must not retain a view into caller/WASM memory.
  await f.advance(259);
  assert.equal(f.client.publishes.length, 1);
  await f.advance(1);
  for (let i = 0; i < 3; i += 1) await f.advance(260);
  assert.deepEqual(f.client.publishes.map(({ kind, sequence, at }) => [kind, sequence, at]), [
    ["event", 1n, 0], ["event", 4n, 260], ["event", 5n, 520], ["event", 6n, 780], ["pose", 8n, 1_040],
  ]);
  assert.deepEqual(f.client.publishes.at(-1).payload, posePayload(2, 8));
  assert.deepEqual(f.client.publishes.at(-1).position, { x: 1, y: 2, z: 3 });
  assertSharedSpacing(f.client.publishes);
});

test("paced reliable outbox stays bounded without latest or pose evicting accepted reliable messages", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  globalThis.weaverRealtimePublishReliable(0n, "fixture profile");
  await flush();
  for (let i = 1; i <= 32; i += 1) globalThis.weaverRealtimePublishReliable(BigInt(i), "fixture chat");
  assert.throws(() => globalThis.weaverRealtimePublishReliable(33n, "fixture overflow"),
    /^Error: reliable publish queue exceeded 32 pending messages$/);
  globalThis.weaverRealtimePublishLatest(34n, "fixture dropped latest");
  for (let i = 35; i <= 100; i += 1) pose(i);
  for (let i = 0; i < 34; i += 1) await f.advance(260);
  assert.deepEqual(f.client.publishes.filter((call) => call.kind === "event").map((call) => call.sequence),
    Array.from({ length: 33 }, (_, i) => BigInt(i)));
  assert.deepEqual(f.client.publishes.filter((call) => call.kind === "pose").map((call) => call.sequence), [100n]);
  assertSharedSpacing(f.client.publishes); // Includes the optional log queued while the FIFO drains.
});

test("optional delayed logs consume the same slot before a pending pose", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  await f.advance(2_990);
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  await flush();
  pose(2);
  await f.advance(10);
  assert.equal(f.client.publishes.length, 1);
  await f.advance(249);
  assert.equal(f.client.publishes.length, 1);
  await f.advance(1);
  assert.deepEqual(f.client.publishes.map(({ kind, at }) => [kind, at]), [["event", 2_990], ["log", 3_250]]);
  await f.advance(260);
  assert.deepEqual(f.client.publishes.map(({ kind, at }) => [kind, at]),
    [["event", 2_990], ["log", 3_250], ["pose", 3_510]]);
  assertSharedSpacing(f.client.publishes);
});

test("an absent publish-rate marker preserves unpaced visitor demos", async (t) => {
  const f = await online(t);
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  globalThis.weaverRealtimePublishReliable(2n, "fixture chat");
  globalThis.weaverRealtimePublishLatest(3n, "fixture latest");
  pose(4);
  await flush();
  assert.deepEqual(f.client.publishes.map(({ kind, sequence, at }) => [kind, sequence, at]),
    [["event", 1n, 0], ["event", 2n, 0], ["event", 3n, 0], ["pose", 4n, 0]]);
});

for (const value of ["", "0", "61", "4.5", " 4", "4 ", "04", "4e0", "NaN", "Infinity", "-1", "4\n"]) {
  test("invalid supplied publish-rate fails closed without a fetch, credential, or connection: " + JSON.stringify(value), async (t) => {
    const f = await shell(t, { publishRateHz: value });
    assert.deepEqual(f.harness.bootEvents, expectedBoot(false));
    globalThis.weaverSceneReady();
    await flush();
    f.elements.get("network-form").requestSubmit();
    f.elements.get("join-network").dispatchEvent(new Event("click"));
    await flush();
    assert.equal(f.harness.fetches.length, 0);
    assert.equal(f.harness.connects.length, 0);
    assert.equal(f.elements.get("join-network").disabled, true);
    assert.equal(f.elements.get("connect-network").disabled, true);
    assert.equal(f.elements.get("reload-scene").hidden, true);
    assert.equal(f.elements.get("network-status").textContent,
      "Invalid publish-rate configuration: expected an integer from 1 to 60.");
    await f.visible("hidden");
    await f.visible("visible");
    assert.equal(f.elements.get("join-network").disabled, true);
    assert.equal(f.elements.get("network-status").textContent,
      "Invalid publish-rate configuration: expected an integer from 1 to 60.");
  });
}

for (const trigger of ["disconnect", "pagehide", "hidden", "scene-fatal", "realtime-fatal", "protocol-error"]) {
  test("paced timers and all queued frames are cancelled on " + trigger, async (t) => {
    const f = await online(t, { publishRateHz: "4" });
    globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
    await flush();
    globalThis.weaverRealtimePublishReliable(2n, "fixture chat");
    pose(3);
    if (trigger === "disconnect") f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
    else if (trigger === "pagehide") f.window.dispatchEvent(new Event("pagehide"));
    else if (trigger === "hidden") await f.visible("hidden");
    else if (trigger === "scene-fatal") globalThis.weaverSceneFatal("fixture graphics fatal");
    else if (trigger === "realtime-fatal") globalThis.weaverRealtimeFatal("fixture bridge fatal");
    else f.client.send({ messageKind: 40, control: { code: () => 10, relatedMessageKind: () => 21 } });
    await flush();
    await f.advance(5_000);
    assert.equal(f.client.closed(), true);
    assert.deepEqual(f.client.publishes.map((call) => call.sequence), [1n]);
    if (trigger === "protocol-error") assert.match(f.elements.get("network-status").textContent, /Woven receive failed: Woven reported a protocol error/);
  });
}

test("manual retry owns fresh pacing and cannot drain old timed reliable or pose frames", async (t) => {
  const f = await online(t, { publishRateHz: "4" });
  globalThis.weaverRealtimePublishReliable(1n, "fixture old profile");
  await flush();
  globalThis.weaverRealtimePublishReliable(2n, "fixture old chat");
  pose(3);
  await f.advance(100);
  f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
  f.elements.get("join-network").dispatchEvent(new Event("click"));
  await flush();
  assert.equal(f.clients.length, 2);
  assert.equal(f.harness.fetches.length, 2);
  assert.match(f.elements.get("network-status").textContent, /Online/);
  globalThis.weaverRealtimePublishReliable(10n, "fixture new profile");
  await flush();
  pose(11);
  await f.advance(160); // The old scheduler's deadline must not send into either connection.
  assert.deepEqual(f.client.publishes.map((call) => call.sequence), [1n]);
  assert.deepEqual(f.clients[1].publishes.map((call) => call.sequence), [10n]);
  await f.advance(100);
  assert.deepEqual(f.clients[1].publishes.map(({ sequence, at }) => [sequence, at]), [[10n, 100], [11n, 360]]);
  await f.advance(3_000);
  assert.deepEqual(f.client.publishes.map((call) => call.sequence), [1n]);
  assert.equal(f.clients[1].closed(), false);
});

for (const rejected of [false, true]) {
  test("a late old write " + (rejected ? "rejection" : "completion") + " cannot drain or disconnect the replacement connection", async (t) => {
    let finish;
    const f = await online(t, { publishRateHz: "4", connect: (client, count) => {
      if (count === 1) {
        const publish = client.publishEvent;
        client.publishEvent = async (...args) => {
          await publish(...args);
          await new Promise((resolve, reject) => { finish = rejected ? reject : resolve; });
        };
      }
      return client;
    } });
    globalThis.weaverRealtimePublishReliable(1n, "fixture old profile");
    await flush();
    globalThis.weaverRealtimePublishReliable(2n, "fixture old chat");
    pose(3);
    f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
    f.elements.get("join-network").dispatchEvent(new Event("click"));
    await flush();
    assert.equal(f.clients.length, 2);
    globalThis.weaverRealtimePublishReliable(10n, "fixture new profile");
    pose(11);
    await flush();
    finish(rejected ? new Error("fixture obsolete write failed") : undefined);
    await flush();
    await f.advance(260);
    assert.deepEqual(f.client.publishes.map((call) => call.sequence), [1n]);
    assert.deepEqual(f.clients[1].publishes.map((call) => call.sequence), [10n, 11n]);
    assert.equal(f.clients[1].closed(), false);
    assert.match(f.elements.get("network-status").textContent, /Online/);
  });
}

test("hidden tabs abort bootstrap; restoration does not autojoin and a manual retry fetches anew", async (t) => {
  const f = await shell(t, { fetch: (_url, _init, count) => count === 1 ? new Promise(() => {}) : jsonResponse() });
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 1);
  await f.visible("hidden");
  assert.equal(f.harness.fetches[0].init.signal.aborted, true);
  await f.visible("visible");
  assert.equal(f.harness.fetches.length, 1);
  assert.equal(f.elements.get("join-network").disabled, false);
  f.elements.get("join-network").dispatchEvent(new Event("click"));
  await flush();
  assert.equal(f.harness.fetches.length, 2);
  assert.equal(f.harness.connects.length, 1);
});

test("pagehide cancels an unresolved handshake and closes a late client without joining", async (t) => {
  let finish;
  const f = await shell(t, { connect: () => new Promise((resolve) => { finish = resolve; }) });
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.connects.length, 1);
  f.window.dispatchEvent(new Event("pagehide"));
  finish(f.client);
  await flush();
  assert.equal(f.client.closed(), true);
  assert.equal(f.harness.events.some(([kind]) => kind === "connected"), false);
  const restored = new Event("pageshow");
  Object.defineProperty(restored, "persisted", { value: true });
  f.window.dispatchEvent(restored);
  await flush();
  assert.equal(f.harness.fetches.length, 1);
  assert.equal(f.elements.get("join-network").disabled, false);
});

test("fatal graphics callbacks disconnect and remain fatal despite later ready notifications", async (t) => {
  const f = await shell(t);
  globalThis.weaverSceneReady();
  await flush();
  globalThis.weaverSceneFatal("fixture GPU lost");
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.client.closed(), true);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.equal(f.elements.get("reload-scene").hidden, false);
  assert.match(f.elements.get("network-status").textContent, /Reload/);
  assert.equal(f.harness.connects.length, 1);
});

test("WASM startup failure is visible and never enables joining", async (t) => {
  const f = await shell(t, { initError: true });
  assert.match(f.elements.get("network-status").textContent, /Unable to start Weaver/);
  assert.equal(f.elements.get("reload-scene").hidden, false);
  assert.equal(f.elements.get("connect-network").disabled, true);
  assert.equal(f.harness.fetches.length, 0);
});

test("unsupported WebTransport is actionable without fetching a credential", async (t) => {
  const f = await shell(t, { noWebTransport: true });
  globalThis.weaverSceneReady();
  await flush();
  assert.match(f.elements.get("network-status").textContent, /WebTransport is required/);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.equal(f.harness.fetches.length, 0);
});

test("local token autoconnect is preserved but waits for renderer readiness", async (t) => {
  const f = await shell(t, { visitor: false, origin: "http://localhost:8000",
    fetch: () => new Response(TOKEN) });
  assert.equal(f.harness.fetches.length, 0);
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 1);
  assert.equal(f.harness.fetches[0].url, "./woven.local-token");
  assert.equal(f.harness.connects.length, 1);
  assert.equal(f.elements.get("woven-token").value, TOKEN);
  assert.equal([...f.storage.values()].some((value) => value.includes(TOKEN)), false);
});

test("startup without the renderer callback times out and cannot be revived by a late ready", async (t) => {
  const f = await shell(t);
  t.mock.timers.tick(20_000);
  globalThis.weaverSceneReady();
  await flush();
  assert.match(f.elements.get("network-status").textContent, /loading stalled/);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.equal(f.harness.fetches.length, 0);
});

test("empty bootstrap metas do not autoconnect the public operator mode", async (t) => {
  const f = await shell(t, { visitor: false });
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 0);
  assert.equal(f.harness.connects.length, 0);
  assert.equal(f.elements.get("join-network").disabled, false);
  assert.match(f.elements.get("network-status").textContent, /Settings/);
});

function expectedBoot(minimal) {
  return [["init"], ["initialized"], ["minimal", minimal], ["name", "Bright Otter"], ["start"]];
}

for (const [presentation, minimal] of [[undefined, false], ["minimal", true], ["immersive", false],
  ["full", false], ["Minimal", false], ["", false]]) {
  test("presentation is set before scene start: " + (presentation === undefined ? "absent marker" : JSON.stringify(presentation)), async (t) => {
    const f = await shell(t, { presentation });
    assert.deepEqual(f.harness.bootEvents, expectedBoot(minimal));
    assert.deepEqual(f.harness.startState, { joinDisabled: true, fetchCount: 0, connectionCount: 0 });
    assert.equal(f.elements.get("display-name").value, "Bright Otter");
    assert.equal(f.harness.fetches.length, 0);
    assert.equal(f.harness.connects.length, 0);
    globalThis.weaverSceneReady();
    await flush();
    assert.equal(f.harness.connects.length, 1);
    assert.equal(f.elements.get("join-network").disabled, true);
    assert.match(f.elements.get("network-status").textContent, /Online/);
  });
}

test("pending WASM initialization cannot set presentation, configure a profile, or start the scene", async (t) => {
  let finishInit;
  let harness;
  const initializing = new Promise((resolve) => { finishInit = resolve; });
  const boot = shell(t, { presentation: "minimal", init: (active) => {
    harness = active;
    return initializing;
  } });
  try {
    await flush();
    assert.ok(harness);
    assert.deepEqual(harness.bootEvents, [["init"]]);
    assert.equal(globalThis.weaverSendChat("too early"), false);
    assert.equal(globalThis.weaverSetChatFocus(true), false);
    assert.equal(harness.pendingChat.length, 0);
    assert.equal(harness.chatFocus.length, 0);
    assert.equal(harness.startState, null);
    assert.equal(harness.fetches.length, 0);
    assert.equal(harness.connects.length, 0);
    finishInit();
    const f = await boot;
    assert.deepEqual(f.harness.bootEvents, expectedBoot(true));
    assert.equal(f.harness.fetches.length, 0);
  } finally {
    finishInit();
    await boot;
  }
});

test("synchronous renderer readiness during start still waits for shell initialization", async (t) => {
  const f = await shell(t, { presentation: "minimal", readyOnStart: true });
  assert.deepEqual(f.harness.bootEvents, expectedBoot(true));
  assert.deepEqual(f.harness.startState, { joinDisabled: true, fetchCount: 0, connectionCount: 0 });
  await flush();
  assert.equal(f.harness.fetches.length, 1);
  assert.equal(f.harness.connects.length, 1);
  assert.match(f.elements.get("network-status").textContent, /Online/);
});

test("failed WASM initialization never sets presentation or starts a scene", async (t) => {
  const f = await shell(t, { presentation: "minimal", initError: true });
  assert.deepEqual(f.harness.bootEvents, [["init"]]);
  assert.equal(f.harness.startState, null);
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 0);
  assert.equal(f.harness.connects.length, 0);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.match(f.elements.get("network-status").textContent, /fixture WASM assets missing/);
});

test("explicit scene startup failures stay fatal and never join", async (t) => {
  const f = await shell(t, { presentation: "minimal", startError: true });
  assert.deepEqual(f.harness.bootEvents, expectedBoot(true));
  globalThis.weaverSceneReady();
  await flush();
  assert.equal(f.harness.fetches.length, 0);
  assert.equal(f.harness.connects.length, 0);
  assert.equal(f.elements.get("join-network").disabled, true);
  assert.equal(f.elements.get("reload-scene").hidden, false);
  assert.match(f.elements.get("network-status").textContent, /fixture renderer startup failed/);
});


function key(f, type, key, options = {}) {
  const event = new KeyboardFixtureEvent(type, { key, code: key === "Enter" ? "Enter" : "Key" + key.toUpperCase(), ...options });
  if (options.target) {
    Object.defineProperty(event, "composedPath", { value: () => [options.target, f.window] });
    Object.defineProperty(event, "target", { value: options.target });
  }
  f.window.dispatchEvent(event);
  return event;
}

test("host callbacks are preserved, Rust alone echoes accepted chat, and decimal u64 IDs remain strings", async (t) => {
  const messages = [];
  const callback = (...args) => messages.push(args);
  const f = await online(t, { presentation: "minimal", chatCallback: callback });
  assert.equal(globalThis.weaverChatMessage, callback);
  assert.equal(globalThis.weaverSendChat("hello 🌍"), true);
  assert.deepEqual(messages, []); // No shell echo on WASM command acceptance.
  f.harness.flushChat();
  f.harness.flushChat();
  const remote = ["18446744073709551615", "Visitor", "validated remote", false];
  globalThis.weaverChatMessage(...remote); // Rust's accepted remote notification seam.
  assert.deepEqual(messages, [["7", "Bright Otter", "hello 🌍", true], remote]);
  const payload = new TextEncoder().encode('{"kind":"chat","version":1,"display_name":"Raw","text":"not yet accepted"}');
  f.client.send({ namespaceId: 2n, sessionId: 2n, spaceId: 3n, spaceEpoch: 1n, channelId: 1n,
    payloadTypeId: 1n, entityId: 8n, senderSequence: 1n, messageKind: 20, payload });
  await flush();
  assert.equal(messages.length, 2); // Shell forwards payload only to Rust, never to the host callback.
  assert.equal(f.harness.events.filter(([kind]) => kind === "payload").length, 1);
  globalThis.weaverSceneFatal("fixture failure");
  assert.equal(messages.length, 2); // Neither diagnostics nor disconnect reasons are chat.
});

test("an absent host callback gets a no-op, and Rust rejection produces no echo", async (t) => {
  const f = await online(t, { acceptChat: false });
  assert.equal(typeof globalThis.weaverChatMessage, "function");
  assert.doesNotThrow(() => globalThis.weaverChatMessage("7", "Ada", "hello", true));
  assert.equal(globalThis.weaverSendChat("rejected"), false);
  assert.deepEqual(f.harness.chatSends, ["rejected"]);
  assert.deepEqual(f.harness.pendingChat, []);
});

test("host send rejects malformed UTF-16 and wrong types before WASM can replace or coerce them", async (t) => {
  const f = await online(t);
  for (const text of [undefined, null, 42, {}, new String("wrapped"), "\ud800", "\udc00", "x\ud800y",
    "\udc00\ud800", "x".repeat(1025), "🌍".repeat(513)]) {
    assert.equal(globalThis.weaverSendChat(text), false);
  }
  assert.deepEqual(f.harness.chatSends, []);
  assert.equal(globalThis.weaverSendChat("🌍".repeat(256)), true);
  assert.equal(f.harness.chatSends.length, 1); // Rust enforces scalar, byte, control, and rate limits.
});

test("send requires a current online visible scene and room queue capacity, but focus works offline", async (t) => {
  const f = await shell(t, { presentation: "minimal", publishRateHz: "12" });
  assert.equal(globalThis.weaverSendChat("offline"), false);
  f.document.pointerLockElement = f.elements.get("scene");
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  assert.equal(f.document.pointerLockExits, 1); // Synchronous even before renderer/network readiness.
  assert.deepEqual(f.harness.chatFocus, [true]);
  assert.equal(globalThis.weaverSetChatFocus(false), true);
  globalThis.weaverSceneReady();
  await flush();
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  await flush();
  for (let i = 0; i < 32; i += 1) globalThis.weaverRealtimePublishReliable(BigInt(i + 2), "fixture queued");
  assert.equal(globalThis.weaverChatSendCapacity(), 0);
  assert.equal(globalThis.weaverSendChat("full"), false);
  assert.equal(f.harness.chatSends.length, 0);
  f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
  assert.equal(globalThis.weaverSendChat("disconnected"), false);
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  await f.visible("hidden");
  assert.equal(globalThis.weaverSendChat("hidden"), false);
  assert.equal(globalThis.weaverSetChatFocus(true), false);
  await f.visible("visible");
  assert.equal(globalThis.weaverSendChat("restored offline"), false);
  assert.equal(globalThis.weaverSetChatFocus(false), true);
  globalThis.weaverSceneFatal("fixture fatal");
  assert.equal(globalThis.weaverSendChat("failed"), false);
  assert.equal(globalThis.weaverSetChatFocus(true), false);
});

test("focus synchronously releases held game keys and pointer lock, blocks native input, and cleans up on exit/blur", async (t) => {
  const f = await shell(t, { presentation: "minimal" });
  const canvas = f.elements.get("scene");
  const released = [];
  canvas.addEventListener("keyup", (event) => released.push(event.code));
  key(f, "keydown", "w");
  key(f, "keydown", "ArrowUp", { code: "ArrowUp" });
  f.document.activeElement = canvas;
  f.document.pointerLockElement = canvas;
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  assert.equal(f.document.pointerLockExits, 1);
  assert.equal(canvas.blurCount, 1);
  assert.deepEqual(released, ["KeyW", "ArrowUp"]);
  let nativeInput = 0;
  f.window.addEventListener("keydown", () => nativeInput += 1);
  f.window.addEventListener("beforeinput", () => nativeInput += 1);
  assert.equal(key(f, "keydown", "w").defaultPrevented, true);
  assert.equal(key(f, "keydown", "Enter").defaultPrevented, true);
  const input = new Event("beforeinput", { cancelable: true });
  f.window.dispatchEvent(input);
  assert.equal(input.defaultPrevented, true);
  assert.equal(nativeInput, 0);
  assert.equal(globalThis.weaverSetChatFocus(false), true); // Host cancel/close path.
  f.document.activeElement = null;
  assert.equal(key(f, "keydown", "w", { repeat: true }).defaultPrevented, true);
  assert.equal(nativeInput, 0); // Held repeat cannot recreate movement after closing the composer.
  key(f, "keyup", "w");
  assert.equal(key(f, "keydown", "w").defaultPrevented, false);
  assert.equal(key(f, "keydown", "w", { repeat: true }).defaultPrevented, false);
  assert.equal(nativeInput, 2);
  f.window.dispatchEvent(new Event("blur"));
  assert.deepEqual(released, ["KeyW", "ArrowUp", "KeyW"]);
  assert.deepEqual(f.harness.chatFocus, [true, false, false]);
  assert.equal(globalThis.weaverSetChatFocus("true"), false);
});

test("explicit resume focuses the scene and requests mouse lock without synthetic clicks", async (t) => {
  const f = await shell(t, { presentation: "minimal" });
  const canvas = f.elements.get("scene");
  let locks = 0;
  let clicks = 0;
  canvas.requestPointerLock = () => { locks += 1; return Promise.resolve(); };
  canvas.addEventListener("click", () => clicks += 1);
  assert.equal(globalThis.weaverResumeGameInput(), false);
  globalThis.weaverSceneReady();
  f.dismissStartup();
  canvas.focusCount = 0;
  await flush();
  globalThis.weaverSetChatFocus(true);
  assert.equal(globalThis.weaverResumeGameInput(), false);
  globalThis.weaverSetChatFocus(false);
  assert.equal(locks, 0); // Closing/canceling chat alone never grabs the mouse.
  assert.equal(globalThis.weaverResumeGameInput(), true);
  assert.equal(canvas.focusCount, 1);
  assert.equal(locks, 1);
  assert.equal(clicks, 0);
  canvas.requestPointerLock = () => Promise.reject(new Error("browser denied"));
  assert.equal(globalThis.weaverResumeGameInput(), true);
  await flush();
  canvas.requestPointerLock = () => { throw new Error("legacy denial"); };
  assert.equal(globalThis.weaverResumeGameInput(), true);
  f.window.matchMedia = () => ({ matches: true });
  canvas.requestPointerLock = () => locks += 1;
  assert.equal(globalThis.weaverResumeGameInput(), true);
  assert.equal(locks, 1); // Coarse pointer only restores focus.
  await f.visible("hidden");
  assert.equal(globalThis.weaverResumeGameInput(), false);
  await f.visible("visible");
  globalThis.weaverSceneFatal("fixture fatal");
  assert.equal(globalThis.weaverResumeGameInput(), false);
});

test("host focus clears Shift and rejects its stale repeats after chat closes", async (t) => {
  const f = await shell(t, { presentation: "minimal" });
  const released = [];
  f.elements.get("scene").addEventListener("keyup", (event) => released.push(event.code));
  key(f, "keydown", "Shift", { code: "ShiftLeft" });
  key(f, "keydown", "Shift", { code: "ShiftRight" });
  globalThis.weaverSetChatFocus(true);
  assert.deepEqual(released, ["ShiftLeft", "ShiftRight"]);
  globalThis.weaverSetChatFocus(false);
  assert.equal(key(f, "keydown", "Shift", { code: "ShiftLeft", repeat: true }).defaultPrevented, true);
  key(f, "keyup", "Shift", { code: "ShiftLeft" });
  assert.equal(key(f, "keydown", "Shift", { code: "ShiftLeft" }).defaultPrevented, false);
});

test("minimal Enter routes once to the host adapter, never native chat, including a physical mobile keyboard", async (t) => {
  let requests = 0;
  const f = await shell(t, { presentation: "minimal", requestChat: () => { requests += 1; return true; } });
  let nativeKeys = 0;
  f.window.addEventListener("keydown", () => nativeKeys += 1);
  assert.equal(key(f, "keydown", "Enter").defaultPrevented, true);
  assert.equal(key(f, "keydown", "Enter").defaultPrevented, true); // Duplicate event, not just repeat=true.
  assert.equal(key(f, "keydown", "Enter", { repeat: true }).defaultPrevented, true);
  assert.equal(requests, 1);
  assert.equal(nativeKeys, 0);
  key(f, "keyup", "Enter");
  assert.equal(key(f, "keydown", "Enter", { code: "NumpadEnter" }).defaultPrevented, true);
  assert.equal(requests, 2);
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  key(f, "keyup", "Enter");
  key(f, "keydown", "Enter");
  assert.equal(requests, 2);
  assert.equal(nativeKeys, 0);
  globalThis.weaverSetChatFocus(false);
  key(f, "keyup", "Enter");
  key(f, "keydown", "Enter", { isComposing: true });
  assert.equal(requests, 2);
});

for (const tagName of ["INPUT", "TEXTAREA", "SELECT", "contenteditable"]) {
  test("minimal shortcut and host focus do not steal composer key input: " + tagName, async (t) => {
    let requests = 0;
    const f = await shell(t, { presentation: "minimal", requestChat: () => { requests += 1; return true; } });
    const composer = new Element();
    composer.tagName = tagName === "contenteditable" ? "DIV" : tagName;
    composer.isContentEditable = tagName === "contenteditable";
    for (const focused of [false, true]) {
      globalThis.weaverSetChatFocus(focused);
      assert.equal(key(f, "keydown", "Enter", { target: composer }).defaultPrevented, false);
      assert.equal(key(f, "keydown", "w", { target: composer }).defaultPrevented, false);
      f.document.activeElement = composer;
      assert.equal(key(f, "keydown", "Enter").defaultPrevented, false);
      const input = new Event("beforeinput", { cancelable: true });
      f.window.dispatchEvent(input);
      assert.equal(input.defaultPrevented, false);
      f.document.activeElement = null;
    }
    assert.equal(requests, 0);
  });
}

for (const presentation of [undefined, "full", "Minimal"]) {
  test("unmarked/standalone demos retain native Enter and window blur behavior: " + presentation, async (t) => {
    let requests = 0;
    const f = await shell(t, { presentation, requestChat: () => { requests += 1; return true; } });
    assert.equal(key(f, "keydown", "Enter").defaultPrevented, false);
    assert.equal(requests, 0);
    f.window.dispatchEvent(new Event("blur"));
    assert.deepEqual(f.harness.chatFocus, []);
  });
}

test("minimal Enter tolerates an absent or declining host adapter without opening native chat", async (t) => {
  const f = await shell(t, { presentation: "minimal" });
  assert.equal(globalThis.weaverHostRequestChat, undefined);
  assert.equal(key(f, "keydown", "Enter").defaultPrevented, true);
  key(f, "keyup", "Enter");
  let requests = 0;
  globalThis.weaverHostRequestChat = () => { requests += 1; return false; };
  key(f, "keydown", "Enter");
  key(f, "keydown", "Enter");
  assert.equal(requests, 1);
});

for (const trigger of ["disconnect", "pagehide", "hidden", "scene-fatal", "realtime-fatal"]) {
  test("pending host submissions cannot echo or send after " + trigger + " or retry", async (t) => {
    const messages = [];
    const f = await online(t, { presentation: "minimal", publishRateHz: "12", chatCallback: (...args) => messages.push(args) });
    assert.equal(globalThis.weaverSendChat("stale pending host command"), true);
    if (trigger === "disconnect") f.elements.get("disconnect-network").dispatchEvent(new Event("click"));
    else if (trigger === "pagehide") f.window.dispatchEvent(new Event("pagehide"));
    else if (trigger === "hidden") await f.visible("hidden");
    else if (trigger === "scene-fatal") globalThis.weaverSceneFatal("fixture fatal");
    else globalThis.weaverRealtimeFatal("fixture fatal");
    assert.equal(globalThis.weaverSendChat("offline"), false);
    f.harness.flushChat();
    assert.deepEqual(messages, []);
    assert.deepEqual(f.client.publishes, []);
    if (trigger === "scene-fatal") return;
    await f.visible("visible");
    f.elements.get("join-network").dispatchEvent(new Event("click"));
    await flush();
    assert.equal(f.clients.length, 2);
    f.harness.flushChat();
    assert.deepEqual(f.clients[1].publishes, []);
    assert.equal(globalThis.weaverSendChat("fresh"), true);
    f.harness.flushChat();
    assert.deepEqual(messages, [["7", "Bright Otter", "fresh", true]]);
  });
}

test("12 Hz pacing remains shared below the 15 Hz server cap while host chat is focused", async (t) => {
  const f = await online(t, { presentation: "minimal", publishRateHz: "12" });
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  globalThis.weaverRealtimePublishReliable(1n, "fixture profile");
  await flush();
  for (let i = 1; i <= 20; i += 1) {
    await f.advance(100);
    pose(i + 1); // Existing Rust generation remains 10 Hz.
    if (i === 5 || i === 10) {
      assert.equal(globalThis.weaverSendChat("host message " + i), true);
      f.harness.flushChat();
    }
    await flush();
  }
  await f.advance(100);
  const calls = f.client.publishes;
  assert.equal(calls.filter((call) => call.kind === "event").length, 3);
  assert.ok(calls.some((call) => call.kind === "pose"));
  for (let i = 1; i < calls.length; i += 1) assert.ok(calls[i].at - calls[i - 1].at >= 1_000 / 12 + 10);
  for (const first of calls) assert.ok(calls.filter((call) => call.at >= first.at && call.at < first.at + 1_000).length <= 12);
  assert.ok(calls.filter((call) => call.kind === "event").every((call) => call.channel === 1n));
  assert.ok(calls.filter((call) => call.kind === "pose").every((call) => call.channel === 4n));
  assert.equal(f.client.closed(), false);
});


test("closing the host composer does not cancel a successfully queued send or eagerly echo it", async (t) => {
  const messages = [];
  const f = await online(t, { presentation: "minimal", chatCallback: (...args) => messages.push(args) });
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  assert.equal(globalThis.weaverSendChat("submitted before close"), true);
  assert.equal(globalThis.weaverSetChatFocus(false), true);
  assert.deepEqual(messages, []);
  assert.deepEqual(f.harness.pendingChat, ["submitted before close"]);
  f.harness.flushChat();
  f.harness.flushChat();
  await flush();
  assert.deepEqual(messages, [["7", "Bright Otter", "submitted before close", true]]);
  assert.equal(f.client.publishes.length, 1);
});


test("physical mobile Enter and focus work when the browser has no pointer-lock API", async (t) => {
  let requests = 0;
  const f = await shell(t, { presentation: "minimal", requestChat: () => { requests += 1; return true; } });
  delete f.document.pointerLockElement;
  delete f.document.exitPointerLock;
  assert.equal(key(f, "keydown", "Enter").defaultPrevented, true);
  assert.equal(requests, 1);
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  assert.equal(globalThis.weaverSetChatFocus(false), true);
});

function hudPointer(f, type, target) {
  const event = new Event(type, { cancelable: true });
  Object.defineProperty(event, "target", { value: target });
  f.window.dispatchEvent(event);
  return event;
}

async function hudShell(t, options = {}) {
  const f = await shell(t, { visitor: false, presentation: "minimal", ...options });
  globalThis.weaverSceneReady(); await f.advance(0);
  f.metadata(); await flush(); f.dismissStartup(); await flush();
  return f;
}

function q(f, type = "keydown", options = {}) {
  return key(f, type, "q", { code: "KeyQ", ...options });
}

test("minimal HUD Q owns input, clears movement before guards, restores prior lock only on explicit dismissal", async (t) => {
  const f = await hudShell(t);
  const canvas = f.elements.get("scene"), overlay = f.elements.get("controller-overlay");
  const audio = f.elements.get("controller-audio");
  const transitions = [], releases = [];
  f.window.addEventListener("weaver-controller-change", event => transitions.push(event.detail.open));
  canvas.addEventListener("keyup", event => releases.push([event.code, f.harness.chatFocus.at(-1)]));
  let locks = 0;
  canvas.requestPointerLock = () => { locks++; };
  canvas.focus(); f.document.pointerLockElement = canvas;
  key(f, "keydown", "w", { code: "KeyW" });
  assert.equal(q(f).defaultPrevented, true);
  assert.deepEqual(releases, [["KeyW", false]]);
  assert.equal(overlay.hidden, false); assert.equal(canvas.inert, true);
  assert.equal(f.document.activeElement, audio); assert.equal(f.document.pointerLockExits, 1);
  assert.equal(f.harness.chatFocus.at(-1), true);
  assert.equal(globalThis.weaverResumeGameInput(), false);
  assert.equal(key(f, "keydown", "w", { code: "KeyW" }).defaultPrevented, true);
  q(f, "keydown", { repeat: true }); assert.equal(overlay.hidden, false);
  q(f); assert.equal(overlay.hidden, false);
  q(f, "keyup"); q(f);
  assert.equal(overlay.hidden, true); assert.equal(canvas.inert, false); assert.equal(locks, 1);
  assert.deepEqual(transitions, [true, false]);
  assert.equal(key(f, "keydown", "w", { code: "KeyW", repeat: true }).defaultPrevented, true);
  q(f, "keyup"); canvas.focus(); f.document.pointerLockElement = canvas; q(f);
  assert.equal(globalThis.weaverSetChatFocus(true), true);
  assert.equal(overlay.hidden, true); assert.equal(locks, 1);
  q(f, "keyup"); q(f); assert.equal(overlay.hidden, true);
  globalThis.weaverSetChatFocus(false);
  q(f, "keyup"); canvas.focus(); q(f);
  await f.visible("hidden"); assert.equal(overlay.hidden, true); assert.equal(locks, 1);
  await f.visible("visible"); assert.equal(overlay.hidden, true); assert.equal(locks, 1);
  globalThis.weaverDisposeTheaterVideo();
  assert.equal(f.elements.get("controller-launcher").hidden, true);
  q(f, "keyup"); q(f); assert.equal(overlay.hidden, true);
});

test("HUD ignores editing, modifiers, composition and startup; completed outside click consumes the gesture", async (t) => {
  const f = await shell(t, { visitor: false, presentation: "minimal" });
  globalThis.weaverSceneReady(); await f.advance(0);
  const overlay = f.elements.get("controller-overlay"), dialog = f.elements.get("controller-dialog");
  assert.equal(overlay.hidden, false); // Controller itself is the startup presentation.
  q(f); assert.equal(overlay.hidden, true); q(f, "keyup");
  for (const options of [{ ctrlKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }, { repeat: true }]) {
    q(f, "keydown", options); assert.equal(overlay.hidden, true); q(f, "keyup");
  }
  const input = new Element(); input.tagName = "INPUT"; f.document.activeElement = input;
  q(f); assert.equal(overlay.hidden, true); q(f, "keyup");
  f.elements.get("scene").focus(); trustedClick(f.elements.get("controller-launcher"));
  assert.equal(overlay.hidden, false);
  let leaked = 0;
  f.window.addEventListener("click", () => leaked++);
  assert.equal(hudPointer(f, "pointerdown", overlay).defaultPrevented, true);
  hudPointer(f, "pointerup", overlay); assert.equal(overlay.hidden, false);
  assert.equal(hudPointer(f, "click", overlay).defaultPrevented, true);
  assert.equal(overlay.hidden, true); assert.equal(leaked, 0);
  q(f); assert.equal(overlay.hidden, false);
  hudPointer(f, "pointerdown", dialog); hudPointer(f, "pointerup", overlay); hudPointer(f, "click", overlay);
  assert.equal(overlay.hidden, false); // A drag starting inside is not an outside click.
  assert.equal(hudPointer(f, "wheel", f.elements.get("scene")).defaultPrevented, true);
  key(f, "keydown", "Tab"); assert.equal(f.document.activeElement, f.elements.get("controller-subtitles"));
  key(f, "keydown", "Tab", { shiftKey: true }); assert.equal(f.document.activeElement, f.elements.get("controller-audio"));
  key(f, "keydown", "Escape"); assert.equal(overlay.hidden, true);
});

test("HUD audio and fullscreen run directly in the gesture, audio art follows preference and controls stay open", async (t) => {
  const f = await hudShell(t);
  const video = f.elements.get("theater-video"), audio = f.elements.get("controller-audio");
  const full = f.elements.get("controller-fullscreen"), subs = f.elements.get("controller-subtitles");
  let gesture = false, toggles = 0;
  let fullscreen = { available: true, active: false, pending: false, error: false };
  f.window.jdharrisonGetFullscreenState = () => fullscreen;
  f.window.jdharrisonToggleFullscreen = () => { assert.equal(gesture, true); toggles++; };
  q(f);
  assert.equal(audio.dataset.state, "off");
  const nativePlay = video.play.bind(video);
  video.play = () => { assert.equal(gesture, true); return nativePlay(); };
  gesture = true; trustedClick(audio); gesture = false;
  assert.equal(video.muted, false); assert.equal(audio.dataset.state, "on");
  assert.equal(f.elements.get("theater-startup").hidden, true);
  trustedClick(audio); assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
  trustedClick(subs); assert.equal(subs.dataset.state, "off");
  assert.equal(f.harness.panelStates.filter(([id]) => id === 3).at(-1)[1], 1);
  gesture = true; trustedClick(full); gesture = false;
  assert.equal(toggles, 1); assert.equal(full.dataset.state, "off");
  fullscreen = { ...fullscreen, active: true, pending: true };
  f.window.dispatchEvent(new CustomEvent("jdharrison-fullscreen-state", { detail: fullscreen }));
  assert.equal(full.dataset.state, "on"); assert.equal(full.disabled, true);
  fullscreen = { ...fullscreen, pending: false, error: true };
  f.window.dispatchEvent(new CustomEvent("jdharrison-fullscreen-state", { detail: fullscreen }));
  assert.equal(full.disabled, false); assert.match(f.elements.get("controller-status").textContent, /Fullscreen failed/);
  assert.equal(f.elements.get("controller-overlay").hidden, false);
  assert.equal(video.paused, false);
});

test("HUD preparing and failed audio never queues a delayed audible attempt or reopens startup", async (t) => {
  const f = await shell(t, { visitor: false, presentation: "minimal" });
  globalThis.weaverSceneReady(); await f.advance(0);
  const audio = f.elements.get("controller-audio"), video = f.elements.get("theater-video");
  trustedClick(audio); assert.equal(video.playCount, 0);
  f.metadata(); await flush(); assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
  video.playError = new Error("NotAllowedError"); trustedClick(audio); await flush();
  assert.equal(audio.dataset.state, "on"); assert.equal(video.muted, true);
  assert.match(f.elements.get("controller-status").textContent, /unavailable.*OFF.*retry\/turn ON/);
  trustedClick(audio); assert.equal(audio.dataset.state, "off");
  video.playError = null;
  trustedClick(audio); await flush(); assert.equal(audio.dataset.state, "on");
  video.error = { code: 4 }; video.dispatchEvent(new Event("error")); video.error = null;
  assert.match(f.elements.get("controller-status").textContent, /unavailable.*OFF.*retry\/turn ON/);
  trustedClick(audio); assert.equal(audio.dataset.state, "off");
  assert.match(f.elements.get("controller-status").textContent, /unavailable.*retry.*again when ready/);
  const failedSources = [...video.sources], failedPlays = video.playCount;
  audio.dispatchEvent(new Event("click")); await flush();
  assert.deepEqual(video.sources, failedSources); assert.equal(video.playCount, failedPlays);
  trustedClick(audio); await flush();
  assert.equal(f.elements.get("theater-startup").hidden, true);
  f.metadata(); await flush(); assert.equal(video.muted, true);
  assert.equal(audio.dataset.state, "off");
  globalThis.weaverDisposeTheaterVideo();
  const plays = video.playCount; trustedClick(audio); await flush(); assert.equal(video.playCount, plays);
});

test("HUD leaves the live connection/movie running and resets Q across suspension without recapture", async (t) => {
  const f = await hudShell(t, { visitor: true });
  const canvas = f.elements.get("scene"), overlay = f.elements.get("controller-overlay");
  assert.equal(f.clients.length, 1); assert.equal(f.client.closed(), false);
  const video = f.elements.get("theater-video"), pauses = video.pauseCount;
  let locks = 0; canvas.requestPointerLock = () => { locks++; };
  q(f);
  assert.equal(f.client.closed(), false); assert.equal(video.pauseCount, pauses);
  assert.equal(video.paused, false);
  f.document.pointerLockElement = canvas;
  f.document.dispatchEvent(new Event("pointerlockchange"));
  assert.equal(f.document.pointerLockElement, null);
  f.elements.get("controller-audio").focus();
  key(f, "keydown", "Tab", { shiftKey: true });
  // No host fullscreen bridge in this fixture, so the last enabled button is Subtitles.
  assert.equal(f.document.activeElement, f.elements.get("controller-subtitles"));
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(overlay.hidden, true); assert.equal(locks, 0);
  q(f); assert.equal(overlay.hidden, false); // Fresh press, even without a hidden-tab keyup.
  globalThis.weaverDisposeTheaterVideo();
  assert.equal(overlay.hidden, true); assert.equal(canvas.inert, false); assert.equal(locks, 0);
});

test("launcher click followed by outside dismissal focuses canvas and never invents a prior pointer lock", async t => {
  const f = await hudShell(t);
  const launcher = f.elements.get("controller-launcher"), canvas = f.elements.get("scene");
  const overlay = f.elements.get("controller-overlay");
  let locks = 0; canvas.requestPointerLock = () => { locks++; };
  launcher.focus(); trustedClick(launcher);
  assert.equal(overlay.hidden, false);
  hudPointer(f, "pointerdown", overlay); hudPointer(f, "pointerup", overlay);
  const click = hudPointer(f, "click", overlay);
  assert.equal(click.defaultPrevented, true);
  assert.equal(overlay.hidden, true); assert.equal(f.document.activeElement, canvas);
  assert.equal(locks, 0);
  f.document.pointerLockElement = canvas;
  launcher.focus(); trustedClick(launcher);
  hudPointer(f, "pointerdown", overlay); hudPointer(f, "pointerup", overlay); hudPointer(f, "click", overlay);
  assert.equal(f.document.activeElement, canvas); assert.equal(locks, 1);
});

for (const presentation of ["minimal", "full"]) {
  test(`controller image preloads are early, nonblocking and minimal-only (${presentation})`, async t => {
    const f = await shell(t, { visitor: false, presentation });
    assert.equal(f.document.head.children.length, presentation === "minimal" ? 8 : 0);
    assert.equal(f.harness.scheduleFetches.length, 0); // Hints do not require renderer/media readiness.
    if (presentation === "minimal") {
      globalThis.weaverSceneReady(); f.dismissStartup();
      trustedClick(f.elements.get("controller-launcher"));
      assert.equal(f.elements.get("controller-overlay").hidden, false);
      assert.equal(f.document.head.children.length, 8);
    }
  });
}

test("HUD starts expanded with desired Audio OFF; the first ready press enables sound synchronously", async t => {
  const f = await shell(t, { visitor: false, presentation: "minimal" });
  const audio = f.elements.get("controller-audio"), video = f.elements.get("theater-video");
  assert.equal(audio.dataset.state, "off"); assert.equal(audio.getAttribute("aria-pressed"), "false");
  assert.equal(video.muted, true);
  globalThis.weaverSceneReady(); await f.advance(0); f.metadata(); await flush();
  assert.equal(audio.dataset.state, "off"); assert.equal(video.muted, true);
  assert.equal(f.elements.get("controller-overlay").hidden, false);
  assert.equal(f.elements.get("theater-startup").hidden, true);
  let gesture = false;
  const play = video.play.bind(video);
  video.play = () => { assert.equal(gesture, true); return play(); };
  gesture = true; trustedClick(audio); gesture = false;
  assert.equal(audio.dataset.state, "on"); assert.equal(video.muted, false);
  globalThis.weaverSceneReady();
  assert.equal(audio.dataset.state, "on"); // Readiness must not reapply initial mute.
});

test("HUD permits OFF while enabling is pending but cannot enqueue a second enabling gesture", async t => {
  const f = await hudShell(t);
  const audio = f.elements.get("controller-audio"), video = f.elements.get("theater-video");
  const pending = deferred(), plays = video.playCount;
  q(f); video.playPromise = pending.promise;
  trustedClick(audio);
  assert.equal(video.playCount, plays + 1); assert.equal(audio.dataset.state, "on");
  assert.equal(audio.disabled, false); assert.equal(audio.getAttribute("aria-busy"), "true");
  assert.match(f.elements.get("controller-status").textContent, /Preparing audio.*OFF.*cancel/);
  // Waiting removes observed audibility but must not turn the desired ON artwork off.
  video.dispatchEvent(new Event("waiting"));
  assert.equal(audio.dataset.state, "on");
  audio.dispatchEvent(new Event("click"));
  assert.equal(audio.dataset.state, "on"); assert.equal(video.muted, false);
  trustedClick(audio);
  assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
  assert.equal(audio.disabled, true);
  assert.match(f.elements.get("controller-status").textContent, /Preparing audio.*again when ready/);
  assert.equal(f.document.activeElement, f.elements.get("controller-subtitles"));
  trustedClick(audio); assert.equal(video.playCount, plays + 1); assert.equal(audio.dataset.state, "off");
  video.playPromise = null; pending.resolve(); await flush();
  assert.equal(audio.disabled, false); assert.equal(audio.dataset.state, "off");
  assert.equal(video.muted, true); assert.equal(video.playCount, plays + 1);
  let gesture = false;
  const nativePlay = video.play.bind(video);
  video.play = () => { assert.equal(gesture, true); return nativePlay(); };
  gesture = true; trustedClick(audio); gesture = false;
  assert.equal(video.playCount, plays + 2); assert.equal(audio.dataset.state, "on");
  assert.equal(video.muted, false);
});

test("second-finger launcher pointerup opens HUD while a canvas movement touch remains held, without click", async t => {
  const f = await hudShell(t);
  const canvas = f.elements.get("scene"), launcher = f.elements.get("controller-launcher"), overlay = f.elements.get("controller-overlay");
  f.harness.panelHit = 0;
  canvas.focus(); key(f, "keydown", "w", { code: "KeyW" });
  scenePointer(f, "pointerdown", { target: canvas, pointerType: "touch", pointerId: 1, isPrimary: true, timeStamp: 100 });
  const releases = [], transitions = [], invalidations = f.harness.sceneInvalidations;
  canvas.addEventListener("keyup", event => releases.push([event.code, f.harness.chatFocus.at(-1)]));
  f.window.addEventListener("weaver-controller-change", event => transitions.push(event.detail.open));
  const second = { target: launcher, pointerType: "touch", pointerId: 2, isPrimary: false, timeStamp: 120 };
  assert.equal(scenePointer(f, "pointerdown", second).defaultPrevented, true);
  assert.equal(overlay.hidden, true);
  assert.equal(scenePointer(f, "pointerup", { ...second, timeStamp: 200 }).defaultPrevented, true);
  assert.equal(overlay.hidden, false); assert.equal(canvas.inert, true);
  assert.deepEqual(releases, [["KeyW", false]]); assert.equal(f.harness.chatFocus.at(-1), true);
  assert.ok(f.harness.sceneInvalidations > invalidations);
  assert.equal(globalThis.weaverResumeGameInput(), false);
  // Late compatibility clicks must neither dismiss the new backdrop nor reopen after dismissal.
  assert.equal(scenePointer(f, "click", { ...second, target: overlay }).defaultPrevented, true);
  key(f, "keydown", "Escape"); assert.equal(overlay.hidden, true);
  assert.equal(scenePointer(f, "click", second).defaultPrevented, true);
  assert.equal(overlay.hidden, true); assert.deepEqual(transitions, [true, false]);
  assert.equal(key(f, "keydown", "w", { code: "KeyW", repeat: true }).defaultPrevented, true);
});

for (const readyOnStart of [false, true]) {
  test(`minimal startup is combined-ready, one-shot and independent of slow media (synchronous=${readyOnStart})`, async t => {
    const manifest = deferred();
    const f = await shell(t, { visitor: false, presentation: "minimal", readyOnStart,
      scheduleFetch: () => manifest.promise });
    const overlay = f.elements.get("controller-overlay"), canvas = f.elements.get("scene");
    assert.equal(overlay.hidden, !readyOnStart);
    assert.deepEqual(f.startupListeners, [], "legacy startup is not constructed");
    assert.equal(f.elements.get("theater-startup").hidden, true);
    const transitions = [];
    f.window.addEventListener("weaver-controller-change", event => transitions.push(event.detail.open));
    globalThis.weaverSceneReady(); globalThis.weaverSceneReady();
    assert.equal(overlay.hidden, false);
    assert.equal(canvas.inert, true);
    assert.equal(globalThis.weaverResumeGameInput(), false);
    assert.equal(f.harness.scheduleFetches.length, 0);
    await f.advance(0); await f.advance(13_000);
    assert.equal(overlay.hidden, false); assert.equal(f.elements.get("theater-startup").hidden, true);
    f.dismissStartup();
    globalThis.weaverSceneReady(); await f.visible("hidden"); await f.visible("visible");
    const restored = new Event("pageshow"); Object.defineProperty(restored, "persisted", { value: true });
    f.window.dispatchEvent(restored);
    assert.equal(overlay.hidden, true); assert.equal(canvas.inert, false);
    assert.deepEqual(transitions, readyOnStart ? [false] : [true, false]);
    manifest.resolve(scheduleResponse()); await flush(); f.metadata(); await flush();
    assert.equal(f.elements.get("theater-video").muted, true);
    assert.equal(overlay.hidden, true);
    q(f); assert.equal(overlay.hidden, false, "manual reopening remains available");
  });
}

for (const terminal of ["none", "fatal", "disposed"]) {
  test(`initially hidden ready scene presents once on first visibility unless ${terminal}`, async t => {
    const f = await shell(t, { visitor: false, presentation: "minimal", visibility: "hidden", readyOnStart: true });
    const overlay = f.elements.get("controller-overlay");
    assert.equal(overlay.hidden, true); assert.equal(f.document.activeElement, null);
    globalThis.weaverSceneReady();
    if (terminal === "fatal") globalThis.weaverSceneFatal("fixture fatal");
    if (terminal === "disposed") globalThis.weaverDisposeTheaterVideo();
    const focus = f.document.activeElement;
    await f.visible("visible");
    assert.equal(overlay.hidden, terminal !== "none");
    if (terminal !== "none") assert.equal(f.document.activeElement, focus);
    else {
      assert.equal(f.harness.scheduleFetches.length, 0);
      await f.visible("hidden"); await f.visible("visible");
      assert.equal(overlay.hidden, true, "suspension consumes no second presentation");
    }
  });
}

for (const dismissal of ["q", "Escape", "backdrop"]) {
  test(`startup ${dismissal} restores canvas keyboard focus without capture, fullscreen or audio`, async t => {
    let requests = 0;
    const f = await shell(t, { visitor: false, presentation: "minimal", requestChat: () => { requests++; return true; } });
    const canvas = f.elements.get("scene"), overlay = f.elements.get("controller-overlay"), video = f.elements.get("theater-video");
    let locks = 0, fullscreens = 0;
    canvas.requestPointerLock = () => { locks++; };
    f.window.jdharrisonToggleFullscreen = () => { fullscreens++; };
    // Even a pre-existing lock/UI focus must not be restored by startup dismissal.
    f.elements.get("controller-launcher").focus(); f.document.pointerLockElement = canvas;
    globalThis.weaverSceneReady();
    assert.equal(f.document.pointerLockElement, null);
    assert.equal(key(f, "keydown", "Enter").defaultPrevented, true); assert.equal(requests, 0);
    assert.equal(key(f, "keydown", "w", { code: "KeyW" }).defaultPrevented, true);
    if (dismissal === "q") { q(f); q(f, "keyup"); }
    else if (dismissal === "Escape") key(f, "keydown", "Escape");
    else for (const type of ["pointerdown", "pointerup", "click"]) hudPointer(f, type, overlay);
    assert.equal(overlay.hidden, true); assert.equal(canvas.inert, false);
    assert.equal(f.document.activeElement, canvas); assert.equal(f.harness.chatFocus.at(-1), false);
    assert.equal(locks, 0); assert.equal(fullscreens, 0); assert.equal(video.playCount, 0); assert.equal(video.muted, true);
    globalThis.weaverSceneReady(); assert.equal(overlay.hidden, true);
  });
}

test("minimal startup defers to host chat ownership without losing its first presentation", async t => {
  const f = await shell(t, { visitor: false, presentation: "minimal" });
  globalThis.weaverSetChatFocus(true);
  globalThis.weaverSceneReady();
  assert.equal(f.elements.get("controller-overlay").hidden, true);
  globalThis.weaverSetChatFocus(false);
  assert.equal(f.elements.get("controller-overlay").hidden, false);
  f.dismissStartup();
  globalThis.weaverSetChatFocus(true); globalThis.weaverSetChatFocus(false);
  assert.equal(f.elements.get("controller-overlay").hidden, true);
});

test("failed manifest keeps controller usable; retry prepares silently and requires a fresh ready Audio press", async t => {
  let available = false;
  const f = await shell(t, { visitor: false, presentation: "minimal", scheduleFetch: () => available ? scheduleResponse() : jsonResponse() });
  const audio = f.elements.get("controller-audio"), video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0);
  assert.equal(f.elements.get("controller-overlay").hidden, false);
  assert.equal(f.elements.get("theater-startup").hidden, true);
  assert.match(f.elements.get("controller-status").textContent, /retry/);
  assert.equal(audio.disabled, false);
  const failedPlays = video.playCount;
  audio.dispatchEvent(new Event("click")); await f.advance(1200);
  assert.equal(f.harness.scheduleFetches.length, 1);
  assert.equal(video.playCount, failedPlays); assert.equal(audio.dataset.state, "off");
  available = true; trustedClick(audio); trustedClick(audio); await flush();
  assert.equal(f.harness.scheduleFetches.length, 2); assert.equal(video.muted, true);
  assert.equal(audio.dataset.state, "off");
  f.metadata(); await flush(); assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
  trustedClick(audio); assert.equal(video.muted, false); assert.equal(audio.dataset.state, "on");
  assert.equal(f.elements.get("controller-overlay").hidden, false);
});

test("minimal startup cannot focus or open during WASM initialization or the synchronous renderer callback", async t => {
  let observed = false;
  const f = await shell(t, { visitor: false, presentation: "minimal", readyOnStart: true,
    init: async () => {
      globalThis.weaverSceneReady();
      assert.equal(document.getElementById("controller-overlay").hidden, true);
      assert.equal(document.activeElement, null);
      observed = true;
    } });
  assert.equal(observed, true);
  assert.equal(f.elements.get("controller-overlay").hidden, false);
  assert.equal(f.document.activeElement, f.elements.get("controller-audio"));
  assert.equal(f.harness.scheduleFetches.length, 0);
  assert.deepEqual(f.startupListeners, []);
});

test("synthetic ready Audio cannot enable or mute; first trusted ready press still enables synchronously", async t => {
  const f = await shell(t, { visitor: false, presentation: "minimal" });
  const audio = f.elements.get("controller-audio"), video = f.elements.get("theater-video");
  globalThis.weaverSceneReady(); await f.advance(0); f.metadata(); await flush();
  const plays = video.playCount;
  audio.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.playCount, plays); assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
  let gesture = false;
  const play = video.play.bind(video);
  video.play = () => { assert.equal(gesture, true); return play(); };
  gesture = true; trustedClick(audio); gesture = false;
  assert.equal(video.playCount, plays + 1); assert.equal(video.muted, false); assert.equal(audio.dataset.state, "on");
  await flush();
  audio.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.playCount, plays + 1); assert.equal(video.muted, false); assert.equal(audio.dataset.state, "on");
  trustedClick(audio); assert.equal(video.muted, true); assert.equal(audio.dataset.state, "off");
});
