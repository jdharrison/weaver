import type { TheaterVideoState } from "./theater-video-player.js";

export interface FullscreenState { available: boolean; active: boolean; pending: boolean; error: boolean }
declare global {
  interface Window {
    jdharrisonToggleFullscreen?: () => unknown;
    jdharrisonGetFullscreenState?: () => FullscreenState;
  }
}
export function fullscreenState(value: unknown): value is FullscreenState {
  return typeof value === "object" && value !== null &&
    ["available", "active", "pending", "error"].every((key) => typeof (value as Record<string, unknown>)[key] === "boolean");
}
export function soundPanelState(state: TheaterVideoState): number {
  return !state.ready ? 0 : state.soundPending ? 2 : state.soundError ? 4 : state.audible ? 3 : 1;
}
export function fullscreenPanelState(state: FullscreenState): number {
  return !state.available ? 0 : state.pending ? 2 : state.error ? 4 : state.active ? 3 : 1;
}

/** Parent functions are installed on this child window; never inspect parent DOM. */
export class FullscreenControl {
  private pending = false;
  private error = false;
  private disposed = false;
  private readonly hostEvent = (event: Event): void => {
    if (this.bridge() && fullscreenState((event as CustomEvent).detail)) this.changed((event as CustomEvent<FullscreenState>).detail);
  };
  private readonly documentEvent = (): void => { this.publish(); };
  private readonly documentError = (): void => { this.pending = false; this.error = true; this.publish(); };
  private readonly child: Window;
  private readonly doc: Document;
  private readonly changed: (state: FullscreenState) => void;
  constructor(child: Window, doc: Document, changed: (state: FullscreenState) => void) {
    this.child = child;
    this.doc = doc;
    this.changed = changed;
    child.addEventListener("jdharrison-fullscreen-state", this.hostEvent);
    doc.addEventListener("fullscreenchange", this.documentEvent);
    doc.addEventListener("fullscreenerror", this.documentError);
  }
  private bridge(): boolean {
    return typeof this.child.jdharrisonToggleFullscreen === "function" && typeof this.child.jdharrisonGetFullscreenState === "function";
  }
  get state(): FullscreenState {
    if (this.bridge()) {
      try {
        const state = this.child.jdharrisonGetFullscreenState!();
        if (fullscreenState(state)) return state;
      } catch { /* An invalid bridge must not fall back to child fullscreen. */ }
      return { available: false, active: false, pending: false, error: true };
    }
    if (this.child.parent && this.child.parent !== this.child) {
      return { available: false, active: false, pending: false, error: false };
    }
    return { available: this.doc.fullscreenEnabled === true && typeof this.doc.documentElement.requestFullscreen === "function" && typeof this.doc.exitFullscreen === "function",
      active: !!this.doc.fullscreenElement, pending: this.pending, error: this.error };
  }
  publish(): void { if (!this.disposed) this.changed(this.state); }
  toggle(): void {
    if (this.disposed || !this.state.available || this.state.pending) return;
    if (this.bridge()) {
      try {
        const result = this.child.jdharrisonToggleFullscreen!();
        this.publish();
        void Promise.resolve(result).then(() => this.publish(), () => this.publish());
      } catch { this.changed({ ...this.state, error: true, pending: false }); }
      return;
    }
    this.pending = true;
    this.error = false;
    this.publish();
    try {
      const result = this.doc.fullscreenElement ? this.doc.exitFullscreen() : this.doc.documentElement.requestFullscreen();
      void Promise.resolve(result).then(() => { this.pending = false; this.publish(); }, () => this.documentError());
    } catch { this.documentError(); }
  }
  dispose(): void {
    this.disposed = true;
    this.child.removeEventListener("jdharrison-fullscreen-state", this.hostEvent);
    this.doc.removeEventListener("fullscreenchange", this.documentEvent);
    this.doc.removeEventListener("fullscreenerror", this.documentError);
  }
}

export interface CanvasPanelOptions {
  window: Window;
  document: Document;
  canvas: HTMLCanvasElement;
  enabled: () => boolean;
  pick: (x: number, y: number) => number;
  activate: (panel: number) => void;
}
/** Capture before Winit. Only hits own an input sequence; misses pass untouched. */
export function installCanvasPanels(options: CanvasPanelOptions): () => void {
  const { window: child, document: doc, canvas } = options;
  const pointers = new Map<number, { panel: number; x: number; y: number; moved: boolean; touch: boolean }>();
  let suppressCompatibility = false;
  const consume = (event: Event): void => { event.preventDefault(); event.stopImmediatePropagation(); };
  const pick = (x: number, y: number): number => {
    if (!options.enabled()) return 0;
    if (doc.pointerLockElement === canvas) return options.pick(0, 0);
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0 || x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return 0;
    return options.pick(2 * (x - rect.left) / rect.width - 1, 1 - 2 * (y - rect.top) / rect.height);
  };
  const down = (event: PointerEvent): void => {
    if (event.target !== canvas || !event.isTrusted || event.button !== 0) return;
    if (event.pointerType === "touch" && event.isPrimary === false) {
      for (const pointer of pointers.values()) if (pointer.touch) pointer.moved = true;
      return;
    }
    suppressCompatibility = false;
    const panel = pick(event.clientX, event.clientY);
    if (panel !== 1 && panel !== 2) return;
    consume(event);
    suppressCompatibility = true;
    const touch = event.pointerType === "touch";
    pointers.set(event.pointerId, { panel, x: event.clientX, y: event.clientY, moved: false, touch });
    if (!touch) options.activate(panel);
  };
  const move = (event: PointerEvent): void => {
    const pointer = pointers.get(event.pointerId);
    if (!pointer) return;
    if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) > 8) pointer.moved = true;
    consume(event);
  };
  const up = (event: PointerEvent): void => {
    const pointer = pointers.get(event.pointerId);
    if (!pointer) return;
    move(event);
    pointers.delete(event.pointerId);
    if (pointer.touch && !pointer.moved && event.isTrusted && pick(event.clientX, event.clientY) === pointer.panel) options.activate(pointer.panel);
  };
  const cancel = (event: PointerEvent): void => { if (pointers.delete(event.pointerId)) consume(event); };
  const compatibility = (event: Event): void => {
    if (event.target === canvas && suppressCompatibility) consume(event);
  };
  const registrations: [string, EventListener][] = [
    ["pointerdown", down as EventListener], ["pointermove", move as EventListener],
    ["pointerup", up as EventListener], ["pointercancel", cancel as EventListener],
    ...["mousedown", "mouseup", "click", "dblclick", "touchstart", "touchmove", "touchend", "touchcancel"].map((type): [string, EventListener] => [type, compatibility]),
  ];
  for (const [type, listener] of registrations) child.addEventListener(type, listener, { capture: true, passive: false });
  return () => { pointers.clear(); for (const [type, listener] of registrations) child.removeEventListener(type, listener, true); };
}
