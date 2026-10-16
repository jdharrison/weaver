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
  return state.disposed ? 0 : state.failed ? 4 : !state.ready ? 5 :
    state.soundPending || state.buffering ? 2 : state.soundError ? 4 : state.audible ? 3 : 1;
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

export interface CanvasInteractionOptions {
  window: Window;
  document: Document;
  canvas: HTMLCanvasElement;
  enabled: () => boolean;
  /** Rust resolves wall controls before seats; zero means no actionable target. */
  pick: (x: number, y: number) => number;
  activate: (target: number) => void;
  /** Clear Rust's pending intent and snapshot without permanently retiring the scene. */
  invalidate: () => void;
}

// Matches the authored catalog in theater.rs; seat codes are one-based in poses.
export const SEAT_COUNT = 24;

export function isSeatInteraction(target: number): boolean {
  return target === 200 || (Number.isInteger(target) && target >= 100 && target < 100 + SEAT_COUNT);
}

/** Mouse hits capture before Winit; touch taps observe without stealing move/look. */
export function installCanvasInteractions(options: CanvasInteractionOptions): { reset(): void; dispose(): void } {
  const { window: child, document: doc, canvas } = options;
  const pointers = new Map<number, { target: number; x: number; y: number; started: number; moved: boolean; touch: boolean }>();
  const touches = new Set<number>();
  let disposed = false;
  let suppressCompatibility = false;
  const consume = (event: Event): void => { event.preventDefault(); event.stopImmediatePropagation(); };
  const pick = (event: PointerEvent): number => {
    if (disposed || !options.enabled() || doc.hidden || canvas.inert) return 0;
    const locked = doc.pointerLockElement === canvas;
    let target: number;
    if (locked) target = options.pick(0, 0);
    else {
      const { clientX: x, clientY: y } = event;
      const rect = canvas.getBoundingClientRect();
      if (![x, y, rect.width, rect.height].every(Number.isFinite) || rect.width <= 0 || rect.height <= 0 ||
          x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return 0;
      target = options.pick(2 * (x - rect.left) / rect.width - 1, 1 - 2 * (y - rect.top) / rect.height);
    }
    // An unlocked mouse click must still be able to resume look without standing up.
    if (target === 200 && !locked && event.pointerType !== "touch") return 0;
    return target === 1 || target === 2 || target === 3 || target === 4 || target === 5 || isSeatInteraction(target) ? target : 0;
  };
  const down = (event: PointerEvent): void => {
    if (disposed || !event.isTrusted || event.button !== 0) return;
    const touch = event.pointerType === "touch";
    if (touch) {
      touches.add(event.pointerId);
      if (event.isPrimary === false || touches.size > 1) {
        for (const pointer of pointers.values()) if (pointer.touch) pointer.moved = true;
        return;
      }
    }
    if (event.target !== canvas) return;
    suppressCompatibility = false;
    const target = pick(event);
    if (target === 0) return;
    suppressCompatibility = true;
    pointers.set(event.pointerId, { target, x: event.clientX, y: event.clientY, started: event.timeStamp, moved: false, touch });
    // Winit needs the original touch down to assign the left/right thumb. Once
    // consumed, releasing a drag after its threshold cannot restore that origin.
    if (!touch) { consume(event); options.activate(target); }
  };
  const move = (event: PointerEvent): void => {
    const pointer = pointers.get(event.pointerId);
    if (!pointer) return;
    if (Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) > 8) pointer.moved = true;
    if (!pointer.touch) consume(event);
  };
  const up = (event: PointerEvent): void => {
    touches.delete(event.pointerId);
    const pointer = pointers.get(event.pointerId);
    if (!pointer) return;
    move(event);
    pointers.delete(event.pointerId);
    const duration = event.timeStamp - pointer.started;
    if (pointer.touch && event.pointerType === "touch" && !pointer.moved && duration >= 0 && duration <= 500 &&
        event.isTrusted && pick(event) === pointer.target) options.activate(pointer.target);
  };
  const cancel = (event: PointerEvent): void => {
    touches.delete(event.pointerId);
    const pointer = pointers.get(event.pointerId);
    pointers.delete(event.pointerId);
    if (pointer) {
      options.invalidate();
      if (!pointer.touch) consume(event);
    }
  };
  const reset = (): void => {
    if (disposed) return;
    pointers.clear();
    touches.clear();
    // A synchronous panel action can change focus before its compatibility click.
    // Keep suppressing that click until the next primary down starts a new gesture.
    options.invalidate();
  };
  const blur = (event: Event): void => { if (event.target === child || event.target === canvas) reset(); };
  const focus = (event: Event): void => { if (event.target !== canvas) reset(); };
  const visibility = (): void => { if (doc.hidden || doc.visibilityState === "hidden") reset(); };
  const compatibility = (event: Event): void => {
    if (event.target === canvas && (event as MouseEvent).button === 0 && suppressCompatibility) consume(event);
  };
  const registrations: [EventTarget, string, EventListener][] = [
    [child, "pointerdown", down as EventListener], [child, "pointermove", move as EventListener],
    [child, "pointerup", up as EventListener], [child, "pointercancel", cancel as EventListener],
    [child, "lostpointercapture", cancel as EventListener], [child, "touchcancel", reset],
    [child, "blur", blur], [child, "focusin", focus], [child, "pagehide", reset],
    [child, "resize", reset], [child, "orientationchange", reset],
    [doc, "visibilitychange", visibility], [doc, "pointerlockchange", reset],
    // Suppress primary mouse compatibility clicks only, never native touch sequences.
    ...["mousedown", "mouseup", "click", "dblclick"].map((type): [EventTarget, string, EventListener] => [child, type, compatibility]),
  ];
  for (const [owner, type, listener] of registrations) owner.addEventListener(type, listener, { capture: true, passive: false });
  return { reset, dispose: () => {
    if (disposed) return;
    reset();
    disposed = true;
    for (const [owner, type, listener] of registrations) owner.removeEventListener(type, listener, true);
  } };
}
