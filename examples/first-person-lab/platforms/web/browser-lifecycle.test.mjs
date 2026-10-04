import assert from "node:assert/strict";
import test from "node:test";
import { BrowserLifecycle, receiveWhileCurrent } from "./browser-lifecycle.ts";

function ready(visible = true) {
  const lifecycle = new BrowserLifecycle(visible);
  lifecycle.initialized = true;
  lifecycle.markReady();
  return lifecycle;
}

test("joining waits for both WASM completion and the actual renderer callback", () => {
  for (const rendererFirst of [true, false]) {
    const lifecycle = new BrowserLifecycle(true);
    assert.equal(lifecycle.canJoin, false);
    if (rendererFirst) lifecycle.markReady();
    else lifecycle.initialized = true;
    assert.equal(lifecycle.canJoin, false);
    assert.equal(lifecycle.consumeAutoJoin(), false);
    if (rendererFirst) lifecycle.initialized = true;
    else lifecycle.markReady();
    assert.equal(lifecycle.canJoin, true);
    assert.equal(lifecycle.consumeAutoJoin(), true);
    assert.equal(lifecycle.consumeAutoJoin(), false);
  }
});

test("hidden startup and tab restoration permit manual joining but never autoconnect", () => {
  const hiddenStartup = ready(false);
  assert.equal(hiddenStartup.canJoin, false);
  hiddenStartup.resume();
  assert.equal(hiddenStartup.canJoin, true);
  assert.equal(hiddenStartup.consumeAutoJoin(), false);
  const lifecycle = ready();
  lifecycle.suspend();
  assert.equal(lifecycle.canJoin, false);
  lifecycle.resume();
  assert.equal(lifecycle.canJoin, true);
  assert.equal(lifecycle.consumeAutoJoin(), false);
});

test("fatal scenes cannot be revived by late ready callbacks or page restoration", () => {
  const lifecycle = ready();
  lifecycle.fail();
  lifecycle.markReady();
  lifecycle.resume();
  assert.equal(lifecycle.canJoin, false);
  assert.equal(lifecycle.consumeAutoJoin(), false);
});

test("explicit user actions suppress the initial automatic attempt", () => {
  const lifecycle = new BrowserLifecycle(true);
  lifecycle.cancelAutoJoin();
  lifecycle.initialized = true;
  lifecycle.markReady();
  assert.equal(lifecycle.consumeAutoJoin(), false);
});

test("a receive resolved after generation invalidation never reaches the scene", async () => {
  let generation = 1;
  const expectedGeneration = generation;
  let resolve;
  const received = new Promise((done) => { resolve = done; });
  const delivered = [];
  const loop = receiveWhileCurrent(() => generation === expectedGeneration,
    () => received, (value) => delivered.push(value));
  generation += 1;
  resolve("old room payload");
  await loop;
  assert.deepEqual(delivered, []);
});

test("current messages are delivered in order and receive failures remain failures", async () => {
  const delivered = [];
  let current = true;
  await receiveWhileCurrent(() => current, async () => "current", (value) => {
    delivered.push(value);
    current = false;
  });
  assert.deepEqual(delivered, ["current"]);
  await assert.rejects(receiveWhileCurrent(() => true, async () => {
    throw new Error("transport closed");
  }, () => {}), /transport closed/);
});
