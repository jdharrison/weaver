export const CONNECTED_LOG_DELAY_MS = 3_000;
export const CONNECTED_LOG_MESSAGE =
  "First-Person Lab: connected successfully; delayed client logging test (3 seconds after connection).";

export function scheduleConnectionLog(
  isCurrentConnection: () => boolean,
  enqueue: (message: string) => void,
): () => void {
  let cancelled = false;
  const timer = setTimeout(() => {
    if (!cancelled && isCurrentConnection()) enqueue(CONNECTED_LOG_MESSAGE);
  }, CONNECTED_LOG_DELAY_MS);
  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}
