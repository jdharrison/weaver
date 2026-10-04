import assert from "node:assert/strict";
import test from "node:test";
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
        export function set_display_name() { return true; }
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

class Element extends EventTarget {
  value = "";
  textContent = "";
  disabled = true;
  hidden = false;
  open = false;
  classes = new Set();
  classList = { add: (value) => this.classes.add(value), toggle: (value, enabled) => {
    if (enabled) this.classes.add(value); else this.classes.delete(value);
  } };
  setCustomValidity() {}
  checkValidity() { return true; }
  requestSubmit() { this.dispatchEvent(new Event("submit", { cancelable: true })); }
}

function clientFixture() {
  const pending = [];
  const inbox = [];
  let closed = false;
  const startup = [{ messageKind: 9 }, { messageKind: 11, entityId: 7n }];
  const client = {
    closed: () => closed,
    supportsPositionedState: () => true,
    admitWithCancellation: async () => ({ kind: "admission", result: { status: 1 } }),
    joinSession: async () => {},
    subscribeSpace: async () => {},
    recvTimeout: async () => startup.shift(),
    recv: () => inbox.length ? Promise.resolve(inbox.shift()) : new Promise((resolve, reject) => pending.push({ resolve, reject })),
    recvDatagram: () => new Promise((_resolve, reject) => pending.push({ reject })),
    publishEvent: async () => {},
    publishUnreliablePositionedState: async () => {},
    logger: { info: async () => {} },
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

async function shell(t, options = {}) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const elements = new Map(["network-form", "connect-network", "disconnect-network", "network-status", "woven-auth",
    "woven-token", "display-name", "join-network", "reload-scene", "multiplayer-settings", "woven-url",
    "woven-namespace", "woven-session", "woven-space", "woven-epoch", "woven-certificate-hash", "status"]
    .map((id) => [id, new Element()]));
  for (const [id, value] of Object.entries({ "woven-auth": "bearer", "woven-url": TARGET.url,
    "woven-namespace": "2", "woven-session": "2", "woven-space": "3", "woven-epoch": "1" })) elements.get(id).value = value;
  const document = Object.assign(new EventTarget(), {
    visibilityState: "visible",
    getElementById: (id) => elements.get(id) ?? null,
    querySelector: (selector) => ({ content: selector.includes("weaver-lobby-bootstrap") ?
      (options.visitor === false ? "" : "/api/lobby") : JSON.stringify(TARGET) }),
  });
  const storage = new Map();
  const window = Object.assign(new EventTarget(), { isSecureContext: true, location: { origin: options.origin ?? "https://portfolio.test" },
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) } });
  const client = clientFixture();
  const harness = { events: [], connects: [], fetches: [],
    init: async () => {
      assert.equal(typeof globalThis.weaverSceneReady, "function");
      assert.equal(typeof globalThis.weaverSceneFatal, "function");
      if (options.initError) throw new Error("fixture WASM assets missing");
    },
    connect: async (config) => {
      harness.connects.push(config);
      return options.connect ? options.connect(client) : client;
    },
  };
  const globals = { window, document, WebTransport: options.noWebTransport ? undefined : function () {},
    __weaverHarness: harness, fetch: async (url, init) => {
      harness.fetches.push({ url, init });
      return options.fetch ? options.fetch(url, init, harness.fetches.length) : jsonResponse();
    } };
  const names = [...Object.keys(globals), "weaverSceneReady", "weaverSceneFatal", "weaverDisplayNameChanged",
    "weaverRealtimeFatal", "weaverRealtimePublishUnreliable", "weaverRealtimePublishPositionedUnreliable",
    "weaverRealtimePublishLatest", "weaverRealtimePublishReliable"];
  const originals = new Map(names.map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  Object.assign(globalThis, globals);
  t.after(async () => {
    window.dispatchEvent(new Event("pagehide"));
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
  return { elements, harness, client, storage, window, document, visible };
}

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
