export interface ControllerHudState {
  audioEnabled: boolean;
  audioAudible: boolean;
  audioPending: boolean;
  audioFailed: boolean;
  audioReady: boolean;
  subtitles: boolean;
  fullscreen: boolean;
  fullscreenAvailable: boolean;
  fullscreenPending: boolean;
  fullscreenError: boolean;
}

export interface ControllerHudOptions {
  window: Window;
  document: Document;
  launcher: HTMLButtonElement;
  overlay: HTMLElement;
  dialog: HTMLElement;
  audio: HTMLButtonElement;
  subtitles: HTMLButtonElement;
  fullscreen: HTMLButtonElement;
  status: HTMLElement;
  background: HTMLElement[];
  canOpen(): boolean;
  canResume(): boolean;
  clearInputs(): void;
  ownership(open: boolean): void;
  resume(restoreLock: boolean): void;
  audioAction(): void;
  subtitlesAction(): void;
  fullscreenAction(): void;
}

/** Browser-only presentation; media, schedule and network keep their own lifetimes. */
export class ControllerHud {
  private opened = false;
  private disposed = false;
  private qHeld = false;
  private restoreLock = false;
  private previousFocus: HTMLElement | null = null;
  private inert = new Map<HTMLElement, boolean>();
  private outsideDown = false;
  private launcherTouch: { id: number; x: number; y: number; started: number; moved: boolean } | undefined;
  private launcherTouchClickId: number | undefined;
  private suppressLauncherClick = false;
  private suppressTouchCompatibility = false;
  private readonly cleanups: (() => void)[] = [];
  private readonly options: ControllerHudOptions;

  constructor(options: ControllerHudOptions) {
    this.options = options;
    const { window: child, document: doc, launcher, overlay, audio, subtitles, fullscreen } = options;
    // Minimal-mode initialization starts every image request; interaction never awaits loading/decoding.
    for (const file of ["collapsed_state.png", "expanded_base_background_x3.png", "audio_on.png", "audio_off.png",
      "subtitles_on.png", "subtitles_off.png", "fullscreen_on.png", "fullscreen_off.png"]) {
      const link = doc.createElement("link");
      link.rel = "preload";
      link.as = "image";
      link.href = `/weaver/assets/controller/${file}`;
      doc.head.append(link);
    }
    launcher.hidden = false;
    overlay.hidden = true;
    launcher.disabled = false;
    // A second finger can deliver pointer events without any compatibility click.
    for (const type of ["pointerdown", "pointermove", "pointerup", "pointercancel", "lostpointercapture"]) {
      this.listen(child, type, event => this.launcherPointer(event as PointerEvent), true);
    }
    this.listen(child, "click", event => {
      if (this.isLauncherCompatibilityClick(event)) this.consume(event);
    }, true);
    this.listen(launcher, "click", event => {
      if (this.isLauncherCompatibilityClick(event)) this.consume(event);
      else this.open();
    });
    this.listen(audio, "click", event => { if (event.isTrusted && !audio.disabled) options.audioAction(); });
    this.listen(subtitles, "click", () => options.subtitlesAction());
    this.listen(fullscreen, "click", () => options.fullscreenAction());
    this.listen(child, "keydown", (input) => this.keydown(input as KeyboardEvent), true);
    this.listen(child, "keyup", (input) => {
      if ((input as KeyboardEvent).code === "KeyQ") this.qHeld = false;
    }, true);
    this.listen(child, "blur", () => { this.qHeld = false; this.close(false); });
    this.listen(doc, "pointerlockchange", () => {
      if (this.opened && doc.pointerLockElement) doc.exitPointerLock?.();
    });
    this.listen(doc, "focusin", (event) => {
      if (this.opened && !options.dialog.contains(event.target as Node)) this.focusFirst();
    }, true);
    // Outside input never reaches Winit, including gestures begun before opening.
    for (const type of ["pointerdown", "pointerup", "pointermove", "pointercancel", "mousedown", "mouseup", "touchstart", "touchmove", "touchend", "wheel", "click", "dblclick", "contextmenu"]) {
      this.listen(child, type, (event) => {
        if (!this.opened) return;
        const inside = options.dialog.contains(event.target as Node);
        if (type === "pointerdown") this.outsideDown = event.target === overlay;
        if (inside) return;
        const dismiss = type === "click" && event.target === overlay && this.outsideDown;
        // Cancelling legacy touchstart/end suppresses the completed compatibility click.
        // The full-screen overlay and touch-action already prevent scene gestures.
        if (event.target === overlay && type.startsWith("touch")) event.stopImmediatePropagation();
        else this.consume(event);
        if (type === "pointercancel") this.outsideDown = false;
        if (dismiss) this.close(true);
      }, true);
      this.listen(overlay, type, (event) => event.stopPropagation());
    }
    for (const type of ["keydown", "keyup", "beforeinput", "input", "compositionstart", "compositionupdate", "compositionend"]) {
      this.listen(overlay, type, (event) => event.stopPropagation());
    }
  }

  get openState(): boolean { return this.opened; }
  contains(target: EventTarget | null): boolean { return this.opened && this.options.dialog.contains(target as Node); }

  private listen(target: EventTarget, type: string, callback: EventListener, capture = false): void {
    target.addEventListener(type, callback, { capture, passive: false });
    this.cleanups.push(() => target.removeEventListener(type, callback, capture));
  }
  private launcherPointer(event: PointerEvent): void {
    const { launcher } = this.options;
    if (event.type === "pointerdown") {
      // A fresh gesture may produce its own legacy click (e.g. backdrop dismissal).
      this.suppressTouchCompatibility = false;
      if (event.pointerType !== "touch") {
        if (launcher.contains(event.target as Node)) {
          this.suppressLauncherClick = false;
          this.launcherTouchClickId = undefined;
        }
        return;
      }
      if (!launcher.contains(event.target as Node) || this.opened || launcher.disabled || launcher.inert || !this.options.canOpen()) return;
      this.suppressLauncherClick = this.suppressTouchCompatibility = true;
      this.launcherTouchClickId = event.pointerId;
      this.launcherTouch = { id: event.pointerId, x: event.clientX, y: event.clientY, started: event.timeStamp, moved: false };
      this.consume(event);
      return;
    }
    const touch = this.launcherTouch;
    if (!touch || event.pointerId !== touch.id) return;
    this.consume(event);
    if (event.type === "pointercancel" || event.type === "lostpointercapture") {
      this.launcherTouch = undefined;
      return;
    }
    if (Math.hypot(event.clientX - touch.x, event.clientY - touch.y) > 8) touch.moved = true;
    if (event.type !== "pointerup") return;
    this.launcherTouch = undefined;
    const elapsed = event.timeStamp - touch.started;
    if (event.pointerType === "touch" && !touch.moved && elapsed >= 0 && elapsed <= 500 &&
        launcher.contains(event.target as Node)) this.open();
  }

  private isLauncherCompatibilityClick(event: Event): boolean {
    if (!this.suppressLauncherClick) return false;
    const click = event as PointerEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } };
    // Native keyboard/assistive activation has zero detail and no touch source.
    if (click.detail === 0 && click.pointerType !== "touch" && !click.sourceCapabilities?.firesTouchEvents) return false;
    return this.options.launcher.contains(event.target as Node) ||
      (click.pointerType === "touch" && click.pointerId === this.launcherTouchClickId) || this.suppressTouchCompatibility;
  }

  private focusFirst(): void {
    [this.options.audio, this.options.subtitles, this.options.fullscreen].find(button => !button.disabled)?.focus({ preventScroll: true });
  }
  private consume(event: Event): void { event.preventDefault(); event.stopImmediatePropagation(); }
  private keydown(event: KeyboardEvent): void {
    const { document: doc } = this.options;
    const editing = (target: EventTarget | null): boolean => target instanceof HTMLElement &&
      (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
    if (event.code === "KeyQ" && !event.isComposing && !event.ctrlKey && !event.altKey && !event.metaKey &&
        !event.composedPath().some(editing) && !editing(doc.activeElement)) {
      if (!this.opened && !this.options.canOpen()) return;
      this.consume(event);
      if (event.repeat || this.qHeld) return;
      this.qHeld = true;
      if (this.opened) this.close(true); else this.open();
      return;
    }
    if (!this.opened) return;
    if (event.key === "Escape") { this.consume(event); this.close(true); return; }
    if (event.key === "Tab") {
      this.consume(event);
      const buttons = [this.options.audio, this.options.subtitles, this.options.fullscreen].filter(button => !button.disabled);
      const index = buttons.indexOf(doc.activeElement as HTMLButtonElement);
      const next = index < 0 ? (event.shiftKey ? buttons.length - 1 : 0) :
        (index + (event.shiftKey ? buttons.length - 1 : 1)) % buttons.length;
      buttons[next]?.focus({ preventScroll: true });
    } else if (!this.contains(event.target)) this.consume(event);
  }

  /** Startup dismissal returns keyboard focus only, never mouse capture or prior UI focus. */
  open(startup = false): void {
    if (this.disposed || this.opened || !this.options.canOpen()) return;
    const { document: doc, launcher, overlay } = this.options;
    this.previousFocus = !startup && doc.activeElement instanceof HTMLElement ? doc.activeElement : null;
    this.restoreLock = !startup && doc.pointerLockElement === doc.getElementById("scene");
    // Releases must precede all input guards/inertness so held native input clears.
    this.options.clearInputs();
    this.launcherTouch = undefined;
    this.opened = true;
    this.options.ownership(true);
    if (doc.pointerLockElement) doc.exitPointerLock?.();
    for (const element of this.options.background) { this.inert.set(element, element.inert); element.inert = true; }
    launcher.hidden = true;
    launcher.setAttribute("aria-expanded", "true");
    overlay.hidden = false;
    this.outsideDown = false;
    this.focusFirst();
    this.publish();
  }

  /** Only an explicit user dismissal may restore mouse capture. */
  close(explicit = false): void {
    this.launcherTouch = undefined;
    if (!explicit) this.qHeld = false;
    if (!this.opened) return;
    this.opened = false;
    this.options.overlay.hidden = true;
    this.options.launcher.hidden = this.disposed;
    this.options.launcher.setAttribute("aria-expanded", "false");
    for (const [element, previous] of this.inert) element.inert = previous;
    this.inert.clear();
    this.options.ownership(false);
    this.publish();
    if (explicit && this.options.canResume()) {
      if (this.previousFocus && !this.options.overlay.contains(this.previousFocus) &&
          this.previousFocus !== this.options.launcher &&
          this.previousFocus !== this.options.document.getElementById("scene") && !this.previousFocus.inert) {
        this.previousFocus.focus({ preventScroll: true });
      } else this.options.resume(this.restoreLock && !this.options.window.matchMedia?.("(pointer: coarse)").matches);
    }
    this.restoreLock = false;
    this.previousFocus = null;
    this.outsideDown = false;
  }

  private publish(): void {
    this.options.window.dispatchEvent(new CustomEvent("weaver-controller-change", { detail: { open: this.opened } }));
  }

  update(state: ControllerHudState): void {
    if (this.disposed) return;
    const { audio, subtitles, fullscreen, status } = this.options;
    for (const [button, on] of [[audio, state.audioEnabled], [subtitles, state.subtitles], [fullscreen, state.fullscreen]] as const) {
      button.setAttribute("aria-pressed", String(on));
      button.dataset.state = on ? "on" : "off";
    }
    // Pending playback may be switched OFF, but must not queue another enabling attempt.
    audio.disabled = !state.audioEnabled && state.audioPending;
    if (this.opened && audio.disabled && this.options.document.activeElement === audio) this.focusFirst();
    subtitles.disabled = false;
    fullscreen.disabled = !state.fullscreenAvailable || state.fullscreenPending;
    audio.setAttribute("aria-busy", String(state.audioPending));
    fullscreen.setAttribute("aria-busy", String(state.fullscreenPending));
    status.textContent = state.audioFailed ? (state.audioEnabled ?
      "Audio unavailable. Select Audio to turn OFF, then select again to retry/turn ON; sound requires a fresh press when ready." :
      "Audio unavailable. Select Audio to retry; then select again when ready for sound.") :
      state.fullscreenError ? "Fullscreen failed. Select Fullscreen to retry." :
      state.audioPending ? (state.audioEnabled ?
              "Preparing audio… Select Audio to turn OFF and cancel sound." :
              "Preparing audio… Select Audio again when ready for sound.") :
      !state.audioReady ? "Movie preparing. Sound requires a fresh Audio press when ready." :
      state.fullscreenPending ? "Changing fullscreen…" :

      "";
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.close(false);
    this.options.launcher.hidden = true;
    for (const cleanup of this.cleanups.splice(0)) cleanup();
  }
}
