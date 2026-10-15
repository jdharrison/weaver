export const SUBTITLE_PREFERENCE_KEY = "weaver.theater.subtitles";
export interface SubtitleStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}
export function subtitlePolicy(preferred: boolean, audible: boolean): { enabled: boolean; forced: boolean; panel: number } {
  return { enabled: preferred || !audible, forced: !audible, panel: !audible ? 6 : preferred ? 3 : 1 };
}
export function captionSource(entry: { source: string; captions?: string } | undefined, mediaSource: string, origin: string): string | undefined {
  if (!entry?.captions || !mediaSource) return undefined;
  try {
    const url = new URL(entry.captions, origin);
    if (url.origin !== origin || !/^\/(?!\/)/.test(entry.captions) || /[\s\\]/.test(entry.captions) ||
        !url.pathname.endsWith(".vtt") || entry.captions.length > 2048) return undefined;
    return new URL(entry.source, origin).href === new URL(mediaSource, origin).href ? url.href : undefined;
  } catch { return undefined; }
}

export function captionsForMedia(entries: readonly { source: string; captions?: string }[], scheduled: { source: string; captions?: string } | undefined, mediaSource: string, origin: string): string | undefined {
  if (!mediaSource) return undefined;
  const matches = (entry: { source: string } | undefined): boolean => {
    try { return !!entry && new URL(entry.source, origin).href === new URL(mediaSource, origin).href; }
    catch { return false; }
  };
  // Association follows the loaded movie even after manual controls stop schedule following.
  const entry = matches(scheduled) ? scheduled : entries.find(matches);
  return captionSource(entry, mediaSource, origin);
}

/** Native tracks own VTT parsing and media-time cue selection; the HUD receives text only. */
export class TheaterSubtitles {
  private preferred = true;
  private audible = false;
  private disposed = false;
  private source: string | undefined;
  private mediaSource = "";
  private track: HTMLTrackElement | undefined;
  private cleanupTrack: (() => void) | undefined;
  private readonly media: HTMLVideoElement;
  private readonly hud: HTMLElement;
  private readonly button: HTMLButtonElement;
  private readonly explanation: HTMLElement;
  private readonly storage: SubtitleStorage;
  private readonly panel: (state: number) => void;
  private readonly reset = (): void => {
    // Source changes clear synchronously via update(); queued emptied events must
    // not retire a newly installed track for the replacement resource.
    if ((this.media.getAttribute("src") ?? "") !== this.mediaSource) this.setSource(undefined);
    this.render();
  };
  private readonly toggleClick = (): void => this.toggle();
  constructor(options: { media: HTMLVideoElement; hud: HTMLElement; button: HTMLButtonElement; explanation: HTMLElement; storage: SubtitleStorage; panel: (state: number) => void }) {
    this.media = options.media;
    this.hud = options.hud;
    this.button = options.button;
    this.explanation = options.explanation;
    this.storage = options.storage;
    this.panel = options.panel;
    try { this.preferred = this.storage.getItem(SUBTITLE_PREFERENCE_KEY) !== "off"; } catch { /* Storage may be blocked. */ }
    this.button.addEventListener("click", this.toggleClick);
    for (const event of ["emptied", "seeking", "seeked", "timeupdate"]) this.media.addEventListener(event, this.reset);
    this.publish();
  }
  update(audible: boolean): void {
    if (this.disposed) return;
    this.audible = audible;
    if ((this.media.getAttribute("src") ?? "") !== this.mediaSource) this.setSource(undefined);
    this.publish();
  }
  toggle(): void {
    if (this.disposed || !this.audible) return;
    this.preferred = !this.preferred;
    try { this.storage.setItem(SUBTITLE_PREFERENCE_KEY, this.preferred ? "on" : "off"); } catch { /* Keep the session preference. */ }
    this.publish();
  }
  setSource(source: string | undefined): void {
    if (this.disposed) return;
    const mediaSource = this.media.getAttribute("src") ?? "";
    if (source === this.source && mediaSource === this.mediaSource) return;
    this.cleanupTrack?.();
    this.cleanupTrack = undefined;
    this.track = undefined;
    this.source = source;
    this.mediaSource = mediaSource;
    this.hud.textContent = "";
    this.hud.hidden = true;
    if (!source || !mediaSource) return;
    const track = this.media.ownerDocument.createElement("track");
    track.kind = "captions";
    track.label = "Movie captions";
    track.src = source;
    let loaded = false;
    const current = (): boolean => !this.disposed && this.track === track && this.media.getAttribute("src") === mediaSource;
    const cues = (): void => { if (current() && loaded) this.render(); };
    const load = (): void => { if (current()) { loaded = true; cues(); } };
    const error = (): void => { if (current()) { loaded = false; this.hud.textContent = ""; this.hud.hidden = true; } };
    track.addEventListener("load", load);
    track.addEventListener("error", error);
    track.track.addEventListener("cuechange", cues);
    this.track = track;
    this.cleanupTrack = () => {
      track.removeEventListener("load", load);
      track.removeEventListener("error", error);
      track.track.removeEventListener("cuechange", cues);
      track.track.mode = "disabled";
      track.remove();
    };
    this.media.append(track);
    track.track.mode = "hidden";
  }
  private render(): void {
    let text = "";
    if (!this.disposed && subtitlePolicy(this.preferred, this.audible).enabled && !this.media.seeking &&
        this.track?.readyState === 2 && this.media.getAttribute("src") === this.mediaSource) {
      const cues = this.track.track.activeCues;
      for (let i = 0; cues && i < cues.length; i++) {
        const cue = cues[i] as VTTCue;
        // getCueAsHTML is the browser's VTT parser, not an HTML injection sink.
        // Read its text only: no cue nodes or author markup enter the live DOM.
        if (this.media.currentTime >= cue.startTime && this.media.currentTime < cue.endTime) {
          const line = cue.getCueAsHTML().textContent ?? "";
          text += (text ? "\n" : "") + line;
        }
      }
    }
    if (this.hud.textContent !== text) this.hud.textContent = text;
    this.hud.hidden = !text;
  }
  private publish(): void {
    const state = subtitlePolicy(this.preferred, this.audible);
    this.button.disabled = this.disposed || state.forced;
    this.button.setAttribute("aria-pressed", String(state.enabled));
    this.button.textContent = state.forced ? "Subtitles: on (required)" : state.enabled ? "Subtitles: on" : "Subtitles: off";
    this.explanation.textContent = state.forced ? "Subtitles stay on while movie audio is not audible. Your preference returns when sound resumes. Captions appear when available." : "Show available movie captions.";
    this.panel(this.disposed ? 0 : state.panel);
    this.render();
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.cleanupTrack?.();
    this.track = undefined;
    this.button.removeEventListener("click", this.toggleClick);
    for (const event of ["emptied", "seeking", "seeked", "timeupdate"]) this.media.removeEventListener(event, this.reset);
    this.publish();
  }
}
