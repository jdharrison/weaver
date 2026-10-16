import { fetchBoundedText } from "./lobby-bootstrap.js";
import { resolveTheaterSchedule, validateTheaterSchedule, type TheaterSchedule } from "./theater-schedule.js";
import { DEFAULT_THEATER_VIDEO, type TheaterVideoMedia, type TheaterVideoPlayer } from "./theater-video-player.js";

export const DEFAULT_THEATER_SCHEDULE = "/weaver/theater-schedule.json";

export async function loadTheaterSchedule(path: string, origin: string, signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<TheaterSchedule> {
  if (path.length > 2048 || !/^\/(?:[A-Za-z0-9._~-]+\/)*[A-Za-z0-9._~-]+$/.test(path) ||
    path.split("/").some((part) => part === "." || part === "..")) throw new Error("Invalid schedule path.");
  const url = new URL(path, origin);
  if (url.origin !== origin || url.pathname !== path) throw new Error("Invalid schedule path.");
  const text = await fetchBoundedText(url.href, {
    signal, maxBytes: 32 * 1024, timeoutMs: 10_000, label: "Theater schedule", expectJson: true,
  }, fetcher);
  return validateTheaterSchedule(JSON.parse(text!));
}

export interface TheaterScheduleControllerOptions {
  player: Pick<TheaterVideoPlayer, "state" | "setSource" | "seek" | "pause" | "play" | "enableSound" | "scheduledAutoplay"> & Partial<Pick<TheaterVideoPlayer, "setMuted">>;
  media: Pick<TheaterVideoMedia, "src" | "loop" | "seeking" | "ended">;
  path: string;
  origin: string;
  /** Used only by explicit local diagnostics when no scheduled source is loaded. */
  localSource?: string;
  fetcher?: typeof fetch;
  visible?: boolean;
  /** Prepare the current clip without playback until the startup choice. */
  waitForGesture?: boolean;
  now?: () => number;
  setTimeout?: (callback: () => void, milliseconds: number) => ReturnType<typeof setTimeout>;
  clearTimeout?: (timer: ReturnType<typeof setTimeout>) => void;
  onChange: (status: string, following: boolean) => void;
  /** A validated, frozen manifest; local world timing does not follow decoder controls. */
  onScheduleLoaded?: (schedule: TheaterSchedule) => void;
}

/** Local UTC presentation only: no room, hosted clock, or decoder ownership. */
export class TheaterScheduleController {
  private schedule: TheaterSchedule | null = null;
  private readonly abort = new AbortController();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private timerGeneration = 0;
  private following = true;
  private visible: boolean;
  private disposed = false;
  private updating = false;
  private force = true;
  private entryKey = "";
  private attempted = false;
  private autoplayBlocked = false;
  private userPaused = false;
  private resumeBlocked = false;
  private playbackEstablished = false;
  private resumeOnVisible = false;
  private resumeLocal = false;
  private lastTick: number | null = null;
  private loopPlaying = false;
  private resumeLoop = false;
  private mediaReady = false;
  private waitingForSeek = false;
  private status = "Waiting for scene…";
  private loading: Promise<void> | null = null;
  private loadPending = false;
  private loadFailed = false;
  private awaitingStart: boolean;
  private reloadSource = false;
  private continueWithoutSound = false;
  private fallbackSelected = false;

  constructor(private readonly options: TheaterScheduleControllerOptions) {
    this.visible = options.visible ?? true;
    this.awaitingStart = options.waitForGesture ?? false;
    this.resumeBlocked = !this.visible;
    this.publish();
  }

  /** The shell calls this once, outside its ready callback, after combined scene readiness. */
  start(): Promise<void> {
    if (this.disposed) return Promise.resolve();
    if (this.loading) return this.loading;
    this.loadPending = true;
    this.loadFailed = false;
    if (this.following) this.status = "Loading UTC schedule…";
    const options = this.options;
    this.loading = loadTheaterSchedule(options.path, options.origin, this.abort.signal, options.fetcher).then((schedule) => {
      if (this.disposed) return;
      this.schedule = schedule;
      this.fallbackSelected = false;
      // An optional world observer must never break playback or its polling loop.
      try { this.options.onScheduleLoaded?.(schedule); } catch { /* Observer is isolated. */ }
      this.update();
      this.arm();
    }).catch(() => {
      if (this.disposed) return;
      this.loadFailed = true;
      if (this.following) this.status = "Schedule unavailable · retry or continue without sound";
      if (this.continueWithoutSound) this.selectSilentFallback();
    }).finally(() => {
      this.loadPending = false;
      if (!this.disposed) this.publish();
    });
    this.loadLocalSource();
    this.publish();
    return this.loading;
  }

  get followsSchedule(): boolean { return this.following; }

  get state(): { loading: boolean; failed: boolean } {
    return { loading: this.loadPending, failed: this.loadFailed };
  }

  /** Explicit retry only; polling never repeatedly downloads a failed source. */
  retry(reloadCurrent = false): void {
    if (this.disposed || !this.visible || this.loadPending) return;
    if (!this.schedule) {
      this.loading = null;
      void this.start();
      return;
    }
    if (!this.options.player.state.failed && !reloadCurrent) return;
    this.awaitingStart = this.options.waitForGesture ?? false;
    this.playbackEstablished = false;
    this.resumeLocal = false;
    this.entryKey = "";
    this.reloadSource = true;
    this.attempted = false;
    this.force = true;
    this.mediaReady = false;
    this.waitingForSeek = false;
    if (this.following) this.update();
    else this.options.player.setSource(this.options.localSource ?? DEFAULT_THEATER_VIDEO);
  }

  /** The shell mutes first. A slow initial download need not block entry. */
  continueMuted(): void {
    if (this.disposed || !this.visible) return;
    this.continueWithoutSound = true;
    this.options.player.setMuted?.(true);
    this.awaitingStart = false;
    this.userPaused = false;
    this.resumeBlocked = false;
    this.attempted = false;
    this.force = true;
    if (this.loadFailed && !this.schedule) { this.selectSilentFallback(); return; }
    if (this.options.player.state.ready) this.play();
    else this.update();
  }

  private selectSilentFallback(): void {
    this.status = "Schedule unavailable · local silent fallback · retry schedule";
    if (!this.fallbackSelected) {
      this.fallbackSelected = true;
      this.attempted = false;
      this.options.player.setMuted?.(true);
      this.options.media.loop = true;
      this.options.player.setSource(this.options.localSource ?? DEFAULT_THEATER_VIDEO);
    }
    this.playerChanged();
    this.publish();
  }

  setFollowing(following: boolean): void {
    if (this.disposed) return;
    this.following = following;
    this.resumeOnVisible = false;
    this.resumeLocal = false;
    this.resumeLoop = false;
    this.loopPlaying = false;
    this.force = true;
    this.attempted = false;
    if (!following) {
      this.options.media.loop = true;
      this.status = "Local manual mode · not synchronized";
      this.loadLocalSource();
    } else {
      // Local diagnostics may have loaded a different asset within the same UTC slot.
      this.entryKey = "";
      this.options.player.pause();
      this.status = this.schedule ? "Following UTC schedule" : "Schedule unavailable · local diagnostics available";
    }
    this.update();
    this.publish();
  }

  pause(): void {
    if (this.disposed) return;
    this.userPaused = true;
    this.playbackEstablished = false;
    this.resumeOnVisible = false;
    this.resumeLocal = false;
    this.loopPlaying = false;
    this.resumeLoop = false;
    this.options.player.pause();
  }

  /** No await: seek and existing play remain in the original user gesture. */
  play(enableSound = false): void {
    if (this.disposed || !this.visible || (this.following && !this.schedule && !this.fallbackSelected)) return;
    this.userPaused = false;
    this.resumeBlocked = false;
    this.resumeLocal = false;
    this.attempted = true;
    this.force = true;
    if (!this.following || this.fallbackSelected || this.update(true)) {
      this.awaitingStart = false;
      // Alignment can emit the old blocked state synchronously. Clear our latch
      // only after alignment, immediately before the gesture's existing play().
      this.autoplayBlocked = false;
      this.resumeLoop = false;
      if (enableSound) this.options.player.enableSound();
      else this.options.player.play();
    }
  }

  /** The player's bounded decoder reload restores metadata/time before calling this. */
  recoverPlayback(): void {
    if (this.disposed || !this.visible) return;
    const state = this.options.player.state;
    this.rememberAutoplayBlock(state.autoplayBlocked);
    // A timed-out initial silent play can recover before playback is established;
    // an unsettled audible gesture cannot grant the same delayed permission.
    const silentAttempt = this.attempted && (state.muted || state.volume === 0);
    if ((!this.playbackEstablished && !silentAttempt) || !this.canAutoplay() || state.soundPending || this.options.media.ended) return;
    this.force = true;
    this.resumeLoop = false;
    this.attempted = false;
    this.resumeLocal = !this.following;
    this.playerChanged();
  }

  playerChanged(): void {
    if (this.disposed) return;
    // Capture settled playback even while update is guarded, not an unsettled sound gesture.
    const state = this.options.player.state;
    this.rememberAutoplayBlock(state.autoplayBlocked);
    if (state.failed) {
      this.playbackEstablished = false;
      this.resumeOnVisible = false;
      this.resumeLocal = false;
    }
    if (state.playing && !state.soundPending && this.canAutoplay()) this.playbackEstablished = true;
    if (!this.following || (this.fallbackSelected && !this.schedule)) {
      if ((this.resumeLocal || this.fallbackSelected) && this.canAutoplay() && state.ready &&
          !state.soundPending && !this.options.media.seeking && !this.options.media.ended && !this.attempted) {
        this.attempted = true;
        this.resumeLocal = false;
        if (!this.options.player.scheduledAutoplay()) { this.attempted = false; this.resumeLocal = true; }
      }
      return;
    }
    if (this.following && state.playing && !this.autoplayBlocked) this.loopPlaying = true;
    if (this.updating) return;
    const ready = state.ready && state.duration > 0;
    const changed = ready !== this.mediaReady;
    this.mediaReady = ready;
    // Frame/time UI notifications are not another schedule polling loop.
    if (changed || this.force || this.waitingForSeek || this.options.media.ended) this.update();
  }

  setVisible(visible: boolean): void {
    if (this.disposed || visible === this.visible) return;
    if (!visible) {
      const state = this.options.player.state;
      this.rememberAutoplayBlock(state.autoplayBlocked);
      this.resumeOnVisible = this.playbackEstablished && this.canAutoplay() && !state.soundPending && !this.options.media.ended;
      this.resumeBlocked = true;
      this.loopPlaying = false;
      this.resumeLoop = false;
      this.cancelTimer();
    } else {
      // Only release the visibility pause, never an earlier clock/startup/manual block.
      if (this.resumeOnVisible) {
        this.resumeBlocked = false;
        this.attempted = false;
        this.resumeLocal = !this.following;
      }
      this.resumeOnVisible = false;
    }
    this.visible = visible;
    this.force = true;
    if (visible) { this.loadLocalSource(); this.playerChanged(); this.arm(); }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.abort.abort();
    this.cancelTimer();
  }

  private loadLocalSource(): void {
    if (!this.disposed && this.loading && this.visible && !this.following && !this.options.media.src) {
      this.options.player.setSource(this.options.localSource ?? DEFAULT_THEATER_VIDEO);
    }
  }

  private now(): number { return this.options.now?.() ?? Date.now() / 1000; }

  private canAutoplay(): boolean {
    const state = this.options.player.state;
    return this.visible && !this.awaitingStart && !this.userPaused && !this.resumeBlocked &&
      !this.autoplayBlocked && !state.autoplayBlocked && !state.failed && !state.disposed;
  }

  private rememberAutoplayBlock(blocked: boolean): void {
    if (!blocked) return;
    this.autoplayBlocked = true;
    this.playbackEstablished = false;
    this.resumeOnVisible = false;
    this.resumeLocal = false;
    this.loopPlaying = false;
    this.resumeLoop = false;
  }

  private clockUnavailable(): void {
    this.force = true;
    this.resumeBlocked = true;
    this.attempted = true;
    this.playbackEstablished = false;
    this.resumeOnVisible = false;
    this.resumeLocal = false;
    this.loopPlaying = false;
    this.resumeLoop = false;
    this.waitingForSeek = false;
    this.lastTick = null;
    this.options.player.pause();
    this.status = "Clock unavailable · playback paused · press Play after clock recovery";
    this.publish();
  }

  private update(manual = false, tick = false): boolean {
    if (this.disposed || this.updating || !this.visible || !this.following || !this.schedule) return false;
    this.updating = true;
    try {
      let now: number;
      let slot: ReturnType<typeof resolveTheaterSchedule>;
      let utc: string;
      try {
        now = this.now();
        slot = resolveTheaterSchedule(this.schedule, now);
        // The model's safe Unix range is wider than JavaScript Date's range.
        // Validate both before changing sources or making any playback decision.
        utc = new Date(now * 1000).toISOString().split("T")[1].slice(0, 8);
      } catch {
        this.clockUnavailable();
        return false;
      }
      if (tick && this.lastTick !== null && (now < this.lastTick || now - this.lastTick > 2)) this.force = true;
      if (tick || this.lastTick === null) this.lastTick = now;
      this.rememberAutoplayBlock(this.options.player.state.autoplayBlocked);
      const key = `${slot.cycleIndex}:${slot.entry.id}`;
      if (key !== this.entryKey) {
        this.entryKey = key;
        this.loopPlaying = false;
        this.resumeLoop = false;
        this.force = true;
        this.attempted = manual;
        // Compare the asset, not the slot: offsets and cycle boundaries never reload it.
        const current = this.options.media.src;
        if (this.reloadSource || (current !== slot.entry.source && current !== new URL(slot.entry.source, this.options.origin).href)) {
          this.reloadSource = false;
          this.options.player.setSource(slot.entry.source);
        }
      }
      // Looping must return to sourceStart, not to the beginning of the asset.
      this.options.media.loop = false;
      const state = this.options.player.state;

      this.status = `${slot.entry.title} · ${utc} UTC · ${this.schedule.revision} · waiting for media`;
      this.waitingForSeek = this.options.media.seeking;
      // Automatic startup waits for seek completion; gestures must retain activation.
      if (!state.ready || state.duration <= 0 || (!manual && this.waitingForSeek)) { this.publish(); return false; }
      const start = Math.min(slot.entry.sourceStartSeconds, state.duration);
      const available = state.duration - start;
      const raw = slot.mediaTimeSeconds;
      const hold = available <= 0 || (!slot.entry.loop && raw >= state.duration);
      // Stay inside the decoded range so a short non-looping source retains its final frame.
      const end = Math.max(0, state.duration - 0.001);
      const desired = hold ? end : slot.entry.loop ? start + slot.elapsedSeconds % available : raw;
      const drift = Math.abs(state.currentTime - desired);
      const restartLoop = slot.entry.loop && this.options.media.ended && this.loopPlaying && !this.userPaused && !this.resumeBlocked && !this.autoplayBlocked;
      if (state.playing && !this.autoplayBlocked) this.loopPlaying = true;
      if (restartLoop) this.resumeLoop = true;
      if (this.force || restartLoop || drift > this.schedule.driftToleranceSeconds) {
        this.force = false; // seek emits synchronously; consume force before calling out.
        this.options.player.seek(desired);
        this.waitingForSeek = this.options.media.seeking;
      }
      this.status = `${slot.entry.title} · ${utc} UTC · drift ${drift.toFixed(1)}s · ${this.schedule.revision}${hold ? " · source ended (holding final frame)" : ""}`;
      if (hold) {
        this.playbackEstablished = false;
        this.resumeLoop = false;
        if (state.playing) this.options.player.pause();
      }
      else if (!manual && !this.awaitingStart && this.resumeLoop && !this.options.media.seeking && !this.autoplayBlocked) {
        this.resumeLoop = false;
        this.loopPlaying = false;
        this.options.player.play();
      }
      else if (!manual && !this.awaitingStart && !this.options.media.seeking && !this.attempted && !this.userPaused && !this.resumeBlocked && !this.autoplayBlocked) {
        // Mark before calling out to prevent synchronous play callbacks from reentering.
        this.attempted = true;
        if (!this.options.player.scheduledAutoplay()) this.attempted = false;
      }
      this.publish();
      return !hold;
    } finally { this.updating = false; }
  }

  private publish(): void { this.options.onChange(this.status, this.following); }
  private arm(): void {
    if (this.disposed || !this.visible || !this.schedule || this.timer !== null) return;
    const generation = ++this.timerGeneration;
    this.timer = (this.options.setTimeout ?? setTimeout)(() => {
      if (generation !== this.timerGeneration) return;
      this.timer = null;
      if (this.disposed || !this.visible) return;
      try {
        this.update(false, true);
      } finally {
        // Clock safety pauses must not stop polling for a later valid clock.
        this.arm();
      }
    }, 1000);
  }
  private cancelTimer(): void {
    this.timerGeneration += 1;
    if (this.timer !== null) (this.options.clearTimeout ?? clearTimeout)(this.timer);
    this.timer = null;
    this.lastTick = null;
  }
}
