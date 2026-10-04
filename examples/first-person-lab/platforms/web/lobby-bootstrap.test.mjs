import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { readLobbyConfig, validateLobbyBootstrap, fetchLobbyBootstrap, fetchBoundedText,
  BOOTSTRAP_TIMEOUT_MS, MAX_BOOTSTRAP_BYTES } from "./lobby-bootstrap.ts";

const ORIGIN = "https://portfolio.test";
const TARGET = { version: 1, url: "https://realtime.test:4434/webtransport",
  namespaceId: "2", sessionId: "2", spaceId: "3", spaceEpoch: "1" };
// Deliberately synthetic bytes, not a credential read from any environment.
const TOKEN = "a".repeat(64);
const config = () => readLobbyConfig("/api/lobby", JSON.stringify(TARGET), ORIGIN);
const responseText = (changes = {}) => JSON.stringify({ ...TARGET, token: TOKEN, ...changes });
const jsonResponse = (text) => new Response(text, { headers: { "content-type": "application/json; charset=utf-8" } });
const signal = () => new AbortController().signal;

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("visitor bootstrap is disabled by empty/off meta values", () => {
  assert.equal(readLobbyConfig("", "not JSON", ORIGIN), null);
  assert.equal(readLobbyConfig("off", "not JSON", ORIGIN), null);
});

test("pins one same-origin path, exact HTTPS endpoint, and exact bigint scope", () => {
  const c = config();
  assert.equal(c.path, ORIGIN + "/api/lobby");
  assert.deepEqual(validateLobbyBootstrap(responseText(), c.target), {
    ...c.target, token: TOKEN,
  });
  const max = "18446744073709551615";
  const target = { ...TARGET, namespaceId: max };
  const high = readLobbyConfig("/api/lobby", JSON.stringify(target), ORIGIN);
  assert.equal(validateLobbyBootstrap(JSON.stringify({ ...target, token: TOKEN }), high.target).namespaceId,
    18446744073709551615n);
});

test("rejects off-origin, relative, ambiguous, redirected-path, or query bootstrap paths", () => {
  for (const path of ["https://other.test/api", "//other.test/api", "api/lobby", "/a/../api", "/api?token=x",
    "/api#fragment", "/\\other.test/api", " /api", "/api\n"]) {
    assert.throws(() => readLobbyConfig(path, JSON.stringify(TARGET), ORIGIN));
  }
});

test("requires explicit allowed HTTPS target without credentials, queries or fragments", () => {
  assert.throws(() => readLobbyConfig("/api/lobby", "", ORIGIN), /valid JSON/);
  for (const url of ["http://realtime.test/webtransport", "/webtransport", "https://user:pass@realtime.test/webtransport",
    "https://realtime.test/webtransport?token=x", "https://realtime.test/webtransport#x", "https://realtime.test/",
    "https://realtime.test/webtransport?", "https://realtime.test/\\webtransport", "https://realtime.test/ path"]) {
    assert.throws(() => readLobbyConfig("/api/lobby", JSON.stringify({ ...TARGET, url }), ORIGIN));
  }
});

test("strict versioned schema rejects extra fields, bad IDs and non-spatial space IDs", () => {
  const c = config();
  for (const change of [{ version: 2 }, { version: "1" }, { ttl: 60 }, { namespaceId: 2 },
    { namespaceId: "02" }, { sessionId: "0" }, { spaceId: "2" }, { spaceEpoch: "1e3" },
    { sessionId: "+2" }, { namespaceId: "18446744073709551616" }, { token: null }]) {
    assert.throws(() => validateLobbyBootstrap(responseText(change), c.target));
  }
  for (const text of ["null", "[]", "not JSON", JSON.stringify(TARGET)]) {
    assert.throws(() => validateLobbyBootstrap(text, c.target));
  }
});

test("bootstrap cannot change allowed origin, port, path, or any scope ID", () => {
  for (const change of [{ url: "https://other.test:4434/webtransport" },
    { url: "https://realtime.test/webtransport" }, { url: "https://realtime.test:4434/other" },
    { namespaceId: "3" }, { sessionId: "3" }, { spaceId: "4" }, { spaceEpoch: "2" }]) {
    assert.throws(() => validateLobbyBootstrap(responseText(change), config().target), /configured allowed target/);
  }
});

test("managed bootstrap bearer is exactly 64 lowercase hex characters", () => {
  for (const token of ["a".repeat(63), "a".repeat(65), "A".repeat(64), "g".repeat(64), " " + TOKEN,
    "Bearer " + TOKEN, 42]) {
    assert.throws(() => validateLobbyBootstrap(responseText({ token }), config().target), /64 lowercase/);
  }
});

test("anonymous fetch omits cookies, caching, referrers and redirects", async () => {
  const calls = [];
  const result = await fetchLobbyBootstrap(config(), signal(), async (url, options) => {
    calls.push({ url, options });
    return jsonResponse(responseText());
  });
  assert.equal(result.token, TOKEN);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, ORIGIN + "/api/lobby");
  assert.equal(calls[0].options.credentials, "omit");
  assert.equal(calls[0].options.cache, "no-store");
  assert.equal(calls[0].options.redirect, "error");
  assert.equal(calls[0].options.referrerPolicy, "no-referrer");
});

test("HTTP failures, redirect responses and non-JSON bodies are rejected without body disclosure", async () => {
  const redirected = jsonResponse(responseText());
  Object.defineProperty(redirected, "redirected", { value: true });
  for (const response of [new Response(TOKEN, { status: 503 }), new Response(TOKEN), redirected]) {
    await assert.rejects(fetchLobbyBootstrap(config(), signal(), async () => response), (error) => {
      assert.equal(error.message.includes(TOKEN), false);
      return true;
    });
  }
});

test("bounds actual streaming bytes, not just Content-Length, and cancels an oversized body", async () => {
  let cancelled = false;
  const response = new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array(MAX_BOOTSTRAP_BYTES));
      controller.enqueue(new Uint8Array([1]));
    },
    cancel() { cancelled = true; },
  }), { headers: { "content-type": "application/json", "content-length": "1" } });
  await assert.rejects(fetchLobbyBootstrap(config(), signal(), async () => response), /too large/);
  assert.equal(cancelled, true);
  await assert.rejects(fetchLobbyBootstrap(config(), signal(), async () => new Response("{}", {
    headers: { "content-type": "application/json", "content-length": String(MAX_BOOTSTRAP_BYTES + 1) },
  })), /too large/);
  assert.throws(() => validateLobbyBootstrap(" ".repeat(MAX_BOOTSTRAP_BYTES + 1), config().target), /too large/);
});

test("the deadline also bounds a stalled streaming body", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const started = deferred();
  let cancelled = false;
  const response = new Response(new ReadableStream({
    pull() { started.resolve(); },
    cancel() { cancelled = true; },
  }), { headers: { "content-type": "application/json" } });
  const pending = fetchLobbyBootstrap(config(), signal(), async () => response);
  const rejected = assert.rejects(pending, /timed out/);
  await started.promise;
  t.mock.timers.tick(BOOTSTRAP_TIMEOUT_MS);
  await rejected;
  assert.equal(cancelled, true);
});

test("external pagehide cancellation aborts a bootstrap request without retry", async () => {
  const controller = new AbortController();
  let calls = 0;
  let transportSignal;
  const pending = fetchLobbyBootstrap(config(), controller.signal, async (_url, options) => {
    calls += 1;
    transportSignal = options.signal;
    return new Promise(() => {});
  });
  const rejected = assert.rejects(pending, /cancelled/);
  controller.abort();
  await rejected;
  assert.equal(transportSignal.aborted, true);
  assert.equal(calls, 1);
});

test("stream errors do not expose response contents in diagnostics", async () => {
  const response = new Response(new ReadableStream({
    start(controller) { controller.error(new Error(TOKEN)); },
  }), { headers: { "content-type": "application/json" } });
  await assert.rejects(fetchLobbyBootstrap(config(), signal(), async () => response), (error) => {
    assert.match(error.message, /could not be read/);
    assert.equal(error.message.includes(TOKEN), false);
    return true;
  });
});

test("local-token 404 remains an explicit offline result with bounded UTF-8 reads", async () => {
  const options = { signal: signal(), label: "Local credential", maxBytes: 4096, allowNotFound: true };
  assert.equal(await fetchBoundedText("./woven.local-token", options,
    async () => new Response(null, { status: 404 })), null);
  await assert.rejects(fetchBoundedText("./woven.local-token", options,
    async () => new Response(new Uint8Array([0xff]))), /UTF-8/);
});

test("browser helpers bundle in memory with existing esbuild, without generated WASM or network", async () => {
  const result = await build({ entryPoints: ["lobby-bootstrap.ts", "application-payload.ts", "browser-lifecycle.ts"],
    bundle: true, write: false, outdir: "unused", format: "esm", platform: "browser", target: "es2022" });
  assert.equal(result.outputFiles.length, 3);
  for (const output of result.outputFiles) assert.ok(output.text.length > 0);
});
