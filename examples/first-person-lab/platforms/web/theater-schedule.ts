export interface TheaterScheduleEntry {
  id: string;
  title: string;
  startSeconds: number;
  durationSeconds: number;
  source: string;
  sourceStartSeconds: number;
  loop: boolean;
}

export interface TheaterSchedule {
  version: 1;
  revision: string;
  anchorUnixSeconds: number;
  cycleSeconds: number;
  driftToleranceSeconds: number;
  entries: TheaterScheduleEntry[];
}

const SCHEDULE_KEYS = [
  "version", "revision", "anchorUnixSeconds", "cycleSeconds",
  "driftToleranceSeconds", "entries",
];
const ENTRY_KEYS = [
  "id", "title", "startSeconds", "durationSeconds", "source",
  "sourceStartSeconds", "loop",
];
const INVALID_SCHEDULE = "Invalid theater schedule.";
const INVALID_TIME = "Invalid theater schedule time.";

function reject(): never {
  throw new Error(INVALID_SCHEDULE);
}

/** Accept only exact, own data fields; do not invoke untrusted accessors. */
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) reject();
  const prototype: unknown = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) reject();
  const ownKeys = Reflect.ownKeys(value);
  if (ownKeys.length !== keys.length || ownKeys.some((key) => typeof key !== "string" || !keys.includes(key))) reject();
  const result: Record<string, unknown> = {};
  for (const key of keys) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (!descriptor || !("value" in descriptor)) reject();
    result[key] = descriptor.value;
  }
  return result;
}

function identifier(value: unknown): value is string {
  return typeof value === "string" && value.length <= 128 && /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value);
}

function title(value: unknown): value is string {
  return typeof value === "string" && value === value.trim() &&
    [...value].length >= 1 && [...value].length <= 100 &&
    !/[\p{Cc}\p{Cf}\p{Cs}]/u.test(value);
}

function finiteInRange(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= minimum && value <= maximum;
}

function integerInRange(value: unknown, minimum: number, maximum: number): value is number {
  return finiteInRange(value, minimum, maximum) && Number.isInteger(value);
}

function assetPath(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 2048 || !/^\/(?:[A-Za-z0-9._~-]+\/)*[A-Za-z0-9._~-]+$/.test(value)) return false;
  return value.split("/").slice(1).every((segment) => segment !== "." && segment !== "..");
}

/** Validate the version-1 wire contract and return an independent, deeply frozen copy. */
export function validateTheaterSchedule(value: unknown): TheaterSchedule {
  try {
    const input = record(value, SCHEDULE_KEYS);
    if (input.version !== 1 || !identifier(input.revision) ||
      !integerInRange(input.anchorUnixSeconds, 0, Number.MAX_SAFE_INTEGER) ||
      !integerInRange(input.cycleSeconds, 1, 3600) ||
      !finiteInRange(input.driftToleranceSeconds, 0, 30) ||
      !Array.isArray(input.entries)) reject();
    const rawEntries: unknown[] = input.entries;
    if (rawEntries.length < 1 || rawEntries.length > 120 ||
      Reflect.ownKeys(rawEntries).length !== rawEntries.length + 1) reject();
    const entries: TheaterScheduleEntry[] = [];
    const ids = new Set<string>();
    let end = 0;
    for (let index = 0; index < rawEntries.length; index++) {
      const descriptor = Object.getOwnPropertyDescriptor(rawEntries, String(index));
      if (!descriptor || !("value" in descriptor)) reject();
      const entry = record(descriptor.value, ENTRY_KEYS);
      if (!identifier(entry.id) || ids.has(entry.id) || !title(entry.title) ||
        !integerInRange(entry.startSeconds, 0, input.cycleSeconds) || entry.startSeconds !== end ||
        !integerInRange(entry.durationSeconds, 1, input.cycleSeconds) ||
        !assetPath(entry.source) || !finiteInRange(entry.sourceStartSeconds, 0, 86400) ||
        typeof entry.loop !== "boolean") reject();
      end += entry.durationSeconds;
      if (end > input.cycleSeconds) reject();
      ids.add(entry.id);
      entries.push(Object.freeze({
        id: entry.id, title: entry.title, startSeconds: entry.startSeconds,
        durationSeconds: entry.durationSeconds, source: entry.source,
        sourceStartSeconds: entry.sourceStartSeconds, loop: entry.loop,
      }));
    }
    if (end !== input.cycleSeconds) reject();
    Object.freeze(entries);
    return Object.freeze({
      version: 1, revision: input.revision, anchorUnixSeconds: input.anchorUnixSeconds,
      cycleSeconds: input.cycleSeconds, driftToleranceSeconds: input.driftToleranceSeconds,
      entries,
    });
  } catch {
    // Never expose paths, labels, accessor/proxy exceptions, or other input content.
    throw new Error(INVALID_SCHEDULE);
  }
}

/**
 * Resolve a validated schedule from UTC Unix seconds, including before its anchor.
 * Entries are half-open intervals. Media time stays continuous even for loops;
 * only a controller with actual media metadata can apply duration modulo.
 * Drift tolerance is controller policy, not a quantization of the supplied clock.
 */
export function resolveTheaterSchedule(schedule: TheaterSchedule, unixSeconds: number): {
  entry: TheaterScheduleEntry;
  cycleIndex: number;
  elapsedSeconds: number;
  mediaTimeSeconds: number;
  remainingSeconds: number;
} {
  const delta = unixSeconds - schedule.anchorUnixSeconds;
  if (!Number.isFinite(unixSeconds) || Math.abs(unixSeconds) > Number.MAX_SAFE_INTEGER ||
    !Number.isFinite(delta) || Math.abs(delta) > Number.MAX_SAFE_INTEGER) {
    throw new Error(INVALID_TIME);
  }
  const cycleIndex = Math.floor(delta / schedule.cycleSeconds) || 0;
  const remainder = delta % schedule.cycleSeconds;
  // Avoid adding the cycle to positive remainders: that loses small fractions.
  const position = remainder < 0 ? remainder + schedule.cycleSeconds : remainder === 0 ? 0 : remainder;
  const entry = schedule.entries.find((candidate) =>
    position >= candidate.startSeconds && position < candidate.startSeconds + candidate.durationSeconds);
  if (!entry) throw new Error(INVALID_TIME);
  const elapsedSeconds = position - entry.startSeconds;
  return {
    entry, cycleIndex, elapsedSeconds,
    mediaTimeSeconds: entry.sourceStartSeconds + elapsedSeconds,
    remainingSeconds: entry.durationSeconds - elapsedSeconds,
  };
}
