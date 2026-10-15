import type { TheaterVideoPlayer } from "./theater-video-player.js";
import type { TheaterScheduleController } from "./theater-schedule-controller.js";

interface TheaterStartupOptions {
  window: Window;
  document: Document;
  root: HTMLElement;
  status: HTMLElement;
  play: HTMLButtonElement;
  retry: HTMLButtonElement;
  continueMuted: HTMLButtonElement;
  background: HTMLElement[];
  player: Pick<TheaterVideoPlayer, "state" | "setMuted">;
  schedule: Pick<TheaterScheduleController, "state" | "play" | "retry" | "continueMuted">;
  onOpenChange: (open: boolean) => void;
  onDismiss: () => void;
}

/** Presentation only: the schedule owns the single current source and UTC alignment. */
export class TheaterStartup {
  private opened = false;
  private presented = false;
  private sceneReady = false;
  private visible = true;
  private disposed = false;
  private starting = false;
  private slow = false;
  private actionFailed = false;
  private acting = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly background = new Map<HTMLElement, boolean>();
  private readonly listeners: (() => void)[] = [];
  private readonly options: TheaterStartupOptions;

  constructor(options: TheaterStartupOptions) {
    this.options = options;
    this.visible = options.document.visibilityState === "visible";
    options.root.hidden = true;
    options.continueMuted.disabled = false;
    this.listen(options.play, "click", (event) => { if (event.isTrusted) this.play(); });
    this.listen(options.retry, "click", (event) => { if (event.isTrusted) this.retry(); });
    this.listen(options.continueMuted, "click", () => this.continueMuted());
    this.listen(options.window, "keydown", (event) => this.keydown(event as KeyboardEvent), true);
    this.listen(options.window, "keyup", (event) => {
      if (this.opened && this.visible) event.stopImmediatePropagation();
    }, true);
    for (const type of ["pointerdown", "pointerup", "pointermove", "mousedown", "mouseup", "click", "dblclick", "wheel", "touchstart", "touchmove", "touchend", "beforeinput", "input", "compositionstart", "compositionupdate", "compositionend"]) {
      this.listen(options.window, type, (event) => {
        if (this.opened && this.visible && !this.inside(event.target)) {
          event.preventDefault();
          event.stopImmediatePropagation();
        }
      }, true);
      this.listen(options.root, type, (event) => event.stopPropagation());
    }
    this.listen(options.document, "focusin", () => {
      if (this.opened && this.visible && !this.inside(options.document.activeElement)) this.focus();
    });
    this.listen(options.document, "pointerlockchange", () => {
      if (this.opened && options.document.pointerLockElement) options.document.exitPointerLock?.();
    });
  }

  get isOpen(): boolean { return this.opened; }

  setSceneReady(ready: boolean): void {
    this.sceneReady = ready;
    if (ready && !this.presented && !this.disposed) {
      this.presented = true;
      this.show();
    }
  }

  /** World Sound uses the same confirmation, keeping healthy playback in the gesture stack. */
  requestSound(): void {
    this.show();
    if (!this.opened || !this.visible || this.disposed) return;
    if (this.options.player.state.failed || this.options.schedule.state.failed) this.retry();
    else if (this.options.player.state.soundPending) { this.starting = true; this.update(); }
    else this.play();
  }

  update(): void {
    if (!this.opened || this.disposed || this.acting) return;
    const { player, schedule, root, status, play, retry } = this.options;
    const state = player.state;
    root.hidden = !this.visible;
    if (!this.visible) return;
    // A fulfilled play promise is not proof of either audible playback or a frame.
    if (this.starting && state.audible && state.frameReady) { this.dismiss(); return; }
    const failed = this.actionFailed || state.failed || schedule.state.failed || state.soundError;
    if (failed) this.starting = false;
    const ready = this.metadataReady();
    const waiting = !failed && (this.starting || !ready);
    if (!waiting) { this.cancelTimer(); this.slow = false; }
    else if (!this.slow && this.timer === undefined) {
      this.timer = setTimeout(() => {
        this.timer = undefined;
        if (!this.opened || !this.visible || this.disposed) return;
        this.slow = true;
        this.update();
      }, 12_000);
    }
    const message = failed ? "The movie couldn’t start. Retry, or continue without sound." :
      this.slow ? "This is taking longer than expected. Retry, or continue without sound." :
      this.starting ? (state.buffering ? "Buffering the current movie…" : "Starting playback…") :
      ready ? "Ready" : "Preparing the current movie…";
    if (status.textContent !== message) status.textContent = message;
    root.dataset.state = failed ? "failed" : this.slow ? "slow" : this.starting ? "starting" : ready ? "ready" : "preparing";
    play.disabled = failed || !ready || this.starting;
    retry.hidden = !failed && !this.slow;
    retry.disabled = schedule.state.loading;
    const active = this.options.document.activeElement;
    if ((active === play && play.disabled) || (active === retry && (retry.hidden || retry.disabled))) this.focus();
  }

  setVisible(visible: boolean): void {
    if (this.disposed) return;
    this.visible = visible;
    if (!visible) {
      this.cancelTimer();
      this.starting = false;
      this.slow = false;
      this.options.root.hidden = true;
    } else if (this.opened) { this.update(); this.focus(); }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.close();
    for (const remove of this.listeners.splice(0)) remove();
  }

  private metadataReady(): boolean {
    const state = this.options.player.state;
    return state.ready && state.duration > 0 && !state.failed && !state.disposed &&
      !this.options.schedule.state.loading && !this.options.schedule.state.failed;
  }

  private show(): void {
    if (!this.sceneReady || this.disposed || this.opened) return;
    this.options.onOpenChange(true); // Release held game keys before the capture guard takes over.
    this.opened = true;
    this.starting = this.slow = this.actionFailed = false;
    for (const element of this.options.background) {
      this.background.set(element, element.inert);
      element.inert = true;
    }
    this.update();
    if (this.visible) this.focus();
  }

  private play(): void {
    if (!this.opened || !this.visible || this.disposed || !this.metadataReady() || this.options.player.state.soundPending) return;
    this.starting = true;
    this.slow = this.actionFailed = false;
    this.cancelTimer();
    this.acting = true;
    try {
      this.options.schedule.play(true); // No await: preserve the trusted click's activation.
      // A UTC boundary can select a new source in this same gesture. Prepare it,
      // then offer Play again rather than replaying audible intent asynchronously.
      if (!this.metadataReady()) this.starting = false;
    } catch { this.actionFailed = true; }
    finally { this.acting = false; }
    this.update();
  }

  private retry(): void {
    if (!this.opened || !this.visible || this.disposed || this.acting || this.options.schedule.state.loading) return;
    const state = this.options.player.state;
    const failed = state.failed || this.options.schedule.state.failed || state.soundError || this.actionFailed;
    if (!this.slow && !failed) return;
    if (!this.slow && this.metadataReady() && !state.soundPending) { this.play(); return; }
    const reloadCurrent = this.slow;
    this.cancelTimer();
    this.starting = this.slow = this.actionFailed = false;
    this.acting = true;
    try {
      if (reloadCurrent) this.options.schedule.retry(true);
      else this.options.schedule.retry();
    } catch { this.actionFailed = true; }
    finally { this.acting = false; }
    this.update();
  }

  private continueMuted(): void {
    if (!this.opened || this.disposed) return;
    this.dismiss();
    this.options.player.setMuted(true);
    this.options.schedule.continueMuted();
  }

  private dismiss(): void {
    this.close();
    this.options.onDismiss();
  }

  private close(): void {
    this.cancelTimer();
    this.options.root.hidden = true;
    if (!this.opened) return;
    this.opened = false;
    this.starting = false;
    for (const [element, inert] of this.background) element.inert = inert;
    this.background.clear();
    this.options.onOpenChange(false);
  }

  private inside(target: EventTarget | null): boolean {
    return target instanceof Node && this.options.root.contains(target);
  }

  private buttons(): HTMLButtonElement[] {
    return [this.options.play, this.options.retry, this.options.continueMuted].filter(button => !button.hidden && !button.disabled);
  }

  private focus(): void { this.buttons()[0]?.focus({ preventScroll: true }); }

  private keydown(event: KeyboardEvent): void {
    if (!this.opened || !this.visible) return;
    event.stopImmediatePropagation();
    if (event.key === "Escape") { event.preventDefault(); this.continueMuted(); }
    else if (event.key === "Tab") {
      event.preventDefault();
      const buttons = this.buttons();
      const index = buttons.indexOf(this.options.document.activeElement as HTMLButtonElement);
      const next = index < 0 ? (event.shiftKey ? buttons.length - 1 : 0) :
        (index + (event.shiftKey ? -1 : 1) + buttons.length) % buttons.length;
      buttons[next]?.focus({ preventScroll: true });
    } else if (!this.inside(event.target)) event.preventDefault();
  }

  private cancelTimer(): void { clearTimeout(this.timer); this.timer = undefined; }

  private listen(target: EventTarget, type: string, listener: EventListener, capture = false): void {
    target.addEventListener(type, listener, { capture, passive: false });
    this.listeners.push(() => target.removeEventListener(type, listener, capture));
  }
}
