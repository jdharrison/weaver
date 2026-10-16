export const DEFAULT_THEATER_VIDEO = "/weaver/assets/theater/Music_SpaceNoirJazz.mp4";

export interface TheaterVideoMedia extends EventTarget {
  src: string;
  muted: boolean;
  volume: number;
  loop: boolean;
  playsInline: boolean;
  preload: string;
  autoplay: boolean;
  currentTime: number;
  duration: number;
  readyState: number;
  paused: boolean;
  seeking: boolean;
  ended?: boolean;
  error: { code: number } | null;
  play(): Promise<void>;
  pause(): void;
  load(): void;
  removeAttribute(name: string): void;
  requestVideoFrameCallback?: (callback: (now: number, metadata: { mediaTime: number }) => void) => number;
  cancelVideoFrameCallback?:              (id: number) => void;
}

export interface TheaterVideoState {
  status: string;
  ready: boolean;
  failed: boolean;
  /** A decoded current frame has been accepted by the renderer bridge. */
  frameReady: boolean;
  buffering: boolean;
  playing: boolean;
  /** Scheduled denial persists through source changes until explicit play(). */
  autoplayBlocked: boolean;
  muted: boolean;
  /** Desired user preference, independent of browser policy and observed audibility. */
  audioEnabled: boolean;
  volume: number;
  soundPending: boolean;
  soundError: boolean;
  audible: boolean;
  currentTime: number;
  duration: number;
  disposed: boolean;
}

export interface TheaterVideoOptions {
  media: TheaterVideoMedia;
  attach: () => boolean;
  markFrame: () => boolean;
  detach: () => void;
  requestAnimationFrame: (callback: () => void) => number;
  cancelAnimationFrame: (id: number) => void;
  onChange?: (state: TheaterVideoState) => void;
  visible?: boolean;
  /** Standalone autoplay is opt-in and starts muted. */
  autoplay?: boolean;
  /** External scheduling owns source selection and autoplay; construction never loads a movie. */
  scheduled?: boolean;
  /** Realign the current schedule after a stalled decoder has reloaded metadata. */
  onRecover?: () => void;
  now?: () => number;
  setTimeout?: (callback: () => void, milliseconds: number) => ReturnType<typeof setTimeout>;
  clearTimeout?: (timer: ReturnType<typeof setTimeout>) => void;
}

/** Owns one media source, one pending play, and at most one frame callback. */
export class TheaterVideoPlayer {
  private readonly options: TheaterVideoOptions;
  private sceneReady = false;
  private visible: boolean;
  private attached = false;
  private disposed = false;
  private failed = false;
  private wantsPlay = false;
  private pendingPlay = false;
  private autoplayAttempted = false;
  private autoplayBlocked = false;
  private soundError = false;
  private audioEnabled = true;
  private observedPlaying = false;
  private buffering = false;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private watchdogGeneration = 0;
  private lastProgressAt = 0;
  private recoveryAttempts = 0;
  private lastRecoveryAt = 0;
  private recoveryTime: number | null = null;
  private playAttempt = 0;

  private generation = 0;
  private frameGeneration = 0;
  private frame: { kind: "video" | "raf"; id: number } | null = null;
  private lastFrameTime = Number.NaN;
  private status = "Loading movie metadata…";
  private readonly listeners: [string, EventListener][] = [];

  constructor(options: TheaterVideoOptions, source = DEFAULT_THEATER_VIDEO) {
    this.options = options;
    this.visible = options.visible ?? true;
    const media = options.media;
    media.autoplay = false;
    media.muted = true;
    media.volume = 0.4;
    media.loop = true;
    media.playsInline = true;
    media.preload = "metadata";
    this.listen("loadedmetadata", () => { this.tryAttach(); this.emit(); });
    for (const event of ["loadeddata", "canplay", "seeked", "timeupdate"]) {
      this.listen(event, () => {
        if (event === "canplay") this.buffering = false;
        this.tryAttach();
        // timeupdate is only a UI clock; decoded callbacks own playback uploads.
        if (event !== "timeupdate") this.dirty(media.currentTime);
        this.emit();
      });
    }
    this.listen("playing", () => {
      if (media.paused) return;
      if (!this.wantsPlay || !this.visible || this.disposed || this.failed) {
        media.pause();
        return;
      }
      this.observedPlaying = true;
      this.buffering = false;
      this.status = this.playingStatus();
      this.scheduleFrame();
      this.emit();
    });
    this.listen("pause", () => {
      // Media events are queued; an earlier pause may arrive after replay starts.
      if (!media.paused) return;
      this.observedPlaying = false;
      // Only pause() clears user playback intent. Browsers may pause the media
      // independently when a decoder or background-tab pipeline is interrupted.
      if (this.wantsPlay) this.status = "Playback interrupted · recovering…";
      this.cancelFrame();
      this.dirty(media.currentTime);
      this.emit();
    });
    this.listen("ended", () => {
      this.observedPlaying = false;
      this.wantsPlay = false;
      this.generation += 1;
      this.cancelWatchdog();
      this.cancelFrame();
      this.status = "Source ended · press Play";
      this.dirty(media.currentTime);
      this.emit();
    });
    this.listen("volumechange", () => this.audioChanged());
    this.listen("seeking", () => this.emit());
    for (const event of ["waiting", "stalled"]) {
      this.listen(event, () => { this.observedPlaying = false; this.buffering = true; this.status = "Buffering…"; this.emit(); });
    }
    this.listen("error", () => {
      // A source reset can leave a queued error event from the retired resource.
      if (media.error && media.src) this.fail(`Movie unavailable (media error ${media.error.code}). Retry the current clip; check the connection and H.264/AAC support if it persists.`);
    });
    if (options.scheduled) {
      this.status = "Waiting for scheduled movie…";
      this.emit();
    } else this.setSource(source);
  }

  get state(): TheaterVideoState {
    const media = this.options.media;
    const ready = this.attached && media.readyState >= 1 && this.recoveryTime === null && this.sceneReady && this.visible && !this.failed && !this.disposed;
    return {
      status: this.status,
      ready,
      failed: this.failed,
      frameReady: ready && media.readyState >= 2 && !media.seeking && Number.isFinite(this.lastFrameTime),
      buffering: this.wantsPlay && (this.buffering || media.seeking || media.readyState < 3),
      playing: this.wantsPlay && (this.pendingPlay || !media.paused),
      autoplayBlocked: this.autoplayBlocked,
      muted: media.muted,
      audioEnabled: this.audioEnabled,
      volume: media.volume,
      soundPending: this.pendingPlay,
      soundError: this.soundError || this.autoplayBlocked,
      audible: this.observedPlaying && !media.paused && !media.ended && !media.muted && media.volume > 0 && !this.failed && !this.disposed,
      currentTime: Number.isFinite(media.currentTime) ? media.currentTime : 0,
      duration: Number.isFinite(media.duration) && media.duration > 0 ? media.duration : 0,
      disposed: this.disposed,
    };
  }

  setSource(source: string): void {
    if (this.disposed) return;
    this.stopPlayback();
    // An unsettled sound attempt grants no permission to replay it on a new source.
    if (this.pendingPlay && !this.options.media.muted) this.applyMuted(true);
    this.release();
    this.failed = false;
    this.soundError = false;
    this.recoveryAttempts = 0;
    this.autoplayAttempted = false;
    if (!this.options.scheduled) this.autoplayBlocked = false;
    if (source.length > 2_048 || !/^\/(?!\/)/.test(source) || /[\s\\]/.test(source)) {
      this.fail("Movie source must be a same-origin root-absolute path.");
      return;
    }
    this.lastFrameTime = Number.NaN;
    this.status = "Loading movie metadata…";
    this.options.media.src = source;
    this.options.media.load();
    this.emit();
  }

  setSceneReady(ready: boolean): void {
    if (this.disposed) return;
    this.sceneReady = ready;
    if (ready) this.tryAttach();
    else if (this.attached || this.wantsPlay) this.pause();
    else this.stopPlayback();
  }

  setVisible(visible: boolean): void {
    if (this.disposed) return;
    this.visible = visible;
    if (!visible) this.pause();
    else this.tryAttach();
    this.emit();
  }

  /** One controller-authorized attempt after alignment; false while busy or blocked. */
  scheduledAutoplay(): boolean {
    if (!this.state.ready || this.pendingPlay || this.autoplayBlocked) return false;
    this.startPlay(true);
    return true;
  }

  /** Call directly from a user gesture to play or retry a blocked autoplay. */
  play(): void {
    if (this.disposed) return;
    this.autoplayBlocked = false;
    this.recoveryAttempts = 0;
    this.startPlay(false);
  }

  /** Call in the gesture stack, including when muted playback is already active. */
  enableSound(): void {
    if (this.disposed) return;
    this.audioEnabled = true;
    if (!this.state.ready || this.pendingPlay) { this.emit(); return; }
    this.soundError = false;
    this.setMuted(false);
    if (this.options.media.volume === 0) this.setVolume(0.4);
    this.autoplayBlocked = false;
    if (this.soundError || this.options.media.muted || this.options.media.volume === 0) {
      this.applyMuted(true);
      this.soundError = true;
      this.startPlay(false);
      this.emit();
      return;
    }
    this.startPlay(false, true);
  }

  private startPlay(autoplay: boolean, explicitSound = false): void {
    if (!this.state.ready || this.pendingPlay || (this.wantsPlay && !this.options.media.paused && !explicitSound)) return;
    this.autoplayAttempted = true;
    this.wantsPlay = true;
    this.pendingPlay = true;
    this.lastProgressAt = this.now();
    this.armWatchdog();
    const generation = this.generation;
    const attempt = ++this.playAttempt;
    this.status = "Starting movie…";
    this.emit();
    const media = this.options.media;
    const audibleAttempt = !media.muted && media.volume > 0;
    const fallback = (reason: unknown): Promise<void> => {
      if (!audibleAttempt || generation !== this.generation || !this.wantsPlay || !this.visible || this.disposed || this.failed) throw reason;
      // Only muted playback may be retried after leaving the gesture stack.
      this.applyMuted(true);
      this.soundError = true;
      if (!media.muted) throw reason;
      return media.play();
    };
    let result: Promise<void>;
    try { result = media.play(); }
    catch (reason) {
      try { result = fallback(reason); }
      catch (failure) { this.pendingPlay = false; this.playRejected(generation, autoplay, failure, audibleAttempt); return; }
      // The synchronous failure already consumed the single muted fallback.
      this.finishPlay(result, generation, autoplay, audibleAttempt, attempt);
      return;
    }
    this.finishPlay(Promise.resolve(result).catch(fallback), generation, autoplay, audibleAttempt, attempt);
  }

  private finishPlay(result: Promise<void>, generation: number, autoplay: boolean, wasAudibleAttempt: boolean, attempt: number): void {
    void Promise.resolve(result).then(() => {
      if (attempt !== this.playAttempt) {
        if (!this.wantsPlay || !this.visible || this.disposed) this.options.media.pause();
        return;
      }
      if (generation !== this.generation || !this.wantsPlay || !this.visible || this.disposed) {
        this.options.media.pause();
        return;
      }
      this.status = this.playingStatus();
      this.scheduleFrame();
      this.emit();
    }, (reason: unknown) => {
      if (attempt === this.playAttempt) this.playRejected(generation, autoplay, reason, wasAudibleAttempt);
    }).finally(() => {
      if (attempt !== this.playAttempt) return;
      this.pendingPlay = false;
      // A replacement source waits for the old play to settle before starting.
      this.tryAutoplay();
      if (!this.disposed) this.emit();
    });
  }

  pause(): void {
    this.autoplayAttempted = true;
    this.stopPlayback();
  }

  private stopPlayback(): void {
    this.generation += 1;
    this.wantsPlay = false;
    this.observedPlaying = false;
    this.buffering = false;
    this.recoveryTime = null;
    this.cancelWatchdog();
    this.cancelFrame();
    this.options.media.pause();
    if (!this.failed && !this.disposed) this.status = this.visible ? "Paused · press Play" : "Hidden · press Play when back";
    this.emit();
  }

  /** Explicit user preference; changing audio never starts or resumes media. */
  setMuted(muted: boolean): void {
    if (this.disposed || typeof muted !== "boolean") return;
    this.audioEnabled = !muted;
    this.applyMuted(muted);
  }

  private applyMuted(muted: boolean): void {
    try {
      this.options.media.muted = muted;
      this.soundError = this.options.media.muted !== muted;
      this.audioChanged();
    } catch {
      this.soundError = true;
      this.status = "Audio change unavailable · try Mute / Unmute again";
      this.emit();
    }
  }

  setVolume(volume: number): void {
    if (this.disposed || typeof volume !== "number" || !Number.isFinite(volume)) return;
    try {
      this.options.media.volume = Math.max(0, Math.min(1, volume));
      this.audioChanged();
    } catch {
      this.soundError = true;
      this.status = "Volume change unavailable · try again";
      this.emit();
    }
  }

  seek(seconds: number): void {
    if (!this.state.ready || !Number.isFinite(seconds) || this.state.duration === 0) return;
    try { this.options.media.currentTime = Math.max(0, Math.min(seconds, this.state.duration)); }
    catch { this.status = "Seek unavailable"; }
    this.emit();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.pause();
    for (const [event, listener] of this.listeners) this.options.media.removeEventListener(event, listener);
    this.release();
    this.status = "Movie stopped";
    this.emit();
  }

  private listen(event: string, listener: EventListener): void {
    this.listeners.push([event, listener]);
    this.options.media.addEventListener(event, listener);
  }

  private tryAttach(): void {
    if (this.disposed || this.failed || !this.sceneReady || !this.visible || !this.options.media.src || this.options.media.readyState < 1) return;
    if (this.recoveryTime !== null) {
      const time = this.recoveryTime;
      this.recoveryTime = null;
      this.wantsPlay = false;
      this.cancelWatchdog();
      try { this.options.media.currentTime = Math.min(time, Math.max(0, this.options.media.duration - 0.001)); }
      catch { /* The controller will retry alignment when the seekable range is ready. */ }
      this.status = "Decoder recovered · aligning playback…";
      const generation = this.generation;
      if (this.options.scheduled) this.options.onRecover?.();
      else this.startPlay(true);
      this.emit();
      // Schedule alignment may select a different clip while handling metadata.
      // Never attach that replacement using the retired decoder's readiness.
      if (generation !== this.generation || !this.sceneReady || !this.visible || this.disposed ||
          this.failed || !this.options.media.src || this.options.media.readyState < 1) return;
    }
    if (this.attached) { this.tryAutoplay(); return; }
    try {
      if (!this.options.attach()) { this.fail("Movie attachment rejected by Weaver. Reload with matching WASM assets."); return; }
      this.attached = true;
      this.status = "Ready · press Play";
      this.dirty(this.options.media.currentTime);
      this.emit();
      this.tryAutoplay();
    } catch { this.fail("Movie attachment failed. Reload with matching WASM assets."); }
  }

  private tryAutoplay(): void {
    if (!this.options.scheduled && this.options.autoplay && !this.autoplayAttempted && this.state.ready && !this.pendingPlay) {
      this.startPlay(true);
    }
  }

  private dirty(time: number): void {
    const media = this.options.media;
    if (!this.attached || this.disposed || this.failed || !this.visible || media.readyState < 2 || media.seeking || time === this.lastFrameTime) return;
    try {
      if (!this.options.markFrame()) { this.fail("Weaver rejected the movie frame."); return; }
      this.lastFrameTime = time;
    } catch { this.fail("Weaver could not accept the movie frame."); }
  }

  private scheduleFrame(): void {
    const media = this.options.media;
    if (this.frame !== null || !this.wantsPlay || !this.visible || !this.attached || media.paused || this.failed || this.disposed) return;
    const generation = this.frameGeneration;
    const consume = (time: number): void => {
      if (generation !== this.frameGeneration) return;
      this.frame = null;
      const previousTime = this.lastFrameTime;
      this.dirty(time);
      if (!Object.is(previousTime, this.lastFrameTime)) {
        this.lastProgressAt = this.now();
        if (this.lastProgressAt - this.lastRecoveryAt > 30_000) this.recoveryAttempts = 0;
        this.emit();
      }
      this.scheduleFrame();
    };
    if (typeof media.requestVideoFrameCallback === "function" && typeof media.cancelVideoFrameCallback === "function") {
      this.frame = { kind: "video", id: media.requestVideoFrameCallback((_now, metadata) => consume(metadata.mediaTime)) };
    } else {
      // RAF polls only while playing; unchanged currentTime never marks a 60 Hz upload.
      this.frame = { kind: "raf", id: this.options.requestAnimationFrame(() => consume(media.currentTime)) };
    }
  }

  private cancelFrame(): void {
    this.frameGeneration += 1;
    if (this.frame?.kind === "video") this.options.media.cancelVideoFrameCallback?.(this.frame.id);
    else if (this.frame?.kind === "raf") this.options.cancelAnimationFrame(this.frame.id);
    this.frame = null;
  }

  private now(): number { return this.options.now?.() ?? performance.now(); }

  private armWatchdog(): void {
    if (this.watchdog !== null || !this.wantsPlay || !this.visible || !this.sceneReady || this.failed || this.disposed) return;
    const generation = this.watchdogGeneration;
    this.watchdog = (this.options.setTimeout ?? setTimeout)(() => {
      if (generation !== this.watchdogGeneration) return;
      this.watchdog = null;
      this.checkPlayback();
      this.armWatchdog();
    }, 1000);
  }

  private cancelWatchdog(): void {
    this.watchdogGeneration += 1;
    if (this.watchdog !== null) (this.options.clearTimeout ?? clearTimeout)(this.watchdog);
    this.watchdog = null;
  }

  private checkPlayback(): void {
    const media = this.options.media;
    if (!this.wantsPlay || !this.visible || !this.sceneReady || this.failed || this.disposed || this.autoplayBlocked || media.ended) return;
    const idle = this.now() - this.lastProgressAt;
    // No currentTime/timeupdate heuristic: a clock can advance while decoded
    // video callbacks have stopped. Schedule seeks also must not hide a freeze.
    if (this.recoveryTime !== null) {
      if (idle >= 15_000) this.fail("Movie recovery timed out · retry the current clip.");
      return;
    }
    if (this.pendingPlay && idle < 15_000) return;
    if (!this.pendingPlay && !media.paused && idle < 8_000) return;
    if (this.recoveryAttempts >= 2) {
      this.fail("Movie playback remains stalled · retry the current clip; check the connection if it persists.");
      return;
    }
    this.recoveryAttempts += 1;
    this.lastRecoveryAt = this.now();
    if (this.recoveryAttempts === 1 && !this.pendingPlay && !media.seeking) {
      // A pause/play resets a suspended playback pipeline without redownloading.
      this.stopPlayback();
      this.startPlay(true);
      return;
    }
    // One bounded decoder reload for a frozen seek/frame or unsettled play().
    // Retire the old promise so late completion cannot pause a replacement play.
    this.recoveryAttempts = 2;
    this.generation += 1;
    this.playAttempt += 1;
    this.pendingPlay = false;
    this.recoveryTime = Number.isFinite(media.currentTime) ? media.currentTime : 0;
    this.lastProgressAt = this.now();
    this.lastFrameTime = Number.NaN;
    this.observedPlaying = false;
    this.buffering = true;
    this.cancelFrame();
    media.pause();
    this.status = "Playback stalled · reloading decoder…";
    try { media.load(); }
    catch { this.fail("Movie recovery failed · retry the current clip."); }
    this.emit();
  }

  private playRejected(generation: number, autoplay: boolean, reason: unknown, wasAudibleAttempt: boolean): void {
    if (this.disposed) return;
    // A stale muted denial still means autoplay is blocked. A stale audible
    // denial says nothing about whether the replacement can play silently.
    if (generation !== this.generation || this.failed) {
      const policyDenied = typeof reason === "object" && reason !== null &&
        "name" in reason && reason.name === "NotAllowedError";
      if (autoplay && this.options.scheduled && policyDenied && !wasAudibleAttempt) {
        this.autoplayBlocked = true;
        this.emit();
      }
      return;
    }
    if (autoplay) this.autoplayBlocked = true;
    this.soundError = true;
    this.pause();
    this.status = autoplay ? "Autoplay blocked/unavailable · press Play" : "Play failed · press Play to retry (browser policy or media unavailable)";
    this.emit();
  }

  private fail(message: string): void {
    if (this.disposed || this.failed) return;
    this.failed = true;
    this.pause();
    this.release();
    this.status = message;
    this.emit();
  }

  private release(): void {
    if (this.attached) {
      this.attached = false;
      try { this.options.detach(); }
      catch {
        // Release the decoder even when the host has already torn down WASM.
      }
    }
    if (this.options.media.src) {
      this.options.media.removeAttribute("src");
      this.options.media.load();
    }
  }

  private playingStatus(): string {
    const media = this.options.media;
    const mode = media.loop ? " loop" : "";
    return media.muted ? `Playing · muted${mode}` : media.volume === 0 ? `Playing · volume 0${mode}` : `Playing · audio enabled${mode}`;
  }

  private audioChanged(): void {
    if (this.wantsPlay && !this.options.media.paused) this.status = this.playingStatus();
    this.emit();
  }

  private emit(): void { this.options.onChange?.(this.state); }
}
