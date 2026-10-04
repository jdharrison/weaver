export const BOOTSTRAP_TIMEOUT_MS = 10_000;
export const MAX_BOOTSTRAP_BYTES = 8_192;
const MAX_U64 = 18_446_744_073_709_551_615n;
const TARGET_KEYS = ["version", "url", "namespaceId", "sessionId", "spaceId", "spaceEpoch"];

export type LobbyTarget = Readonly<{
  version: 1;
  url: string;
  namespaceId: bigint;
  sessionId: bigint;
  spaceId: bigint;
  spaceEpoch: bigint;
}>;
export type LobbyConfig = Readonly<{ path: string; target: LobbyTarget }>;
export type LobbyBootstrap = LobbyTarget & Readonly<{ token: string }>;

function jsonObject(text: string, label: string): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error(`${label} must be valid JSON`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be a JSON object`);
  }
  return value as Record<string, unknown>;
}

function exactKeys(value: Record<string, unknown>, keys: string[], label: string): void {
  if (Object.keys(value).length !== keys.length || keys.some((key) => !Object.hasOwn(value, key))) {
    throw new Error(`${label} has missing or unsupported fields`);
  }
}

function decimalId(value: unknown, label: string, minimum = 1n): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) {
    throw new Error(`${label} must be a canonical positive decimal u64 string`);
  }
  const id = BigInt(value);
  if (id < minimum || id > MAX_U64) {
    throw new Error(`${label} is outside the allowed u64 range`);
  }
  return id;
}

function httpsEndpoint(value: unknown): string {
  if (typeof value !== "string" || value.length > 512 || /[\s\\]/u.test(value)) {
    throw new Error("Lobby endpoint must be a bounded absolute HTTPS WebTransport URL");
  }
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error("Lobby endpoint must be an absolute HTTPS WebTransport URL");
  }
  if (!value.startsWith("https://") || url.protocol !== "https:" || url.username || url.password ||
      url.search || url.hash || value.includes("?") || value.includes("#") || url.pathname === "/") {
    throw new Error("Lobby endpoint requires HTTPS and a path, without credentials, query, or fragment");
  }
  return url.href;
}

function targetFields(value: Record<string, unknown>): LobbyTarget {
  if (value.version !== 1) throw new Error("Unsupported lobby contract version (expected 1)");
  return {
    version: 1,
    url: httpsEndpoint(value.url),
    namespaceId: decimalId(value.namespaceId, "Lobby namespaceId"),
    sessionId: decimalId(value.sessionId, "Lobby sessionId"),
    spaceId: decimalId(value.spaceId, "Lobby spaceId", 3n),
    spaceEpoch: decimalId(value.spaceEpoch, "Lobby spaceEpoch"),
  };
}

export function readLobbyConfig(path: string, targetText: string, pageOrigin: string): LobbyConfig | null {
  if (path === "" || path === "off") return null;
  if (path.length > 512 || !path.startsWith("/") || path.startsWith("//") || /[\s\\?#]/u.test(path)) {
    throw new Error("Lobby bootstrap must be a same-origin absolute path without query or fragment");
  }
  const url = new URL(path, pageOrigin);
  if (url.origin !== pageOrigin || url.pathname !== path) {
    throw new Error("Lobby bootstrap path must be canonical and same-origin");
  }
  if (new TextEncoder().encode(targetText).byteLength > MAX_BOOTSTRAP_BYTES) {
    throw new Error("Lobby target configuration is too large");
  }
  const target = jsonObject(targetText, "Lobby target configuration");
  exactKeys(target, TARGET_KEYS, "Lobby target configuration");
  return { path: url.href, target: targetFields(target) };
}

export function validateLobbyBootstrap(text: string, target: LobbyTarget): LobbyBootstrap {
  if (new TextEncoder().encode(text).byteLength > MAX_BOOTSTRAP_BYTES) {
    throw new Error("Lobby bootstrap response is too large");
  }
  const value = jsonObject(text, "Lobby bootstrap response");
  exactKeys(value, [...TARGET_KEYS, "token"], "Lobby bootstrap response");
  const resolved = targetFields(value);
  if (TARGET_KEYS.some((key) => resolved[key as keyof LobbyTarget] !== target[key as keyof LobbyTarget])) {
    throw new Error("Lobby bootstrap endpoint or scope does not match the configured allowed target");
  }
  if (typeof value.token !== "string" || !/^[0-9a-f]{64}$/.test(value.token)) {
    throw new Error("Lobby credential must be exactly 64 lowercase hexadecimal characters");
  }
  return { ...resolved, token: value.token };
}

/** Both the request and streaming body share one deadline; neither response bodies nor URLs are logged. */
export async function fetchBoundedText(
  url: string,
  options: {
    signal: AbortSignal;
    maxBytes: number;
    label: string;
    timeoutMs?: number;
    allowNotFound?: boolean;
    expectJson?: boolean;
  },
  fetcher: typeof fetch = fetch,
): Promise<string | null> {
  const controller = new AbortController();
  let timedOut = false;
  let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  const cancel = (): void => controller.abort();
  const timer = setTimeout(() => {
    timedOut = true;
    cancel();
  }, options.timeoutMs ?? BOOTSTRAP_TIMEOUT_MS);
  options.signal.addEventListener("abort", cancel, { once: true });
  if (options.signal.aborted) cancel();
  let rejectCancelled: () => void = () => {};
  const cancelled = new Promise<never>((_resolve, reject) => {
    rejectCancelled = () => reject(new Error(`${options.label} ${timedOut ? "timed out" : "cancelled"}`));
    controller.signal.addEventListener("abort", rejectCancelled, { once: true });
    if (controller.signal.aborted) rejectCancelled();
  });
  const read = async (): Promise<string | null> => {
    controller.signal.throwIfAborted();
    let response: Response;
    try {
      response = await fetcher(url, {
        signal: controller.signal,
        cache: "no-store",
        credentials: "omit",
        redirect: "error",
        referrerPolicy: "no-referrer",
        ...(options.expectJson ? { headers: { Accept: "application/json" } } : {}),
      });
    } catch {
      throw new Error(`${options.label} request failed; check the route and retry`);
    }
    controller.signal.throwIfAborted();
    if (response.redirected) throw new Error(`${options.label} redirects are not allowed`);
    if (options.allowNotFound && response.status === 404) return null;
    if (!response.ok) throw new Error(`${options.label} request failed with HTTP ${response.status}`);
    if (options.expectJson && response.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "application/json") {
      throw new Error(`${options.label} must return application/json`);
    }
    const length = Number(response.headers.get("content-length"));
    if (length > options.maxBytes) throw new Error(`${options.label} response is too large`);
    if (response.body === null) throw new Error(`${options.label} response body is missing`);
    reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let bytes = 0;
    while (true) {
      let chunk: ReadableStreamReadResult<Uint8Array>;
      try {
        chunk = await reader.read();
      } catch {
        throw new Error(`${options.label} response could not be read`);
      }
      controller.signal.throwIfAborted();
      if (chunk.done) break;
      if (chunk.value.byteLength === 0) continue;
      bytes += chunk.value.byteLength;
      if (bytes > options.maxBytes) throw new Error(`${options.label} response is too large`);
      chunks.push(chunk.value.slice());
    }
    reader.releaseLock();
    reader = undefined;
    const body = new Uint8Array(bytes);
    let offset = 0;
    for (const chunk of chunks) {
      body.set(chunk, offset);
      offset += chunk.byteLength;
    }
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(body);
    } catch {
      throw new Error(`${options.label} response is not valid UTF-8`);
    }
  };
  try {
    return await Promise.race([read(), cancelled]);
  } catch (error) {
    controller.abort();
    if (reader !== undefined) void reader.cancel().catch(() => {});
    throw error;
  } finally {
    clearTimeout(timer);
    options.signal.removeEventListener("abort", cancel);
    controller.signal.removeEventListener("abort", rejectCancelled);
  }
}

export async function fetchLobbyBootstrap(
  config: LobbyConfig,
  signal: AbortSignal,
  fetcher: typeof fetch = fetch,
): Promise<LobbyBootstrap> {
  const text = await fetchBoundedText(config.path, {
    signal,
    maxBytes: MAX_BOOTSTRAP_BYTES,
    label: "Lobby bootstrap",
    expectJson: true,
  }, fetcher);
  return validateLobbyBootstrap(text!, config.target);
}
