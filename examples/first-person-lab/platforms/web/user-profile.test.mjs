import assert from "node:assert/strict";
import test from "node:test";
import { USER_PROFILE_KEY, loadUserProfile, saveUserProfile } from "./user-profile.ts";

const ID = "a8712050-f10b-4d38-93b1-556fab8633d2";
const NAME = "BrightCalm";

function fixture(initial = null) {
  const values = new Map(initial === null ? [] : [[USER_PROFILE_KEY, initial]]);
  const calls = { name: 0, id: 0 };
  return {
    storage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
    },
    generators: {
      generateUserName: () => { calls.name += 1; return NAME; },
      generateUserId: () => { calls.id += 1; return ID; },
    },
    calls,
    values,
  };
}

test("generates and stores a guest profile only once across reloads", () => {
  const f = fixture();
  const profile = loadUserProfile(f.storage, f.generators);
  assert.deepEqual(profile, { userId: ID, displayName: NAME });
  assert.deepEqual(loadUserProfile(f.storage, f.generators), profile);
  assert.deepEqual(f.calls, { name: 1, id: 1 });
});

test("renaming keeps the stored UUID stable", () => {
  const f = fixture();
  const profile = loadUserProfile(f.storage, f.generators);
  assert.equal(saveUserProfile(f.storage, { ...profile, displayName: "AgileSunny" }), true);
  assert.deepEqual(loadUserProfile(f.storage, f.generators), { userId: ID, displayName: "AgileSunny" });
  assert.deepEqual(f.calls, { name: 1, id: 1 });
});

test("invalid names regenerate without changing a valid ID", () => {
  for (const displayName of ["", " Guest", "bad\nname", "x".repeat(25), 42, null]) {
    const f = fixture(JSON.stringify({ userId: ID, displayName }));
    assert.deepEqual(loadUserProfile(f.storage, f.generators), { userId: ID, displayName: NAME });
    assert.deepEqual(f.calls, { name: 1, id: 0 });
  }
});

test("invalid IDs regenerate without changing a valid name", () => {
  for (const userId of ["", ID.toUpperCase(), ID.replace("-4d38-", "-1d38-"), "entity-7", null]) {
    const f = fixture(JSON.stringify({ userId, displayName: "AgileSunny" }));
    assert.deepEqual(loadUserProfile(f.storage, f.generators), { userId: ID, displayName: "AgileSunny" });
    assert.deepEqual(f.calls, { name: 0, id: 1 });
  }
});

test("corrupt, oversized, or non-object storage recovers safely", () => {
  for (const text of ["not json", "null", "[]", "42", "x".repeat(513)]) {
    const f = fixture(text);
    assert.deepEqual(loadUserProfile(f.storage, f.generators), { userId: ID, displayName: NAME });
  }
});

test("disabled storage and quota failures do not prevent an in-memory profile", () => {
  const blocked = {
    getItem() { throw new Error("Storage blocked"); },
    setItem() { throw new Error("Quota exceeded"); },
  };
  for (const storage of [blocked, null]) {
    const f = fixture();
    assert.deepEqual(loadUserProfile(storage, f.generators), { userId: ID, displayName: NAME });
    assert.equal(saveUserProfile(storage, { userId: ID, displayName: NAME }), false);
  }
});

test("saving invalid fields cannot replace a valid profile", () => {
  const f = fixture(JSON.stringify({ userId: ID, displayName: NAME }));
  const original = f.values.get(USER_PROFILE_KEY);
  assert.equal(saveUserProfile(f.storage, { userId: "not-a-uuid", displayName: NAME }), false);
  assert.equal(saveUserProfile(f.storage, { userId: ID, displayName: "bad\nname" }), false);
  assert.equal(f.values.get(USER_PROFILE_KEY), original);
});
