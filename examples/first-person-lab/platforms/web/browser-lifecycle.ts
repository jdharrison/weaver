/** WASM startup and renderer readiness may arrive in either order. */
export class BrowserLifecycle {
  initialized = false;
  rendererReady = false;
  failed = false;
  suspended: boolean;
  private autoJoinAllowed: boolean;

  constructor(visible: boolean) {
    this.suspended = !visible;
    this.autoJoinAllowed = visible;
  }

  get canJoin(): boolean {
    return this.initialized && this.rendererReady && !this.failed && !this.suspended;
  }

  markReady(): void {
    if (!this.failed) this.rendererReady = true;
  }

  fail(): void {
    this.failed = true;
    this.rendererReady = false;
    this.cancelAutoJoin();
  }

  suspend(): void {
    this.suspended = true;
    this.cancelAutoJoin();
  }

  resume(): void {
    this.suspended = false;
  }

  cancelAutoJoin(): void {
    this.autoJoinAllowed = false;
  }

  consumeAutoJoin(): boolean {
    if (!this.canJoin || !this.autoJoinAllowed) return false;
    this.cancelAutoJoin();
    return true;
  }
}

/** Invalidate a pending receive before it can deliver into a replacement scene/session. */
export async function receiveWhileCurrent<T>(
  isCurrent: () => boolean,
  receive: () => Promise<T>,
  deliver: (value: T) => void,
): Promise<void> {
  while (isCurrent()) {
    const value = await receive();
    if (!isCurrent()) return;
    deliver(value);
  }
}
