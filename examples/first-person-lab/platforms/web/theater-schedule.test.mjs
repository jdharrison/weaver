import assert from "node:assert/strict";
import test from "node:test";
import { validateTheaterSchedule, resolveTheaterSchedule } from "./theater-schedule.ts";

function fixture() {
  return {
    version: 1, revision: "program-1", anchorUnixSeconds: 1709164800,
    cycleSeconds: 60, driftToleranceSeconds: 0.5,
    entries: [
      { id: "intro", title: "Introduction", startSeconds: 0, durationSeconds: 20,
        source: "/assets/theater/intro.mp4", sourceStartSeconds: 2.5, loop: false },
      { id: "feature", title: "Feature", startSeconds: 20, durationSeconds: 40,
        source: "/assets/theater/feature.webm", sourceStartSeconds: 5, loop: true },
    ],
  };
}

function invalid(mutator) {
  const value = fixture();
  mutator(value);
  assert.throws(() => validateTheaterSchedule(value), { message: "Invalid theater schedule." });
}

function at(schedule, delta) {
  return resolveTheaterSchedule(schedule, schedule.anchorUnixSeconds + delta);
}

test("half-open entry boundaries, exact ends, cycle wrap, and fractional time", () => {
  const schedule = validateTheaterSchedule(fixture());
  for (const [delta, id, cycleIndex, elapsedSeconds, mediaTimeSeconds, remainingSeconds] of [
    [0, "intro", 0, 0, 2.5, 20],
    [19.75, "intro", 0, 19.75, 22.25, 0.25],
    [20, "feature", 0, 0, 5, 40],
    [59.75, "feature", 0, 39.75, 44.75, 0.25],
    [60, "intro", 1, 0, 2.5, 20],
    [140, "feature", 2, 0, 5, 40],
    [-0.25, "feature", -1, 39.75, 44.75, 0.25],
    [-40, "feature", -1, 0, 5, 40],
    [-60, "intro", -1, 0, 2.5, 20],
    [-60.25, "feature", -2, 39.75, 44.75, 0.25],
  ]) {
    const result = at(schedule, delta);
    assert.equal(result.entry, schedule.entries.find((entry) => entry.id === id));
    assert.deepEqual({ ...result, entry: id }, { entry: id, cycleIndex, elapsedSeconds, mediaTimeSeconds, remainingSeconds });
  }
});

test("independent devices agree; drift tolerance does not alter wall-clock resolution", () => {
  const first = validateTheaterSchedule(fixture());
  const second = validateTheaterSchedule(JSON.parse(JSON.stringify(fixture())));
  for (const delta of [-12345.5, 0, 20, 987654.25]) {
    assert.deepEqual(at(first, delta), at(second, delta));
  }
  assert.equal(at(first, 20.25).elapsedSeconds, 0.25);
  assert.equal(at(second, 21).elapsedSeconds - at(first, 20.25).elapsedSeconds, 0.75);
  const differentTolerance = fixture();
  differentTolerance.driftToleranceSeconds = 30;
  assert.deepEqual(at(first, 20.25), at(validateTheaterSchedule(differentTolerance), 20.25));
});

test("leap day, DST offsets, time zones, and overnight cycles use only Unix time", () => {
  const schedule = validateTheaterSchedule(fixture());
  const timestamp = (iso) => Date.parse(iso) / 1000;
  assert.equal(schedule.anchorUnixSeconds, timestamp("2024-02-29T00:00:00Z"));
  assert.equal(resolveTheaterSchedule(schedule, timestamp("2024-03-01T00:00:00Z")).cycleIndex, 1440);
  assert.deepEqual(
    resolveTheaterSchedule(schedule, timestamp("2024-03-10T01:59:50-05:00")),
    resolveTheaterSchedule(schedule, timestamp("2024-03-10T06:59:50Z")),
  );
  const before = resolveTheaterSchedule(schedule, timestamp("2024-03-10T01:59:50-05:00"));
  const after = resolveTheaterSchedule(schedule, timestamp("2024-03-10T03:00:10-04:00"));
  assert.equal(before.entry.id, "feature");
  assert.equal(after.entry.id, "intro");
  assert.equal(after.cycleIndex, before.cycleIndex + 1);
  assert.deepEqual(
    resolveTheaterSchedule(schedule, timestamp("2024-11-03T01:30:00-04:00")),
    resolveTheaterSchedule(schedule, timestamp("2024-11-03T05:30:00Z")),
  );
  const overnight = fixture();
  overnight.anchorUnixSeconds = timestamp("2024-02-29T23:59:40Z");
  const result = resolveTheaterSchedule(validateTheaterSchedule(overnight), timestamp("2024-03-01T00:00:10Z"));
  assert.equal(result.entry.id, "feature");
  assert.equal(result.elapsedSeconds, 10);
  assert.equal(result.cycleIndex, 0);
});

test("clock rejects nonfinite and unsafe absolute or relative seconds", () => {
  const schedule = validateTheaterSchedule(fixture());
  for (const value of [NaN, Infinity, -Infinity, Number.MAX_VALUE, Number.MAX_SAFE_INTEGER + 1, -Number.MAX_SAFE_INTEGER]) {
    assert.throws(() => resolveTheaterSchedule(schedule, value), { message: "Invalid theater schedule time." });
  }
  const value = fixture();
  value.anchorUnixSeconds = 0;
  const epoch = validateTheaterSchedule(value);
  assert.equal(resolveTheaterSchedule(epoch, 0).cycleIndex, 0);
  assert.equal(resolveTheaterSchedule(epoch, -0).cycleIndex, 0);
  assert.equal(resolveTheaterSchedule(epoch, 0.125).elapsedSeconds, 0.125);
  for (const unix of [Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER]) {
    const result = resolveTheaterSchedule(epoch, unix);
    const divisor = BigInt(epoch.cycleSeconds);
    const seconds = BigInt(unix);
    const remainder = ((seconds % divisor) + divisor) % divisor;
    assert.equal(result.cycleIndex, Number((seconds - remainder) / divisor));
    assert.equal(result.entry.startSeconds + result.elapsedSeconds, Number(remainder));
  }
});

test("safe integer extremes retain correct floor and modulo for every supported cycle length", () => {
  for (let cycleSeconds = 1; cycleSeconds <= 3600; cycleSeconds++) {
    const value = fixture();
    value.anchorUnixSeconds = 0;
    value.cycleSeconds = cycleSeconds;
    value.entries = [{ ...value.entries[0], durationSeconds: cycleSeconds }];
    const schedule = validateTheaterSchedule(value);
    for (const unixSeconds of [Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER]) {
      const seconds = BigInt(unixSeconds);
      const divisor = BigInt(cycleSeconds);
      const remainder = ((seconds % divisor) + divisor) % divisor;
      const result = resolveTheaterSchedule(schedule, unixSeconds);
      assert.equal(result.cycleIndex, Number((seconds - remainder) / divisor));
      assert.equal(result.elapsedSeconds, Number(remainder));
    }
  }
});

test("schema is exact and accepts only supported version and bounded scalars", () => {
  for (const value of [null, undefined, [], 1, "private input", {}]) {
    assert.throws(() => validateTheaterSchedule(value), { message: "Invalid theater schedule." });
  }
  for (const key of Object.keys(fixture())) invalid((value) => { delete value[key]; });
  for (const key of Object.keys(fixture().entries[0])) invalid((value) => { delete value.entries[0][key]; });
  invalid((value) => { value.extra = "untrusted"; });
  invalid((value) => { value[Symbol("hidden")] = true; });
  invalid((value) => { value.entries[0].extra = true; });
  invalid((value) => { value.version = 2; });
  invalid((value) => { value.version = "1"; });
  for (const key of ["revision", "id"]) {
    for (const bad of ["", "a".repeat(129), "with spaces", "a/b", "x\n", "é", 1]) {
      invalid((value) => { (key === "revision" ? value : value.entries[0])[key] = bad; });
    }
  }
  for (const bad of ["", " leading", "trailing ", "a".repeat(101), "x\u0000", "x\u007f", "x\u0085", "x\u202e", "\ud800", 1]) {
    invalid((value) => { value.entries[0].title = bad; });
  }
  const unicode = fixture();
  unicode.entries[0].title = "🎬".repeat(100);
  unicode.revision = "a".repeat(128);
  unicode.entries[0].id = "b".repeat(128);
  assert.equal(validateTheaterSchedule(unicode).entries[0].title, unicode.entries[0].title);
  for (const [key, badValues] of [
    ["anchorUnixSeconds", [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, "0"]],
    ["cycleSeconds", [0, -1, 3601, 1.5, NaN, Infinity, "60"]],
    ["driftToleranceSeconds", [-0.01, 30.01, NaN, Infinity, "0"]],
  ]) for (const bad of badValues) invalid((value) => { value[key] = bad; });
  for (const bad of [-1, 86400.01, NaN, Infinity, "0"]) invalid((value) => { value.entries[0].sourceStartSeconds = bad; });
  for (const bad of [0, 1, "true", null]) invalid((value) => { value.entries[0].loop = bad; });
  const limits = fixture();
  limits.anchorUnixSeconds = Number.MAX_SAFE_INTEGER;
  limits.cycleSeconds = 3600;
  limits.driftToleranceSeconds = 30;
  limits.entries[1].durationSeconds = 3580;
  limits.entries[0].sourceStartSeconds = 86400;
  assert.equal(validateTheaterSchedule(limits).cycleSeconds, 3600);
});

test("entries are bounded, dense, unique, ordered and contiguous with exact cycle total", () => {
  for (const entries of [[], null, {}, Array(2)]) invalid((value) => { value.entries = entries; });
  invalid((value) => { value.entries.extra = true; });
  invalid((value) => { value.entries[1].id = value.entries[0].id; });
  invalid((value) => { value.entries.reverse(); });
  for (const bad of [1, -1, 0.5, NaN, Infinity]) invalid((value) => { value.entries[0].startSeconds = bad; });
  for (const bad of [19, 21]) invalid((value) => { value.entries[1].startSeconds = bad; });
  for (const bad of [0, -1, 1.5, NaN, Infinity, 61]) invalid((value) => { value.entries[0].durationSeconds = bad; });
  for (const bad of [39, 41]) invalid((value) => { value.entries[1].durationSeconds = bad; });
  const maximum = fixture();
  maximum.cycleSeconds = 120;
  maximum.entries = Array.from({ length: 120 }, (_, index) => ({ ...fixture().entries[0], id: `entry-${index}`, startSeconds: index, durationSeconds: 1 }));
  assert.equal(validateTheaterSchedule(maximum).entries.length, 120);
  maximum.cycleSeconds++;
  maximum.entries.push({ ...maximum.entries[0], id: "extra", startSeconds: 120 });
  assert.throws(() => validateTheaterSchedule(maximum), { message: "Invalid theater schedule." });
  const minimum = fixture();
  minimum.cycleSeconds = 1;
  minimum.entries = [{ ...minimum.entries[0], durationSeconds: 1 }];
  assert.equal(at(validateTheaterSchedule(minimum), 1).cycleIndex, 1);
});

test("asset paths reject origins, credentials, aliases, queries, fragments and noncanonical paths", () => {
  for (const source of [
    "https://example.com/movie.mp4", "https://user:secret@example.com/movie.mp4",
    "//example.com/movie.mp4", "movie.mp4", "data:video/mp4;base64,secret", "blob:secret",
    "/", "/assets/", "/assets//movie.mp4", "/./movie.mp4", "/assets/../movie.mp4",
    "/%61ssets/movie.mp4", "/assets/%2e%2e/movie.mp4", "/assets/%252e/movie.mp4",
    "/assets/movie%2fclip.mp4", "/assets/movie.mp4?token=secret", "/assets/movie.mp4#clip",
    "/assets\\movie.mp4", "/assets/movie name.mp4", "/assets/movie\n.mp4", "/assets/é.mp4",
    "/user:secret@host/movie.mp4", "/" + "a".repeat(2048), null,
  ]) invalid((value) => { value.entries[0].source = source; });
  for (const source of ["/movie.mp4", "/assets/movie-v1_2.webm", "/assets/.clip~1.mp4"]) {
    const value = fixture();
    value.entries[0].source = source;
    assert.equal(validateTheaterSchedule(value).entries[0].source, source);
  }
});

test("validation copies and freezes all data and resolver retains the validated entry reference", () => {
  const input = fixture();
  const snapshot = structuredClone(input);
  const schedule = validateTheaterSchedule(input);
  assert.deepEqual(schedule, snapshot);
  assert.notEqual(schedule, input);
  assert.notEqual(schedule.entries, input.entries);
  assert.notEqual(schedule.entries[0], input.entries[0]);
  assert.equal(Object.isFrozen(input), false);
  input.entries[0].title = "Changed";
  input.entries.splice(0);
  input.anchorUnixSeconds = 0;
  assert.deepEqual(schedule, snapshot);
  for (const value of [schedule, schedule.entries, ...schedule.entries]) assert.equal(Object.isFrozen(value), true);
  assert.throws(() => { schedule.entries[0].title = "Changed"; }, TypeError);
  assert.throws(() => schedule.entries.push(snapshot.entries[0]), TypeError);
  assert.equal(at(schedule, 0).entry, schedule.entries[0]);
});

test("untrusted accessors, prototypes and throwing proxies produce only the fixed error", () => {
  let reads = 0;
  invalid((value) => Object.defineProperty(value, "revision", { get() { reads++; throw new Error("secret"); } }));
  invalid((value) => Object.defineProperty(value.entries[0], "source", { get() { reads++; throw new Error("secret"); } }));
  invalid((value) => Object.defineProperty(value.entries, "0", { get() { reads++; throw new Error("secret"); } }));
  invalid((value) => Object.setPrototypeOf(value, { secret: true }));
  assert.equal(reads, 0);
  assert.throws(() => validateTheaterSchedule(new Proxy({}, { ownKeys() { throw new Error("secret"); } })), { message: "Invalid theater schedule." });
});

test("optional captions survive validation and resolution without altering legacy entries", () => {
  const input = fixture();
  input.entries[0].captions = "/weaver/assets/theater/captions/intro.vtt";
  const schedule = validateTheaterSchedule(input);
  assert.deepEqual(schedule, input);
  assert.equal(at(schedule, 0).entry.captions, input.entries[0].captions);
  assert.equal(Object.isFrozen(schedule.entries[0]), true);
  assert.equal(Object.hasOwn(schedule.entries[1], "captions"), false);
  input.entries[0].captions = "/changed.vtt";
  assert.notEqual(schedule.entries[0].captions, input.entries[0].captions);
});

test("captions require canonical root-relative VTT paths and reject accessors", () => {
  for (const captions of [undefined, null, 1, "", "intro.vtt", "//host/intro.vtt",
    "https://host/intro.vtt", "/intro.mp4", "/intro.VTT", "/intro.vtt?x=1",
    "/intro.vtt#cue", "/../intro.vtt", "/a/./intro.vtt", "/%61.vtt",
    "/a//intro.vtt", "/a\\intro.vtt", "/a b.vtt", "/" + "a".repeat(2048) + ".vtt"]) {
    invalid((value) => { value.entries[0].captions = captions; });
  }
  let reads = 0;
  invalid((value) => Object.defineProperty(value.entries[0], "captions", {
    get() { reads++; return "/intro.vtt"; },
  }));
  assert.equal(reads, 0);
});

test("short looping media returns continuous raw media time; metadata modulo belongs to controller", () => {
  const schedule = validateTheaterSchedule(fixture());
  const times = [20, 23, 28, 59.5].map((delta) => at(schedule, delta).mediaTimeSeconds);
  assert.deepEqual(times, [5, 8, 13, 44.5]);
  const actualDuration = 3;
  assert.deepEqual(times.map((time) => time % actualDuration), [2, 2, 1, 2.5]);
  assert.equal(at(schedule, 19).mediaTimeSeconds, 21.5);
});
