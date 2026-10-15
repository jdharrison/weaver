export const MAX_WASM_BYTES = 512 * 1024 * 1024;

export interface WasmLoadingOptions {
  signal: AbortSignal;
  onDownload: (loadedBytes: number, totalBytes: number | null) => void;
  onActivity: () => void;
  onComplete: () => void;
  fetcher?: typeof fetch;
  now?: () => number;
}

/** Observe the one response consumed by WASM, keeping streaming compilation/backpressure. */
export async function fetchWasmWithProgress(url: URL, options: WasmLoadingOptions): Promise<Response> {
  const response = await (options.fetcher ?? fetch)(url, { signal: options.signal });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`WASM request failed (${response.status}).`);
  }
  options.signal.throwIfAborted();
  // Fetch exposes decoded chunks, but a CDN Content-Length may describe gzip/br bytes.
  const encoding = response.headers.get("content-encoding")?.trim().toLowerCase();
  const length = response.headers.get("content-length");
  const candidate = !encoding || encoding === "identity" ? Number(length) : 0;
  let totalBytes = length !== null && /^\d+$/.test(length) && Number.isSafeInteger(candidate) &&
    candidate > 0 && candidate <= MAX_WASM_BYTES ? candidate : null;
  let loadedBytes = 0;
  const now = options.now ?? (() => performance.now());
  let lastReport = now();
  options.onDownload(0, totalBytes);
  // An implementation without a readable body still gets native initialization;
  // it cannot supply byte progress. The caller reports initialization on completion.
  if (!response.body) return response;

  const body = response.body.pipeThrough(new TransformStream<Uint8Array, Uint8Array>({
    transform(chunk, controller) {
      options.signal.throwIfAborted();
      loadedBytes += chunk.byteLength;
      if (loadedBytes > MAX_WASM_BYTES) throw new Error("WASM response exceeds the size limit.");
      if (totalBytes !== null && loadedBytes > totalBytes) totalBytes = null;
      if (chunk.byteLength > 0) options.onActivity();
      const timestamp = now();
      if (timestamp - lastReport >= 100) {
        options.onDownload(loadedBytes, totalBytes);
        lastReport = timestamp;
      }
      controller.enqueue(chunk);
    },
    flush() {
      options.signal.throwIfAborted();
      if (totalBytes !== null && loadedBytes !== totalBytes) totalBytes = null;
      options.onDownload(loadedBytes, totalBytes);
      options.onComplete();
    },
  }), { signal: options.signal });
  const headers = new Headers(response.headers);
  // The new stream is already decoded, while Content-Type must survive for compileStreaming.
  headers.delete("content-encoding");
  headers.delete("content-length");
  return new Response(body, { status: response.status, statusText: response.statusText, headers });
}
