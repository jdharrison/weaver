import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { build } from "esbuild";

// A unit-test bundle only: no real Woven client, generated WASM, or output files.
const bundled = await build({ entryPoints: ["main.ts"], bundle: true, write: false,
  format: "esm", platform: "browser", target: "es2022", plugins: [{
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
        export default function init() { return globalThis.__weaverHarness.init(); }
        export function GenerateUserName() { return "BrightCalm"; }
        export function GenerateUserID() { return "a8712050-f10b-4d38-93b1-556fab8633d2"; }
        export function set_display_name(value) { globalThis.__weaverHarness.bootEvents.push(["name", value]); return true; }
        export function set_minimal_ui(value) { globalThis.__weaverHarness.bootEvents.push(["minimal", value]); }
        export function start() { globalThis.__weaverHarness.start(); }
        export function attach_theater_video(video) { globalThis.__weaverHarness.videoEvents.push(["attach", video]); return true; }
        export function mark_theater_video_frame() { globalThis.__weaverHarness.videoEvents.push(["frame"]); return true; }
        export function detach_theater_video() { globalThis.__weaverHarness.videoEvents.push(["detach"]); }
        export function set_room_lighting(lobby, auditorium) { globalThis.__weaverHarness.videoEvents.push(["light", lobby, auditorium]); return true; }
        export function pick_control_panel(x, y) { globalThis.__weaverHarness.panelPicks.push([x, y]); return globalThis.__weaverHarness.panelHit; }
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
        export function realtime_connected(id) { globalThis.__weaverHarness.events.push(["connected", id]); return true; }
        export function realtime_disconnected(reason) { globalThis.__weaverHarness.events.push(["disconnected", reason]); return true; }
        export function realtime_entity_entered(id) { globalThis.__weaverHarness.events.push(["entered", id]); return true; }
        export function realtime_entity_left(id) { globalThis.__weaverHarness.events.push(["left", id]); return true; }
        export function realtime_payload(id, seq, text) { globalThis.__weaverHarness.events.push(["payload", id, seq, text]); return true; }
        export function realtime_unreliable_payload() { return true; }
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
let moduleId = 0;

class KeyboardFixtureEvent extends Event {
  constructor(type, options = {}) {
    super(type, { cancelable: true, ...options });
    for (const [name, fallback] of Object.entries({ key: "", code: "", repeat: false, isComposing: false })) {
      this[name] = options[name] ?? fallback;
    }
  }
}

class Element extends EventTarget {
  tagName = "DIV";
  isContentEditable = false;
  blurCount = 0;
  focusCount = 0;
  focus() { this.focusCount += 1; }
  blur() { this.blurCount += 1; this.dispatchEvent(new Event("blur")); }
  value = "";
  textContent = "";
  disabled = true;
  hidden = false;
  open = false;
  classes = new Set();
  classList = { add: (value) => this.classes.add(value), toggle: (value, enabled) => {
    if (enabled) this.classes.add(value); else this.classes.delete(value);
  } };
  contains(target) { return target === this; }
  getBoundingClientRect() { return { left: 0, top: 0, right: 200, bottom: 100, width: 200, height: 100 }; }
  remove() { this.removed = true; }
  setCustomValidity() {}
  checkValidity() { return true; }
  requestSubmit() { this.dispatchEvent(new Event("submit", { cancelable: true })); }
}

function clientFixture() {
  const publishes = [];
  const pending = [];
  const inbox = [];
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
    recvDatagram: () => new Promise((_resolve, reject) => pending.push({ reject })),
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
      for (const waiter of pending.splice(0)) waiter.reject(new Error("fixture closed"));
    },
    send: (value) => {
      const index = pending.findIndex((waiter) => waiter.resolve);
      if (index === -1) inbox.push(value);
      else pending.splice(index, 1)[0].resolve(value);
    },
  };
  return client;
}

class VideoElement extends Element {
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
  load() {}
  removeAttribute(name) { if (name === "src") this.src = ""; }
  pause() { this.paused = true; this.pauseCount += 1; }
  play() {
    this.playCount += 1;
    if (this.playError) return Promise.reject(this.playError);
    this.paused = false; this.dispatchEvent(new Event("playing")); return Promise.resolve();
  }
}

async function shell(t, options = {}) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0;
  t.mock.method(Date, "now", () => 0);
  t.mock.method(performance, "now", () => now);
  const elements = new Map(["network-form", "connect-network", "disconnect-network", "network-status", "woven-auth",
    "woven-token", "display-name", "join-network", "reload-scene", "multiplayer-settings", "woven-url",
    "woven-namespace", "woven-session", "woven-space", "woven-epoch", "woven-certificate-hash", "status", "scene",
        "theater-controls", "theater-play", "theater-restart", "theater-mute", "theater-volume", "theater-seek", "theater-time", "theater-status",
        "theater-lobby-light", "theater-auditorium-light", "theater-follow-schedule", "theater-schedule-status"]
    .map((id) => [id, new Element()]));
  for (const [id, value] of Object.entries({ "woven-auth": "bearer", "woven-url": TARGET.url,
    "woven-namespace": "2", "woven-session": "2", "woven-space": "3", "woven-epoch": "1",
        "theater-lobby-light": "1", "theater-auditorium-light": "0.18" })) elements.get(id).value = value;
  elements.set("theater-video", new VideoElement());
  const rafs = new Map();
  let nextRaf = 0;
  const document = Object.assign(new EventTarget(), {
    visibilityState: "visible",
    activeElement: null,
    pointerLockElement: null,
    pointerLockExits: 0,
    exitPointerLock() { this.pointerLockExits += 1; this.pointerLockElement = null; },
    documentElement: { dataset: {
      ...(options.presentation === undefined ? {} : { weaverPresentation: options.presentation }),
      ...(options.publishRateHz === undefined ? {} : { weaverPublishRateHz: options.publishRateHz }),
    } },
    getElementById: (id) => elements.get(id) ?? null,
    querySelector: (selector) => selector.includes("weaver-theater-schedule") ? { content: "/weaver/theater-schedule.json" } : selector.includes("weaver-theater-video") ? { content: "" } : ({ content: selector.includes("weaver-lobby-bootstrap") ?
      (options.visitor === false ? "" : "/api/lobby") : JSON.stringify(TARGET) }),
  });
  const storage = new Map();
  const window = Object.assign(new EventTarget(), { isSecureContext: true, location: { origin: options.origin ?? "https://portfolio.test" },
    requestAnimationFrame: (callback) => { rafs.set(++nextRaf, callback); return nextRaf; },
    cancelAnimationFrame: (id) => rafs.delete(id),
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) } });
  const client = clientFixture();
  const clients = [];
  const harness = { events: [], connects: [], fetches: [], bootEvents: [], startState: null,
    panelPicks: [], panelStates: [], panelHit: 1, panelInvalidations: 0,
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
        globalThis.weaverChatMessage("7", "BrightCalm", text, true);
        globalThis.weaverRealtimePublishReliable(harness.nextChatSequence++, JSON.stringify({
          kind: "chat", version: 1, display_name: "BrightCalm", text,
        }));
      }
    },
    init: async () => {
      harness.bootEvents.push(["init"]);
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
  window.parent = { postMessage: (data, origin) => harness.progress.push({ ...JSON.parse(data), origin }) };
  const globals = { window, document, HTMLElement: Element, Node: Element, KeyboardEvent: KeyboardFixtureEvent, WebTransport: options.noWebTransport ? undefined : function () {},
    __weaverHarness: harness, fetch: async (url, init) => {
      if (url.endsWith("/weaver/theater-schedule.json")) {
        return options.scheduleFetch ? options.scheduleFetch(url, init) : new Response(JSON.stringify({
          version: 1, revision: "fixture", anchorUnixSeconds: 0, cycleSeconds: 3600, driftToleranceSeconds: 5,
          entries: [{ id: "jazz", title: "Space Noir Jazz", startSeconds: 0, durationSeconds: 3600,
            source: "/weaver/assets/theater/Music_SpaceNoirJazz.mp4", sourceStartSeconds: 0, loop: false }],
        }), { headers: { "content-type": "application/json" } });
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
  return { elements, harness, client, clients, storage, window, document, visible, advance, rafs };
}

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
    await flush();
    assert.deepEqual(harness.npcCycles, []);
    assert.deepEqual(harness.npcActivations, []);
    assert.equal(globalThis.weaverGetNpcInteractionContext(), null);
    assert.equal(globalThis.weaverSubmitNpcQuestion("concession_support", "too_early", "hello"), false);
  } });
  await flush();
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
  globalThis.weaverSceneReady(); await flush();
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

test("sound HTML is absent; minimal presentation retains hidden diagnostic source controls", () => {
  const html = readFileSync("index.html", "utf8");
  const css = readFileSync("styles.css", "utf8");
  assert.doesNotMatch(html + css, /theater-enable-sound|theater-sound-access/);
  assert.match(html, /id="theater-controls"/);
  assert.match(css, /html\[data-weaver-presentation="minimal"\] \.theater-controls \{ display: none; \}/);
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

test("schedule status and controls are independent; bootstrap-shaped manifest fails closed without breaking scene", async (t) => {
  const f = await shell(t, { scheduleFetch: async () => jsonResponse() });
  const video = f.elements.get("theater-video");
  video.readyState = 2; video.dispatchEvent(new Event("loadedmetadata"));
  globalThis.weaverSceneReady(); await flush();
  assert.match(f.elements.get("theater-schedule-status").textContent, /Schedule unavailable/);
  assert.equal(video.playCount, 0); assert.equal(f.elements.get("theater-follow-schedule").checked, true);
  assert.equal(f.elements.get("theater-restart").disabled, true);
  assert.equal(f.elements.get("theater-seek").disabled, true);
  assert.equal(f.elements.get("theater-play").disabled, false);
  f.elements.get("theater-play").dispatchEvent(new Event("click")); assert.equal(video.playCount, 0);
  const follow = f.elements.get("theater-follow-schedule"); follow.checked = false; follow.dispatchEvent(new Event("change"));
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
  globalThis.weaverSceneReady(); await flush();
  assert.deepEqual(f.harness.videoEvents.map(([event]) => event), ["attach", "frame"]);
  globalThis.weaverSceneReady();
  assert.equal(f.harness.videoEvents.filter(([event]) => event === "attach").length, 1);
  assert.equal(video.playCount, 1);
  assert.equal(video.autoplay, false);
  assert.equal(video.src, "/weaver/assets/theater/Music_SpaceNoirJazz.mp4");
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
  globalThis.weaverSceneReady(); await flush();
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
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
  globalThis.weaverSceneReady(); await flush();
  const play = f.elements.get("theater-play");
  assert.equal(video.playCount, 1); assert.equal(video.muted, true);
  assert.equal(play.disabled, false); assert.equal(play.textContent, "Play");
  assert.equal(f.elements.get("theater-status").textContent, "Autoplay blocked/unavailable · press Play");
  globalThis.weaverSceneReady();
  for (const event of ["loadedmetadata", "loadeddata", "canplay", "timeupdate"]) video.dispatchEvent(new Event(event));
  await flush(); assert.equal(video.playCount, 1);
  video.playError = null;
  play.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.playCount, 2); assert.equal(video.paused, false); assert.equal(video.muted, true);
  assert.equal(play.textContent, "Pause");
  await f.visible("hidden"); await f.visible("visible");
  assert.equal(video.playCount, 2); assert.equal(video.paused, true);
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
  globalThis.weaverSceneReady(); await flush();
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
  assert.equal(video.paused, true); assert.equal(video.volume, 0.25);
  play.dispatchEvent(new Event("click")); await flush();
  assert.equal(video.muted, true);
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
    duringInit = [...harness.videoEvents];
  } });
  assert.deepEqual(duringInit, []);
  assert.deepEqual(f.harness.videoEvents.map(([event]) => event), ["attach", "frame"]);
});

test("theater focus composes with host chat focus without reviving game input", async (t) => {
  const f = await shell(t, { presentation: "minimal" }); globalThis.weaverSceneReady(); await flush();
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
  globalThis.weaverSceneReady(); await flush();
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
async function online(t, options = {}) {
  const f = await shell(t, options);
  globalThis.weaverSceneReady();
  await flush();
  assert.match(f.elements.get("network-status").textContent, /Online/);
  return f;
}

function pose(sequence, payload = new Uint8Array(25)) {
  globalThis.weaverRealtimePublishPositionedUnreliable(BigInt(sequence), 1, 2, 3, payload);
}

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
  const payload = new Uint8Array(25).fill(8);
  pose(8, payload);
  payload.fill(99); // The bridge must not retain a view into caller/WASM memory.
  await f.advance(259);
  assert.equal(f.client.publishes.length, 1);
  await f.advance(1);
  for (let i = 0; i < 3; i += 1) await f.advance(260);
  assert.deepEqual(f.client.publishes.map(({ kind, sequence, at }) => [kind, sequence, at]), [
    ["event", 1n, 0], ["event", 4n, 260], ["event", 5n, 520], ["event", 6n, 780], ["pose", 8n, 1_040],
  ]);
  assert.deepEqual(f.client.publishes.at(-1).payload, new Uint8Array(25).fill(8));
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
  assert.match(f.elements.get("network-status").textContent, /startup timed out/);
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
  return [["init"], ["initialized"], ["minimal", minimal], ["name", "BrightCalm"], ["start"]];
}

for (const [presentation, minimal] of [[undefined, false], ["minimal", true], ["immersive", false],
  ["full", false], ["Minimal", false], ["", false]]) {
  test("presentation is set before scene start: " + (presentation === undefined ? "absent marker" : JSON.stringify(presentation)), async (t) => {
    const f = await shell(t, { presentation });
    assert.deepEqual(f.harness.bootEvents, expectedBoot(minimal));
    assert.deepEqual(f.harness.startState, { joinDisabled: true, fetchCount: 0, connectionCount: 0 });
    assert.equal(f.elements.get("display-name").value, "BrightCalm");
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
  if (options.target) Object.defineProperty(event, "composedPath", { value: () => [options.target, f.window] });
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
  assert.deepEqual(messages, [["7", "BrightCalm", "hello 🌍", true], remote]);
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
    assert.deepEqual(messages, [["7", "BrightCalm", "fresh", true]]);
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
  assert.deepEqual(messages, [["7", "BrightCalm", "submitted before close", true]]);
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
