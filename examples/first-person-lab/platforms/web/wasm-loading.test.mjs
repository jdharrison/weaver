import assert from "node:assert/strict";
import test from "node:test";
import { fetchWasmWithProgress, MAX_WASM_BYTES } from "./wasm-loading.ts";

const url = new URL("https://site.test/weaver/first-person-lab/pkg/first_person_lab_bg.wasm");
const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
function fixture(response, overrides = {}) {
  const abort = new AbortController(), updates = [], calls = [];
  let completed = 0, activity = 0, clock = 0;
  const options = {
    signal: abort.signal,
    fetcher: async (...args) => { calls.push(args); return response; },
    onDownload: (loaded, total) => updates.push([loaded, total]),
    onActivity: () => activity++, onComplete: () => completed++,
    now: () => clock,
    ...overrides,
  };
  return { abort, updates, calls, options, complete: () => completed, activity: () => activity,
    advance: ms => { clock += ms; } };
}

test("one streamed response compiles WASM without cloning or collecting the input body", async () => {
  const response = new Response(bytes, { headers: { "content-type": "application/wasm", "content-length": "8" } });
  for (const method of ["clone", "arrayBuffer", "text", "blob"]) response[method] = () => { throw new Error("Must stream"); };
  const f = fixture(response);
  const observed = await fetchWasmWithProgress(url, f.options);
  assert.equal(observed.headers.get("content-type"), "application/wasm");
  const module = await WebAssembly.compileStreaming(observed);
  assert.ok(module instanceof WebAssembly.Module);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0][0], url);
  assert.equal(f.calls[0][1].signal, f.abort.signal);
  assert.deepEqual(f.updates, [[0, 8], [8, 8]]);
  assert.equal(f.complete(), 1);
  assert.equal(f.activity(), 1);
});

test("reports actual advancing chunks at a bounded cadence and always reports final bytes", async () => {
  let source;
  const f = fixture(new Response(new ReadableStream({ start(controller) { source = controller; } }), {
    headers: { "content-type": "application/wasm", "content-length": "8" },
  }));
  const response = await fetchWasmWithProgress(url, f.options);
  const reader = response.body.getReader();
  source.enqueue(bytes.subarray(0, 2)); await reader.read();
  assert.deepEqual(f.updates, [[0, 8]]);
  f.advance(100);
  source.enqueue(bytes.subarray(2, 4)); await reader.read();
  assert.deepEqual(f.updates, [[0, 8], [4, 8]]);
  source.enqueue(bytes.subarray(4)); await reader.read();
  source.close(); await reader.read();
  assert.deepEqual(f.updates, [[0, 8], [4, 8], [8, 8]]);
  assert.equal(f.complete(), 1);
  assert.equal(f.activity(), 3);
});

for (const headers of [
  {}, { "content-length": "nonsense" }, { "content-length": "-1" },
  { "content-length": "0" }, { "content-length": String(MAX_WASM_BYTES + 1) },
  { "content-length": "3", "content-encoding": "gzip" },
  { "content-length": "3", "content-encoding": "br" },
]) test("unknown/encoded lengths never pretend to be decoded totals: " + JSON.stringify(headers), async () => {
  const f = fixture(new Response(bytes, { headers: { ...headers, "content-type": "application/wasm" } }));
  const response = await fetchWasmWithProgress(url, f.options);
  assert.equal(response.headers.get("content-encoding"), null);
  assert.equal(response.headers.get("content-length"), null);
  assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
  assert.deepEqual(f.updates, [[0, null], [8, null]]);
});

for (const length of ["3", "10"]) test("contradictory length is discarded, never clamped or fabricated: " + length, async () => {
  const f = fixture(new Response(bytes, { headers: { "content-length": length } }));
  await (await fetchWasmWithProgress(url, f.options)).arrayBuffer();
  assert.deepEqual(f.updates.at(-1), [8, null]);
});

test("HTTP failure cancels its body and does not report download/initialization", async () => {
  let cancelled = false;
  const f = fixture(new Response(new ReadableStream({ cancel() { cancelled = true; } }), { status: 404 }));
  await assert.rejects(fetchWasmWithProgress(url, f.options), /WASM request failed \(404\)/);
  assert.equal(cancelled, true); assert.deepEqual(f.updates, []); assert.equal(f.complete(), 0);
});

test("stream failure propagates without reporting initialization", async () => {
  const f = fixture(new Response(new ReadableStream({ start(controller) { controller.error(new Error("connection lost")); } })));
  await assert.rejects((await fetchWasmWithProgress(url, f.options)).arrayBuffer(), /connection lost/);
  assert.equal(f.complete(), 0);
});

test("aborting a stalled stream cancels the input and prevents late initialization", async () => {
  let cancelled = false;
  const f = fixture(new Response(new ReadableStream({ cancel() { cancelled = true; } })));
  const reading = (await fetchWasmWithProgress(url, f.options)).arrayBuffer();
  const rejected = assert.rejects(reading, { name: "AbortError" });
  f.abort.abort(); await rejected;
  assert.equal(cancelled, true); assert.equal(f.complete(), 0);
  assert.deepEqual(f.updates, [[0, null]]);
});

test("consumer cancellation propagates without consuming the rest of the movie-sized response", async () => {
  let cancelled = false;
  const f = fixture(new Response(new ReadableStream({ cancel() { cancelled = true; } })));
  const response = await fetchWasmWithProgress(url, f.options);
  await response.body.cancel();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(cancelled, true); assert.equal(f.complete(), 0);
});

test("oversized responses fail before forwarding the offending chunk", async () => {
  const f = fixture(new Response(new ReadableStream({
    start(controller) { controller.enqueue({ byteLength: MAX_WASM_BYTES + 1 }); controller.close(); },
  })));
  await assert.rejects((await fetchWasmWithProgress(url, f.options)).arrayBuffer(), /size limit/);
  assert.equal(f.complete(), 0);
});

test("missing readable body supplies no invented completion or byte count", async () => {
  const original = new Response(null);
  const f = fixture(original);
  assert.equal(await fetchWasmWithProgress(url, f.options), original);
  assert.deepEqual(f.updates, [[0, null]]); assert.equal(f.complete(), 0);
});
