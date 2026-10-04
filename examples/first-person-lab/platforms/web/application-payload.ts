export const MAX_APPLICATION_MESSAGE_BYTES = 2_048;
const decoder = new TextDecoder("utf-8", { fatal: true });

/** Invalid application bytes are a dropped message, not a transport failure. */
export function decodeApplicationPayload(payload: Uint8Array): string | null {
  if (payload.byteLength > MAX_APPLICATION_MESSAGE_BYTES) return null;
  try {
    return decoder.decode(payload);
  } catch {
    return null;
  }
}
