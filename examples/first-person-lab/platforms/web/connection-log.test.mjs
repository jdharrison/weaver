import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CONNECTED_LOG_DELAY_MS,
  CONNECTED_LOG_MESSAGE,
  scheduleConnectionLog,
} from "./connection-log.ts";

test("sends the custom message once, three seconds after a successful connection", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const messages = [];
  const cancel = scheduleConnectionLog(() => true, (message) => messages.push(message));
  t.mock.timers.tick(CONNECTED_LOG_DELAY_MS - 1);
  assert.deepEqual(messages, []);
  t.mock.timers.tick(1);
  assert.deepEqual(messages, [CONNECTED_LOG_MESSAGE]);
  assert.equal(
    messages[0],
    "First-Person Lab: connected successfully; delayed client logging test (3 seconds after connection).",
  );
  t.mock.timers.tick(60_000);
  assert.equal(messages.length, 1);
  cancel();
});

test("disconnect or page hide cancels the delayed message", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const messages = [];
  const cancel = scheduleConnectionLog(() => true, (message) => messages.push(message));
  t.mock.timers.tick(2_000);
  cancel();
  cancel();
  t.mock.timers.tick(60_000);
  assert.deepEqual(messages, []);
});

test("a stale connection never queues a log even when its timer was not cancelled", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let current = true;
  const messages = [];
  scheduleConnectionLog(() => current, (message) => messages.push(message));
  current = false;
  t.mock.timers.tick(CONNECTED_LOG_DELAY_MS);
  assert.deepEqual(messages, []);
});

test("reconnecting gets a fresh delay without sending for the old connection", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const messages = [];
  const cancelOld = scheduleConnectionLog(() => true, () => messages.push("old"));
  t.mock.timers.tick(2_000);
  cancelOld();
  scheduleConnectionLog(() => true, (message) => messages.push(message));
  t.mock.timers.tick(CONNECTED_LOG_DELAY_MS - 1);
  assert.deepEqual(messages, []);
  t.mock.timers.tick(1);
  assert.deepEqual(messages, [CONNECTED_LOG_MESSAGE]);
});
