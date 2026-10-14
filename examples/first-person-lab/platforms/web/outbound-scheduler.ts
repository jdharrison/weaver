export const PUBLISH_RATE_CONFIGURATION_ERROR =
  "Invalid publish-rate configuration: expected an integer from 1 to 60.";
export const PUBLISH_SPACING_MARGIN_MS = 10;

/** An absent marker preserves unpaced demos; supplied values must be canonical integers. */
export function parsePublishRateHz(value: string | undefined): number | null {
  if (value === undefined) return null;
  const rate = Number(value);
  if (!Number.isInteger(rate) || rate < 1 || rate > 60 || String(rate) !== value) {
    throw new Error(PUBLISH_RATE_CONFIGURATION_ERROR);
  }
  return rate;
}

type SchedulerOptions = {
  rateHz: number | null;
  isCurrent: () => boolean;
  hasPending: () => boolean;
  sendNext: () => Promise<void>;
  onError: (error: unknown) => void;
  now?: () => number;
};

/** One connection-owned send slot shared by every lane, without catch-up bursts. */
export class OutboundScheduler {
  private readonly options: SchedulerOptions;
  private readonly spacingMs: number;
  private readonly now: () => number;
  private nextSendAt = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private inFlight = false;
  private stopped = false;

  constructor(options: SchedulerOptions) {
    this.options = options;
    this.spacingMs = options.rateHz === null ? 0 : 1_000 / options.rateHz + PUBLISH_SPACING_MARGIN_MS;
    this.now = options.now ?? (() => performance.now());
  }

  kick(): void {
    if (this.stopped) return;
    if (!this.options.isCurrent()) {
      this.stop();
      return;
    }
    if (this.inFlight || this.timer !== undefined || !this.options.hasPending()) return;
    const remaining = this.nextSendAt - this.now();
    if (remaining > 0) {
      this.timer = setTimeout(() => {
        this.timer = undefined;
        // Recheck identity and the monotonic deadline, including early timer callbacks.
        this.kick();
      }, Math.ceil(remaining));
      return;
    }
    void this.send();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }

  private async send(): Promise<void> {
    this.inFlight = true;
    this.nextSendAt = this.now() + this.spacingMs;
    try {
      await this.options.sendNext();
    } catch (error) {
      const current = !this.stopped && this.options.isCurrent();
      this.stop();
      if (current) this.options.onError(error);
    } finally {
      this.inFlight = false;
      this.kick();
    }
  }
}
