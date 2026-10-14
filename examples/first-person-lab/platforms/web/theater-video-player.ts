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
  playing: boolean;
  /** Scheduled denial persists through source changes until explicit play(). */
  autoplayBlocked: boolean;
  muted: boolean;
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
  /** External scheduling owns alignment and autoplay; constructor must not race it. */
  scheduled?: boolean;
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
  private observedPlaying = false;

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
    this.listen("loadedmetadata", () => this.tryAttach());
    for (const event of ["loadeddata", "canplay", "seeked", "timeupdate"]) {
      this.listen(event, () => {
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
      this.status = this.playingStatus();
      this.scheduleFrame();
      this.emit();
    });
    this.listen("pause", () => {
      // Media events are queued; an earlier pause may arrive after replay starts.
      if (!media.paused) return;
      this.observedPlaying = false;
      if (this.wantsPlay) {
        this.wantsPlay = false;
        this.generation += 1;
        this.status = "Paused · press Play";
      }
      this.cancelFrame();
      this.dirty(media.currentTime);
      this.emit();
    });
    this.listen("ended", () => {
      this.observedPlaying = false;
      this.wantsPlay = false;
      this.generation += 1;
      this.cancelFrame();
      this.status = "Source ended · press Play";
      this.dirty(media.currentTime);
      this.emit();
    });
    this.listen("volumechange", () => this.audioChanged());
    this.listen("seeking", () => this.emit());
    this.listen("waiting", () => { this.observedPlaying = false; this.status = "Buffering…"; this.emit(); });
    this.listen("error", () => this.fail(`Movie unavailable (media error ${media.error?.code ?? "unknown"}). Check the same-origin asset and H.264 support.`));
    this.setSource(source);
  }

  get state(): TheaterVideoState {
    const media = this.options.media;
    return {
      status: this.status,
      ready: this.attached && this.sceneReady && this.visible && !this.failed && !this.disposed,
      playing: this.wantsPlay && (this.pendingPlay || !media.paused),
      autoplayBlocked: this.autoplayBlocked,
      muted: media.muted,
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
    this.release();
    this.failed = false;
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
    this.startPlay(false);
  }

  /** Call in the gesture stack, including when muted playback is already active. */
  enableSound(): void {
    if (!this.state.ready || this.pendingPlay) return;
    this.soundError = false;
    this.setMuted(false);
    if (this.options.media.volume === 0) this.setVolume(0.4);
    if (this.options.media.muted || this.options.media.volume === 0) { this.soundError = true; this.emit(); return; }
    this.autoplayBlocked = false;
    this.startPlay(false, true);
  }

  private startPlay(autoplay: boolean, explicitSound = false): void {
    if (!this.state.ready || this.pendingPlay || (this.wantsPlay && !explicitSound)) return;
    this.autoplayAttempted = true;
    this.soundError = false;
    this.wantsPlay = true;
    this.pendingPlay = true;
    const generation = this.generation;
    this.status = "Starting movie…";
    this.emit();
    let result: Promise<void>;
    try { result = this.options.media.play(); }
    catch (reason) { this.pendingPlay = false; this.playRejected(generation, autoplay, reason); return; }
    void Promise.resolve(result).then(() => {
      if (generation !== this.generation || !this.wantsPlay || !this.visible || this.disposed) {
        this.options.media.pause();
        return;
      }
      this.status = this.playingStatus();
      this.scheduleFrame();
      this.emit();
    }, (reason: unknown) => this.playRejected(generation, autoplay, reason)).finally(() => {
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
    this.cancelFrame();
    this.options.media.pause();
    if (!this.failed && !this.disposed) this.status = this.visible ? "Paused · press Play" : "Hidden · press Play when back";
    this.emit();
  }

  /** Explicit user preference; changing audio never starts or resumes media. */
  setMuted(muted: boolean): void {
    if (this.disposed || typeof muted !== "boolean") return;
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
    if (this.disposed || this.failed || !this.sceneReady || !this.visible || this.options.media.readyState < 1) return;
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
      if (!Object.is(previousTime, this.lastFrameTime)) this.emit();
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

  private playRejected(generation: number, autoplay: boolean, reason: unknown): void {
    if (this.disposed) return;
    // Source reset normally aborts a pending play. Only a genuine policy denial
    // from an obsolete attempt may latch permission state for the replacement.
    if (generation !== this.generation || this.failed) {
      const policyDenied = typeof reason === "object" && reason !== null &&
        "name" in reason && reason.name === "NotAllowedError";
      if (autoplay && this.options.scheduled && policyDenied) {
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
    this.options.media.removeAttribute("src");
    this.options.media.load();
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
