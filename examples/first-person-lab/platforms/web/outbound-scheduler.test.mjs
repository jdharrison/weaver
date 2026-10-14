import assert from "node:assert/strict";
import test from "node:test";
import { OutboundScheduler, parsePublishRateHz, PUBLISH_RATE_CONFIGURATION_ERROR } from "./outbound-scheduler.ts";

const flush = async () => { for (let i = 0; i < 20; i += 1) await Promise.resolve(); };

function fixture(t, rateHz = 4, send) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0;
  let current = true;
  const queue = [1, 2, 3];
  const attempts = [];
  const errors = [];
  const scheduler = new OutboundScheduler({
    rateHz,
    now: () => now,
    isCurrent: () => current,
    hasPending: () => queue.length > 0,
    sendNext: async () => {
      attempts.push({ value: queue.shift(), at: now });
      if (send) await send();
    },
    onError: (error) => errors.push(error),
  });
  t.after(() => scheduler.stop());
  return { scheduler, queue, attempts, errors,
    obsolete: () => { current = false; },
    setNow: (value) => { now = value; },
    advance: async (ms) => { now += ms; t.mock.timers.tick(ms); await flush(); },
  };
}

test("rate marker accepts every canonical integer 1..60 and leaves absent demos unpaced", () => {
  assert.equal(parsePublishRateHz(undefined), null);
  for (let rate = 1; rate <= 60; rate += 1) assert.equal(parsePublishRateHz(String(rate)), rate);
});

test("invalid rate markers fail with one fixed sanitized configuration error", () => {
  for (const value of ["", "0", "61", "-1", "4.5", "4.0", "04", " 4", "4 ", "4\n", "4\r\n", "4e0",
    "NaN", "Infinity", "+4", "0x04", "fixture-secret", "999999999999999999999"]) {
    assert.throws(() => parsePublishRateHz(value), (error) => error.message === PUBLISH_RATE_CONFIGURATION_ERROR);
  }
});

test("fractional spacing rounds timers up and never spends a send slot early", async (t) => {
  const f = fixture(t, 60);
  f.scheduler.kick();
  await flush();
  assert.deepEqual(f.attempts, [{ value: 1, at: 0 }]);
  await f.advance(26);
  assert.equal(f.attempts.length, 1);
  await f.advance(1);
  assert.deepEqual(f.attempts, [{ value: 1, at: 0 }, { value: 2, at: 27 }]);
  await f.advance(26);
  assert.equal(f.attempts.length, 2);
  await f.advance(1);
  assert.deepEqual(f.attempts.at(-1), { value: 3, at: 54 });
});

test("an early timer callback rechecks monotonic time and rearms the remaining gap", async (t) => {
  const f = fixture(t);
  f.scheduler.kick();
  await flush();
  f.setNow(200);
  t.mock.timers.tick(260); // Deliberately fire the timer before the injected monotonic deadline.
  await flush();
  assert.equal(f.attempts.length, 1);
  await f.advance(59);
  assert.equal(f.attempts.length, 1);
  await f.advance(1);
  assert.deepEqual(f.attempts.at(-1), { value: 2, at: 260 });
});

test("a late timer sends once, without a catch-up burst", async (t) => {
  const f = fixture(t);
  f.scheduler.kick();
  await flush();
  await f.advance(2_000);
  assert.deepEqual(f.attempts, [{ value: 1, at: 0 }, { value: 2, at: 2_000 }]);
  await f.advance(259);
  assert.equal(f.attempts.length, 2);
  await f.advance(1);
  assert.deepEqual(f.attempts.at(-1), { value: 3, at: 2_260 });
});

test("only one write is in flight and a slow write does not create catch-up credit", async (t) => {
  let finish;
  let calls = 0;
  const f = fixture(t, 4, () => ++calls === 1 ? new Promise((resolve) => { finish = resolve; }) : undefined);
  f.scheduler.kick();
  f.scheduler.kick();
  await flush();
  await f.advance(1_000);
  f.scheduler.kick();
  assert.equal(f.attempts.length, 1);
  finish();
  await flush();
  assert.deepEqual(f.attempts, [{ value: 1, at: 0 }, { value: 2, at: 1_000 }]);
  await f.advance(259);
  assert.equal(f.attempts.length, 2);
  await f.advance(1);
  assert.deepEqual(f.attempts.at(-1), { value: 3, at: 1_260 });
});

test("an empty queue retains the previous deadline when more work arrives", async (t) => {
  const f = fixture(t);
  f.queue.length = 1;
  f.scheduler.kick();
  await flush();
  await f.advance(100);
  f.queue.push(2);
  f.scheduler.kick();
  f.scheduler.kick();
  await f.advance(159);
  assert.equal(f.attempts.length, 1);
  await f.advance(1);
  assert.deepEqual(f.attempts.at(-1), { value: 2, at: 260 });
});

for (const obsolete of [false, true]) {
  test((obsolete ? "obsolete identity" : "explicit stop") + " suppresses scheduled sends and cannot be restarted", async (t) => {
    const f = fixture(t);
    f.scheduler.kick();
    await flush();
    if (obsolete) f.obsolete(); else f.scheduler.stop();
    await f.advance(2_000);
    f.scheduler.kick();
    await flush();
    assert.equal(f.attempts.length, 1);
    assert.equal(f.errors.length, 0);
  });
}

test("a current failed write stops the queue and reports the failure exactly once", async (t) => {
  const error = new Error("fixture write failed");
  const f = fixture(t, 4, () => Promise.reject(error));
  f.scheduler.kick();
  await flush();
  await f.advance(2_000);
  f.scheduler.kick();
  assert.equal(f.attempts.length, 1);
  assert.deepEqual(f.errors, [error]);
});

test("a late obsolete write failure cannot report an error or drain the old queue", async (t) => {
  let fail;
  const f = fixture(t, 4, () => new Promise((_resolve, reject) => { fail = reject; }));
  f.scheduler.kick();
  f.obsolete();
  fail(new Error("fixture old write failed"));
  await flush();
  await f.advance(2_000);
  assert.equal(f.attempts.length, 1);
  assert.equal(f.errors.length, 0);
});

test("an absent marker has no timer delay while retaining write serialization", async (t) => {
  const f = fixture(t, null);
  f.scheduler.kick();
  await flush();
  assert.deepEqual(f.attempts, [{ value: 1, at: 0 }, { value: 2, at: 0 }, { value: 3, at: 0 }]);
});
