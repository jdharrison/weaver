import {
  AdmissionStatus,
  AuthenticationScheme,
  DeliveryClass,
  MessageKind,
  QueueState,
  WovenClient,
  type DecodedEnvelope,
} from "@signalweave/woven-client";
import { decodeApplicationPayload } from "./application-payload.js";
import { BrowserLifecycle, receiveWhileCurrent } from "./browser-lifecycle.js";
import { fetchBoundedText, fetchLobbyBootstrap, readLobbyConfig, type LobbyConfig } from "./lobby-bootstrap.js";
import { loadUserProfile, saveUserProfile, type ProfileStorage, type UserProfile } from "./user-profile.js";
import { scheduleConnectionLog } from "./connection-log.js";
import { OutboundScheduler, parsePublishRateHz, PUBLISH_RATE_CONFIGURATION_ERROR } from "./outbound-scheduler.js";
import { DEFAULT_THEATER_VIDEO, TheaterVideoPlayer, type TheaterVideoState } from "./theater-video-player.js";
import { DEFAULT_THEATER_SCHEDULE, TheaterScheduleController } from "./theater-schedule-controller.js";
import { FullscreenControl, fullscreenPanelState, installCanvasInteractions, isSeatInteraction, SEAT_COUNT, soundPanelState, type FullscreenState } from "./control-panels.js";
import { resolveTheaterSchedule, type TheaterSchedule } from "./theater-schedule.js";
import { captionsForMedia, TheaterSubtitles } from "./theater-subtitles.js";
import { fetchWasmWithProgress } from "./wasm-loading.js";
import { TheaterStartup } from "./theater-startup.js";
import { ControllerHud } from "./controller-hud.js";
import { isPortfolioInteraction, openPortfolioLink } from "./portfolio-links.js";

type NpcInteractionSnapshot = {
  version: 1; npcId: string; profile: string; displayName: string;
  sceneGeneration: string; frameSequence: string; distanceSquared: number;
  radiusWorldUnits: number; room: string; showState: string;
  nearbyProject: string | null; playerHasSeenIntro: boolean;
};
type NpcScheduleAction =
  | { type: "spawn" | "move"; point: string; speed?: number }
  | { type: "wait" | "idle" | "watch" | "leave_seat" | "despawn"; seconds?: number }
  | { type: "sit"; seatId: string; seconds?: number };
type ScriptedNpcDefinition = {
  id: string; displayName: string; avatarId: string; profile: string;
  permanent: boolean; spawnOffsetSeconds: number; knowledgeStyle: string;
  nearbyProject: string | null;
  interaction: { enabled: boolean; radius: number; promptLabel: string };
  schedule: NpcScheduleAction[];
};

type Scope = {
  namespaceId: bigint;
  sessionId: bigint;
  spaceId: bigint;
  spaceEpoch: bigint;
  channelId: bigint;
};

type PendingPublish =
  | { kind: "latest" | "reliable"; sequence: bigint; payload: string }
  | { kind: "log"; message: string };

type Connection = {
  client: WovenClient;
  entityId: bigint;
  scope: Scope;
  cancelConnectionLog: (() => void) | null;
  scheduler: OutboundScheduler;
  publisher: {
    pending: PendingPublish[];
  };
  posePublisher: {
    pending: {
      sequence: bigint;
      position: { x: number; y: number; z: number };
      payload: Uint8Array;
    } | null;
  };
};

type ConnectionState = "offline" | "connecting" | "online";
type JoinMode = "visitor" | "operator" | "local";
type ConnectionInput = { url: string; token: string; scope: Scope; managed: boolean; hash: ArrayBuffer | null };

declare global {
  var weaverSendChat: (text: string) => boolean;
  var weaverSetChatFocus: (active: boolean) => boolean;
  var weaverResumeGameInput: () => boolean;
  var weaverChatMessage: ((entityId: string, displayName: string, text: string, isLocal: boolean) => void) | undefined;
  var weaverHostRequestChat: (() => boolean) | undefined;
  var weaverRequestChatOpen: (() => boolean) | undefined;
  var weaverNpcInteraction: ((snapshot: NpcInteractionSnapshot | null) => void) | undefined;
  var weaverNpcInteractionFrame: (encoded: string) => void;
  var weaverGetNpcInteractionContext: () => string | null;
  var weaverSubmitNpcQuestion: (npcId: string, requestId: string, text: string) => boolean;
  var weaverPresentNpcResponse: (npcId: string, requestId: string, answer: string, gesture: string) => boolean;
  var weaverClearNpcDialogue: () => void;
  var weaverGetNpcDefinitions: () => ScriptedNpcDefinition[] | null;
  // Internal Rust capacity query; the parent adapter owns the weaverHost* bridge.
  var weaverChatSendCapacity: () => number;
  var weaverDisplayNameChanged: (value: string) => void;
  var weaverRealtimePublishLatest: (sequence: bigint, payload: string) => void;
  var weaverRealtimePublishReliable: (sequence: bigint, payload: string) => void;
  var weaverRealtimePublishUnreliable: (sequence: bigint, payload: Uint8Array) => void;
  var weaverRealtimePublishPositionedUnreliable: (
    sequence: bigint,
    x: number,
    y: number,
    z: number,
    payload: Uint8Array,
  ) => void;
  var weaverRealtimeFatal: (reason: string) => void;
  var weaverDisposeTheaterVideo: () => void;
  var weaverSceneReady: () => void;
  var weaverSceneFatal: (message: string) => void;
}

type LoadStage = "shell" | "module" | "wasm" | "profile" | "started" | "scene";
const loadMilestones: Record<LoadStage, number> = {
  shell: 1, module: 3, wasm: 7, profile: 10, started: 12, scene: 14,
};
let loadCompleted = 0;
let loadFailed = false;
function reportLoadProgress(stage: LoadStage): void {
  const completed = loadMilestones[stage];
  if (loadFailed || completed <= loadCompleted) return;
  loadCompleted = completed;
  bootActivity();
  notifyLoading({ event: "progress", stage, completed, total: 14 });
}

function notifyLoading(message: Record<string, string | number | null>): void {
  if (loadFailed || window.parent === window || !window.parent) return;
  try {
    window.parent.postMessage(JSON.stringify({ type: "weaver-host", version: 1, ...message }), window.location.origin);
  } catch { /* A departing host must not interrupt engine initialization. */ }
}

function bootActivity(): void {
  clearTimeout(bootTimer);
  if (loadFailed || loadCompleted === loadMilestones.scene) return;
  bootTimer = setTimeout(() => {
    globalThis.weaverSceneFatal("Scene loading stalled. Check the connection and browser graphics support.");
  }, 20_000);
}

let initializingReported = false;
function reportInitializing(): void {
  if (loadFailed || initializingReported) return;
  initializingReported = true;
  bootActivity();
  notifyLoading({ event: "initializing" });
}

const MAX_U64 = 18_446_744_073_709_551_615n;
const SETUP_OPERATION_TIMEOUT_MS = 10_000;
const MAX_PENDING_PUBLISHES = 32;
const POSE_CHANNEL_ID = 4n;
const LEGACY_POSE_PAYLOAD_BYTES = 25;
const POSE_PAYLOAD_BYTES = 26;

// Float, world-bound and seat eligibility validation remains in the shared Rust codec.
function validPosePayload(payload: unknown): payload is Uint8Array {
  return payload instanceof Uint8Array && (
    (payload.byteLength === LEGACY_POSE_PAYLOAD_BYTES && payload[0] === 1) ||
    (payload.byteLength === POSE_PAYLOAD_BYTES && payload[0] === 2 && payload[1] <= SEAT_COUNT)
  );
}
const LOCAL_DEVELOPMENT_ORIGINS = new Set([
  "http://127.0.0.1:8000",
  "http://localhost:8000",
]);
const encoder = new TextEncoder();
const lifecycle = new BrowserLifecycle(document.visibilityState === "visible");
type FirstPersonWasm = typeof import("./pkg/first_person_lab.js") & {
  GenerateGuestName(): string;
  IsLegacyUserName(name: string): boolean;
  set_minimal_ui(value: boolean): void;
  send_host_chat(text: string): boolean;
  set_host_chat_focus(active: boolean): void;
  reset_host_chat(): void;
  start(): void;
  attach_theater_video(video: HTMLVideoElement): boolean;
  mark_theater_video_frame(): boolean;
  detach_theater_video(): void;
  set_room_lighting(lobby: number, auditorium: number): boolean;
  pick_control_panel(x: number, y: number): number;
  pick_scene_interaction(x: number, y: number): number;
  activate_seat_interaction(target: number): boolean;
  invalidate_scene_interactions(): void;
  set_control_panel_state(panel: number, state: number): boolean;
  invalidate_control_panels(): void;
  get_npc_interaction_context(): string | undefined;
  submit_npc_question(npcId: string, requestId: string, text: string): boolean;
  present_npc_response(npcId: string, requestId: string, answer: string, gesture: string): boolean;
  clear_npc_dialogue(): void;
  get_npc_definitions(): string;
  set_npc_interaction_active(active: boolean): void;
  set_npc_show_cycle(anchorUnixSeconds: number, cycleSeconds: number): boolean;
};
let wasm: FirstPersonWasm;
let networkState: ConnectionState = "offline";
let hasConnected = false;
let lobbyConfig: LobbyConfig | null = null;
let lobbyConfigurationError: string | null = null;
let publishRateHz: number | null = null;
let publishRateConfigurationError: string | null = null;
let retryMode: JoinMode = LOCAL_DEVELOPMENT_ORIGINS.has(window.location.origin) ? "local" : "operator";
let bootTimer: ReturnType<typeof setTimeout> | undefined;
const bootAbort = new AbortController();
let connection: Connection | null = null;
let connectingClient: WovenClient | null = null;
let setupAbort: AbortController | null = null;
let connectionGeneration = 0;
let connectionAttemptActive = false;
let wasmReady = false;
let npcDisposed = false;
let npcActive = false;
let npcSchedule: TheaterSchedule | null = null;
let npcCycleKey = "";
let hostChatFocused = false;
let theaterControlsFocused = false;
let theaterStartupFocused = false;
let controllerHudFocused = false;
let controllerHud: ControllerHud | undefined;
let subtitlePanelState = 3;
let enterHeld = false;
const heldGameKeys = new Map<string, string>();
const movementCodes = new Set(["KeyW", "KeyA", "KeyS", "KeyD", "ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight", "ShiftLeft", "ShiftRight"]);
let userProfile: UserProfile | null = null;
const profileStorage: ProfileStorage | null = (() => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
})();

const form = requiredElement<HTMLFormElement>("network-form");
const connectButton = requiredElement<HTMLButtonElement>("connect-network");
const disconnectButton = requiredElement<HTMLButtonElement>("disconnect-network");
const networkStatus = requiredElement<HTMLElement>("network-status");
const connectionBadge = requiredElement<HTMLElement>("connection-badge");
const connectionLabel = requiredElement<HTMLElement>("connection-label");
const authSelect = requiredElement<HTMLSelectElement>("woven-auth");
const tokenInput = requiredElement<HTMLInputElement>("woven-token");
const displayNameInput = requiredElement<HTMLInputElement>("display-name");
const retryButton = requiredElement<HTMLButtonElement>("join-network");
const reloadLink = requiredElement<HTMLAnchorElement>("reload-scene");
const settings = requiredElement<HTMLDetailsElement>("multiplayer-settings");
const theaterPanel = requiredElement<HTMLElement>("theater-controls");
const subtitleControls = requiredElement<HTMLElement>("subtitle-controls");
const theaterMedia = requiredElement<HTMLVideoElement>("theater-video");
const theaterPlay = requiredElement<HTMLButtonElement>("theater-play");
const theaterRestart = requiredElement<HTMLButtonElement>("theater-restart");
const theaterMute = requiredElement<HTMLButtonElement>("theater-mute");
let panelsDisposed = false;
let canvasInteractions: ReturnType<typeof installCanvasInteractions> | undefined;
const panelStates = new Map<number, number>();
function setPanelState(panel: number, state: number): void {
  if (!wasmReady || panelsDisposed || panelStates.get(panel) === state) return;
  try { if (wasm.set_control_panel_state(panel, state)) panelStates.set(panel, state); }
  catch { /* Matching generated bindings are required; do not break media diagnostics. */ }
}
const subtitles = new TheaterSubtitles({
  media: theaterMedia,
  hud: requiredElement("theater-subtitle-hud"),
  button: requiredElement<HTMLButtonElement>("theater-subtitles"),
  explanation: requiredElement("theater-subtitle-explanation"),
  storage: {
    getItem: (key) => window.localStorage.getItem(key),
    setItem: (key, value) => window.localStorage.setItem(key, value),
  },
  panel: (state) => { subtitlePanelState = state; setPanelState(3, state); syncControllerHud(); },
});
const fullscreenControl = new FullscreenControl(window, document, (state) => {
  setPanelState(2, fullscreenPanelState(state));
  syncControllerHud(state);
});
const theaterVolume = requiredElement<HTMLInputElement>("theater-volume");
const theaterSeek = requiredElement<HTMLInputElement>("theater-seek");
const theaterTime = requiredElement<HTMLElement>("theater-time");
const theaterStatus = requiredElement<HTMLElement>("theater-status");
const lobbyLight = requiredElement<HTMLInputElement>("theater-lobby-light");
const auditoriumLight = requiredElement<HTMLInputElement>("theater-auditorium-light");
const theaterFollow = requiredElement<HTMLInputElement>("theater-follow-schedule");
const theaterScheduleStatus = requiredElement<HTMLElement>("theater-schedule-status");
theaterFollow.checked = true;
let theaterSchedule: TheaterScheduleController | undefined;
let theaterStartup: TheaterStartup | undefined;
let theaterStartTimer: ReturnType<typeof setTimeout> | undefined;
let theaterStarted = false;
const theaterPlayer = new TheaterVideoPlayer({
  media: theaterMedia,

  scheduled: true,
  onRecover: () => theaterSchedule?.recoverPlayback(),
  attach: () => wasm.attach_theater_video(theaterMedia),
  markFrame: () => wasm.mark_theater_video_frame(),
  detach: () => wasm.detach_theater_video(),
  requestAnimationFrame: (callback) => window.requestAnimationFrame(callback),
  cancelAnimationFrame: (id) => window.cancelAnimationFrame(id),
  visible: document.visibilityState === "visible",
  onChange: (state) => {
    theaterStatus.textContent = state.status;
    theaterPlay.disabled = !state.ready;
    theaterPlay.textContent = state.playing ? "Pause" : "Play";
    syncTheaterSound(state);
    theaterMute.disabled = !state.ready;
    theaterMute.textContent = state.muted ? "Unmute" : "Mute";
    theaterVolume.disabled = !state.ready;
    theaterVolume.value = String(state.volume);
    theaterRestart.disabled = theaterFollow.checked || !state.ready || state.duration === 0;
    theaterSeek.disabled = theaterFollow.checked || !state.ready || state.duration === 0;
    theaterSeek.max = String(state.duration);
    theaterSeek.value = String(state.currentTime);
    theaterTime.textContent = `${movieTime(state.currentTime)} / ${movieTime(state.duration)}`;
    theaterSchedule?.playerChanged();
    theaterStartup?.update();
  },
});

theaterSchedule = new TheaterScheduleController({
  player: theaterPlayer,
  media: theaterMedia,
  waitForGesture: false,
  path: document.querySelector<HTMLMetaElement>('meta[name="weaver-theater-schedule"]')?.content || DEFAULT_THEATER_SCHEDULE,
  origin: window.location.origin,
  localSource: document.querySelector<HTMLMetaElement>('meta[name="weaver-theater-video"]')?.content.trim() || DEFAULT_THEATER_VIDEO,
  visible: document.visibilityState === "visible",
  onScheduleLoaded: (schedule) => {
    npcSchedule = schedule;
    syncNpcRuntime();
  },
  onChange: (status, following) => {
    theaterScheduleStatus.textContent = status;
    theaterFollow.checked = following;
    // Use the schedule's existing publication cadence, not a second timer.
    let captions: string | undefined;
    if (npcSchedule) {
      let scheduled: TheaterSchedule["entries"][number] | undefined;
      try {
        scheduled = resolveTheaterSchedule(npcSchedule, Date.now() / 1000).entry;
      } catch { /* Still match the loaded movie if the schedule clock cannot resolve. */ }
      captions = captionsForMedia(npcSchedule.entries, scheduled, theaterMedia.getAttribute("src") ?? "", window.location.origin);
    }
    subtitles.setSource(captions);
    theaterRestart.disabled = theaterSeek.disabled = following || !theaterPlayer.state.ready || theaterPlayer.state.duration === 0;
    syncTheaterSound(theaterPlayer.state);
    theaterStartup?.update();
  },
});
theaterStartup = new TheaterStartup({
  window, document,
  root: requiredElement("theater-startup"),
  status: requiredElement("theater-startup-status"),
  play: requiredElement<HTMLButtonElement>("theater-startup-play"),
  retry: requiredElement<HTMLButtonElement>("theater-startup-retry"),
  continueMuted: requiredElement<HTMLButtonElement>("theater-startup-continue"),
  background: [requiredElement("scene"), requiredElement("scene-instructions"), requiredElement("connection-controls"), theaterPanel, subtitleControls, requiredElement("controller-launcher")],
  player: theaterPlayer,
  schedule: theaterSchedule,
  onOpenChange: (open) => {
    if (open) {
      clearHeldGameInputs();
      theaterStartupFocused = true;
      controllerHud?.close(false);
      if (document.pointerLockElement) document.exitPointerLock?.();
      enterHeld = false;
    }
    theaterStartupFocused = open;
    syncGameInputFocus();
  },
  onDismiss: () => {
    if (lifecycle.canJoin && !hostChatFocused && !theaterControlsFocused && !controllerHudFocused) {
      requiredElement("scene").focus({ preventScroll: true });
    }
  },
});

function syncControllerHud(fullscreenState?: FullscreenState): void {
  if (!controllerHud) return;
  const state = theaterPlayer.state;
  const fullscreen = fullscreenState ?? fullscreenControl.state;
  controllerHud.update({
    audioEnabled: state.audioEnabled, audioAudible: state.audible, audioPending: state.soundPending || !!theaterSchedule?.state.loading,
    audioFailed: state.failed || state.soundError || !!theaterSchedule?.state.failed,
    audioReady: state.ready, subtitles: subtitlePanelState === 3,
    fullscreen: fullscreen.active, fullscreenAvailable: fullscreen.available,
    fullscreenPending: fullscreen.pending, fullscreenError: fullscreen.error,
  });
}

function syncTheaterSound(state: TheaterVideoState): void {
  subtitles.update(state.audible);
  const playerState = soundPanelState(state);
  const scheduleState = theaterSchedule?.state;
  setPanelState(1, state.disposed ? playerState : scheduleState?.failed ? 4 :
    scheduleState?.loading ? 5 : playerState);
  syncControllerHud();
}

theaterFollow.addEventListener("change", () => theaterSchedule?.setFollowing(theaterFollow.checked));

function queueTheaterStart(): void {
  if (theaterStarted || theaterStartTimer !== undefined || !lifecycle.canJoin ||
      document.visibilityState !== "visible" || theaterPlayer.state.disposed) return;
  // Let the host's synchronous ready wrapper finish before any manifest/media work.
  theaterStartTimer = setTimeout(() => {
    theaterStartTimer = undefined;
    if (!lifecycle.canJoin || document.visibilityState !== "visible" || theaterPlayer.state.disposed) return;
    theaterStarted = true;
    void theaterSchedule?.start();
  }, 0);
}

function cancelTheaterStart(): void {
  clearTimeout(theaterStartTimer);
  theaterStartTimer = undefined;
}

function movieTime(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
}

// Host shutdown is independent of the page/network lifecycle and safe to repeat.
globalThis.weaverDisposeTheaterVideo = (): void => {
  cancelTheaterStart();
  controllerHud?.dispose();
  theaterStartup?.dispose();
  if (!npcDisposed) {
    npcDisposed = true;
    syncNpcRuntime();
    globalThis.weaverClearNpcDialogue();
    publishNpcInteraction(null);
  }
  if (!panelsDisposed) {
    panelsDisposed = true;
    canvasInteractions?.dispose();
    fullscreenControl.dispose();
    if (wasmReady) { try { wasm.invalidate_control_panels(); } catch { /* Already shut down. */ } }
  }
  theaterSchedule?.dispose();
  theaterFollow.disabled = true;
  subtitles.dispose();
  theaterPlayer.dispose();
  theaterMedia.remove();
  lobbyLight.disabled = true;
  auditoriumLight.disabled = true;
};
theaterPlay.addEventListener("click", () => {
  if (theaterPlayer.state.playing) theaterSchedule?.pause();
  else theaterSchedule?.play();
});

theaterMute.addEventListener("click", () => theaterPlayer.setMuted(!theaterPlayer.state.muted));
theaterVolume.addEventListener("input", () => theaterPlayer.setVolume(Number(theaterVolume.value)));
theaterRestart.addEventListener("click", () => { if (!theaterFollow.checked) theaterPlayer.seek(0); });
theaterSeek.addEventListener("input", () => { if (!theaterFollow.checked) theaterPlayer.seek(Number(theaterSeek.value)); });
for (const light of [lobbyLight, auditoriumLight]) {
  light.addEventListener("input", () => {
    if (!wasmReady || !lifecycle.canJoin || theaterPlayer.state.disposed) return;
    try {
      if (!wasm.set_room_lighting(Number(lobbyLight.value), Number(auditoriumLight.value))) {
        theaterStatus.textContent = "Room lighting update rejected";
      }
    } catch {
      theaterStatus.textContent = "Room lighting unavailable · matching WASM assets required";
    }
  });
}

for (const controlRoot of [theaterPanel, subtitleControls]) {
  controlRoot.addEventListener("focusin", () => {
    theaterControlsFocused = true;
    if (document.pointerLockElement && typeof document.exitPointerLock === "function") document.exitPointerLock();
    clearHeldGameInputs();
    syncGameInputFocus();
  });
  controlRoot.addEventListener("focusout", (event) => {
    theaterControlsFocused = event.relatedTarget instanceof Node &&
      (theaterPanel.contains(event.relatedTarget) || subtitleControls.contains(event.relatedTarget));
    syncGameInputFocus();
  });
  for (const event of ["keydown", "keyup", "pointerdown", "pointerup", "click", "wheel"]) {
    controlRoot.addEventListener(event, (input) => input.stopPropagation());
  }
}

canvasInteractions = installCanvasInteractions({
  window, document, canvas: requiredElement<HTMLCanvasElement>("scene"),
  enabled: () => wasmReady && lifecycle.canJoin && !panelsDisposed && !hostChatFocused && !theaterControlsFocused &&
    !theaterStartupFocused && !controllerHudFocused && !editableElement(document.activeElement),
  pick: (x, y) => {
    try { return wasm.pick_scene_interaction(x, y); }
    catch { return 0; } // Stale bindings fail closed; never guess a seat or retry a panel-only pick.
  },
  activate: (target) => {
    if (isSeatInteraction(target)) {
      try { wasm.activate_seat_interaction(target); }
      catch { /* A rejected/retired target must not fall through to another action. */ }
    } else if (isPortfolioInteraction(target)) {
      if (!openPortfolioLink(window, target)) theaterStatus.textContent = "LinkedIn profile link is not configured yet.";
    } else if (target === 3) subtitles.toggle();
    else if (target === 2) fullscreenControl.toggle();
    else if (target === 1) {
      if (theaterPlayer.state.audible && theaterPlayer.state.frameReady) theaterPlayer.setMuted(true);
      else theaterStartup?.requestSound();
    }
  },
  invalidate: () => {
    if (wasmReady) {
      try { wasm.invalidate_scene_interactions(); }
      catch { /* Cleanup also works before matching WASM bindings are available. */ }
    }
  },
});

if (document.documentElement.dataset.weaverPresentation === "minimal") {
  controllerHud = new ControllerHud({
    window, document,
    launcher: requiredElement<HTMLButtonElement>("controller-launcher"),
    overlay: requiredElement("controller-overlay"), dialog: requiredElement("controller-dialog"),
    audio: requiredElement<HTMLButtonElement>("controller-audio"),
    subtitles: requiredElement<HTMLButtonElement>("controller-subtitles"),
    fullscreen: requiredElement<HTMLButtonElement>("controller-fullscreen"), status: requiredElement("controller-status"),
    background: [requiredElement("scene"), requiredElement("scene-instructions"), requiredElement("connection-controls"), theaterPanel, subtitleControls],
    canOpen: () => wasmReady && lifecycle.canJoin && !panelsDisposed && !hostChatFocused && !theaterControlsFocused && !theaterStartupFocused,
    canResume: () => wasmReady && lifecycle.canJoin && !panelsDisposed && !hostChatFocused && !theaterControlsFocused && !theaterStartupFocused,
    clearInputs: clearHeldGameInputs,
    ownership: (open) => { controllerHudFocused = open; enterHeld = false; syncGameInputFocus(); },
    resume: (restoreLock) => {
      requiredElement("scene").focus({ preventScroll: true });
      if (restoreLock) globalThis.weaverResumeGameInput();
    },
    audioAction: () => {
      if (theaterPlayer.state.audioEnabled) theaterPlayer.setMuted(true);
      else if (theaterPlayer.state.soundPending) {
        syncControllerHud();
        return;
      } else if (theaterPlayer.state.failed || theaterSchedule?.state.failed) {
        theaterPlayer.setMuted(true);
        theaterSchedule?.retry();
      } else if (theaterPlayer.state.ready && !theaterSchedule?.state.loading) theaterSchedule?.play(true);
      syncControllerHud();
    },
    subtitlesAction: () => subtitles.toggle(),
    fullscreenAction: () => fullscreenControl.toggle(),
  });
  syncControllerHud();
}

function syncGameInputFocus(): void {
  canvasInteractions?.reset();
  if (wasmReady) wasm.set_host_chat_focus(hostChatFocused || theaterControlsFocused || theaterStartupFocused || controllerHudFocused);
}

function requiredElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) throw new Error(`missing required element #${id}`);
  return element as T;
}

function safeDiagnostic(message: string): string {
  const token = tokenInput.value;
  const redacted = token.length === 0 ? message : message.replaceAll(token, "[redacted]");
  return redacted.replace(/[0-9a-fA-F]{64}/g, "[redacted]").slice(0, 768);
}

function errorMessage(error: unknown): string {
  const message = error instanceof Error
    ? error.message
    : typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error);
  return safeDiagnostic(message);
}

function connectionErrorMessage(error: unknown): string {
  const message = errorMessage(error);
  if (/opening handshake failed/i.test(message)) {
    return (
      `WebTransport opening handshake failed before Woven authentication. ` +
      `Verify UDP reachability, TLS, endpoint path, and that the server allows the exact browser ` +
      `origin ${window.location.origin}.`
    );
  }
  if (/admission operation rejected/i.test(message)) {
    const namespaceId = requiredElement<HTMLInputElement>("woven-namespace").value;
    const sessionId = requiredElement<HTMLInputElement>("woven-session").value;
    return (
      `Woven authenticated the credential but rejected admission to namespace ${namespaceId}, ` +
      `session ${sessionId}. Use the numeric scope IDs and credential from the same current ` +
      `product generation.`
    );
  }
  return message;
}

function positiveId(id: string): bigint {
  const input = requiredElement<HTMLInputElement>(id);
  const value = BigInt(input.value);
  if (value <= 0n || value > MAX_U64) {
    throw new Error(`${input.name} must be an integer in 1..2^64-1`);
  }
  return value;
}

function certificateHash(): ArrayBuffer | null {
  const value = requiredElement<HTMLInputElement>("woven-certificate-hash").value
    .trim()
    .replaceAll(":", "");
  if (value.length === 0) return null;
  if (!/^[0-9a-fA-F]{64}$/.test(value)) {
    throw new Error("certificate hash must be exactly 32 SHA-256 bytes in hexadecimal");
  }
  const buffer = new ArrayBuffer(32);
  const bytes = new Uint8Array(buffer);
  for (const [index, byte] of (value.match(/../g) ?? []).entries()) {
    bytes[index] = Number.parseInt(byte, 16);
  }
  return buffer;
}

function readScope(): Scope {
  const spaceId = positiveId("woven-space");
  if (spaceId <= 2n) {
    throw new Error("Spatial Space must be a preconfigured spatial subspace with ID 3 or greater");
  }
  return {
    namespaceId: positiveId("woven-namespace"),
    sessionId: positiveId("woven-session"),
    spaceId,
    spaceEpoch: positiveId("woven-epoch"),
    channelId: 1n,
  };
}

function webTransportAvailable(): boolean {
  return window.isSecureContext && typeof globalThis.WebTransport === "function";
}

function updateControls(state: ConnectionState): void {
  networkState = state;
  const label = lifecycle.failed || !webTransportAvailable() ? "Unavailable" :
    state === "online" ? "CONNECTED" : state === "connecting" ? "Connecting" :
    hasConnected ? "Disconnected" : "Offline";
  // Only mutate the live region on transitions, not on repeated readiness updates.
  if (connectionLabel.textContent !== label) {
    connectionLabel.textContent = label;
    connectionBadge.dataset.state = label.toLowerCase();
  }
  const canJoin = lifecycle.canJoin && window.navigator.onLine !== false && webTransportAvailable() && !connectionAttemptActive &&
    publishRateConfigurationError === null;
  connectButton.disabled = state !== "offline" || !canJoin;
  retryButton.disabled = state !== "offline" || !canJoin;
  retryButton.textContent = retryMode === "visitor" ? "Join / retry room" : "Connect / retry";
  disconnectButton.disabled = state === "offline";
}

function setNetworkStatus(message: string, failed = false): void {
  networkStatus.textContent = safeDiagnostic(message);
  networkStatus.classList.toggle("failed", failed);
}

function rememberDisplayName(value: string): void {
  if (userProfile === null || userProfile.displayName === value) return;
  userProfile = { ...userProfile, displayName: value };
  saveUserProfile(profileStorage, userProfile);
}

function applyDisplayName(): void {
  const value = displayNameInput.value.trim();
  if (!wasmReady || lifecycle.failed) return;
  const valid = wasm.set_display_name(value);
  displayNameInput.setCustomValidity(
    valid ? "" : "Use 1–24 visible characters without line breaks or control characters.",
  );
  if (valid) {
    displayNameInput.value = value;
    rememberDisplayName(value);
  }
}

// Chat's /name command and the settings field share the same local profile.
globalThis.weaverDisplayNameChanged = (value: string): void => {
  displayNameInput.value = value;
  displayNameInput.setCustomValidity("");
  rememberDisplayName(value);
};

// A classic head script can install the actual callback before this module loads.
// Only Rust emits accepted messages; the shell never forwards raw payloads or diagnostics.
if (globalThis.weaverChatMessage === undefined) globalThis.weaverChatMessage = () => {};

function npcAvailable(): boolean {
  return wasmReady && lifecycle.canJoin && !npcDisposed && document.visibilityState === "visible";
}

function syncNpcRuntime(): void {
  if (!wasmReady) return;
  const active = npcAvailable();
  try {
    if (lifecycle.initialized && npcSchedule && !npcDisposed) {
      const key = `${npcSchedule.anchorUnixSeconds}:${npcSchedule.cycleSeconds}`;
      if (key !== npcCycleKey && wasm.set_npc_show_cycle(npcSchedule.anchorUnixSeconds, npcSchedule.cycleSeconds)) {
        npcCycleKey = key;
      }
    }
    if (active !== npcActive) {
      wasm.set_npc_interaction_active(active);
      npcActive = active;
    }
  } catch { /* A mismatched export leaves the optional local capability unavailable. */ }
}

function publishNpcInteraction(snapshot: NpcInteractionSnapshot | null): void {
  try { globalThis.weaverNpcInteraction?.(snapshot); } catch { /* The host cannot break simulation. */ }
}

// Rust emits JSON after its collided player/NPC update. Preserve the host's callback.
// No network state, DOM location, camera estimate, or chat event supplies proximity.
globalThis.weaverNpcInteractionFrame = (encoded: string): void => {
  if (!npcAvailable() || typeof encoded !== "string" || encoded.length > 4096) return;
  try { publishNpcInteraction(JSON.parse(encoded) as NpcInteractionSnapshot | null); }
  catch { publishNpcInteraction(null); }
};

globalThis.weaverGetNpcInteractionContext = (): string | null => {
  if (!npcAvailable()) return null;
  try {
    const encoded = wasm.get_npc_interaction_context();
    return typeof encoded === "string" && encoded.length <= 4096 ? encoded : null;
  } catch { return null; }
};

globalThis.weaverSubmitNpcQuestion = (npcId: string, requestId: string, text: string): boolean => {
  if (!npcAvailable() || typeof npcId !== "string" || typeof requestId !== "string" ||
    typeof text !== "string" || !wellFormedChatText(text)) return false;
  try { return wasm.submit_npc_question(npcId, requestId, text) === true; }
  catch { return false; }
};

globalThis.weaverPresentNpcResponse = (npcId: string, requestId: string, answer: string, gesture: string): boolean => {
  if (!npcAvailable() || typeof npcId !== "string" || typeof requestId !== "string" ||
    typeof answer !== "string" || typeof gesture !== "string" || !wellFormedChatText(answer, 24 * 1024)) return false;
  try { return wasm.present_npc_response(npcId, requestId, answer, gesture) === true; }
  catch { return false; }
};

globalThis.weaverClearNpcDialogue = (): void => {
  if (!wasmReady) return;
  try { wasm.clear_npc_dialogue(); } catch { /* Already gone or unmatched bindings. */ }
};

globalThis.weaverGetNpcDefinitions = (): ScriptedNpcDefinition[] | null => {
  if (!wasmReady || npcDisposed || lifecycle.failed) return null;
  try {
    const encoded = wasm.get_npc_definitions();
    if (typeof encoded !== "string" || encoded.length > 32 * 1024) return null;
    const definitions: unknown = JSON.parse(encoded);
    return Array.isArray(definitions) && definitions.length <= 11 ? definitions as ScriptedNpcDefinition[] : null;
  } catch { return null; }
};

function requestChatOpen(): boolean {
  try {
    const callback = globalThis.weaverRequestChatOpen ?? globalThis.weaverHostRequestChat;
    return callback?.() === true;
  } catch { return false; }
}

globalThis.weaverChatSendCapacity = (): number => {
  if (!wasmReady || !lifecycle.canJoin || networkState !== "online" || connection === null) return 0;
  return Math.max(0, MAX_PENDING_PUBLISHES - connection.publisher.pending.length);
};

// JS strings can contain unpaired surrogates, which WASM's UTF-8 conversion would replace.
function wellFormedChatText(text: string, maxUnits = 1_024): boolean {
  if (text.length > maxUnits) return false;
  for (let i = 0; i < text.length; i += 1) {
    const unit = text.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = text.charCodeAt(++i);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}

globalThis.weaverSendChat = (text: string): boolean => {
  if (typeof text !== "string" || !wellFormedChatText(text) || globalThis.weaverChatSendCapacity() === 0) return false;
  return wasm.send_host_chat(text);
};

function clearHeldGameInputs(): void {
  canvasInteractions?.reset();
  const canvas = document.getElementById("scene");
  // Winit tracks physical keys. Deliver releases before blocking focused input, and
  // blur the canvas through its supported focus-loss path (also clears touch/drag).
  for (const [code, key] of heldGameKeys) {
    canvas?.dispatchEvent(new KeyboardEvent("keyup", { code, key, bubbles: true }));
  }
  heldGameKeys.clear();
  if (document.activeElement === canvas) canvas?.blur();
}

globalThis.weaverSetChatFocus = (active: boolean): boolean => {
  if (typeof active !== "boolean" || !wasmReady || lifecycle.failed || lifecycle.suspended) return false;
  clearHeldGameInputs();
  hostChatFocused = active;
  if (active) controllerHud?.close(false);
  if (active && document.pointerLockElement && typeof document.exitPointerLock === "function") {
    document.exitPointerLock();
  }
  syncGameInputFocus();
  return true;
};

globalThis.weaverResumeGameInput = (): boolean => {
  if (!wasmReady || !lifecycle.canJoin || hostChatFocused || theaterControlsFocused || theaterStartupFocused || controllerHudFocused) return false;
  const canvas = document.getElementById("scene");
  if (canvas === null) return false;
  // Called synchronously from the user's accepted send, never from lifecycle cleanup.
  canvas.focus({ preventScroll: true });
  if (!window.matchMedia?.("(pointer: coarse)").matches &&
      typeof canvas.requestPointerLock === "function" && document.pointerLockElement !== canvas) {
    try {
      const pending = canvas.requestPointerLock();
      // Older browsers return void; newer implementations reject a Promise on denial.
      if (pending && typeof pending.catch === "function") pending.catch(() => {});
    } catch {
      // User/browser policy can still require a click to capture.
    }
  }
  return true;
};

function editableElement(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
}

function editableTarget(event: Event): boolean {
  return event.composedPath().some(editableElement) || editableElement(document.activeElement);
}

window.addEventListener("keydown", (event) => {
  if (controllerHud?.contains(event.target)) return;
  if (controllerHudFocused) {
    event.preventDefault();
    event.stopImmediatePropagation();
    return;
  }
  if (event.composedPath().includes(theaterPanel) || event.composedPath().includes(subtitleControls)) return;
  if (editableTarget(event)) return;
  if (hostChatFocused || theaterControlsFocused) {
    event.preventDefault();
    event.stopImmediatePropagation();
    return;
  }
  if (event.code === "KeyE" && npcAvailable() && !event.isComposing &&
      globalThis.weaverGetNpcInteractionContext() !== null) {
    if (!event.repeat) requestChatOpen();
    event.preventDefault();
    event.stopImmediatePropagation();
    return;
  }
  if (event.key === "Enter" && document.documentElement.dataset.weaverPresentation === "minimal" &&
      wasmReady && !lifecycle.failed && !lifecycle.suspended && !event.isComposing) {
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.repeat || enterHeld) return;
    enterHeld = true;
    // Also works for physical mobile keyboards; Flutter owns touchscreen opening.
    requestChatOpen();
    return;
  }
  if (movementCodes.has(event.code)) {
    // A held key can keep repeating after blur/composer close. Require a fresh
    // physical press rather than letting a repeat recreate cleared Winit state.
    if (event.repeat && !heldGameKeys.has(event.code)) {
      event.preventDefault();
      event.stopImmediatePropagation();
      return;
    }
    if (!event.repeat) heldGameKeys.set(event.code, event.key);
  }
}, true);
window.addEventListener("keyup", (event) => {
  heldGameKeys.delete(event.code);
  if (event.key === "Enter") enterHeld = false;
}, true);
for (const type of ["beforeinput", "input", "compositionstart", "compositionupdate", "compositionend"]) {
  window.addEventListener(type, (event) => {
    if (!hostChatFocused || editableTarget(event)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  }, true);
}
window.addEventListener("blur", () => {
  enterHeld = false;
  clearHeldGameInputs();
  if (wasmReady && (hostChatFocused || document.documentElement.dataset.weaverPresentation === "minimal")) {
    syncGameInputFocus();
  }
});

function setTokenLabel(): void {
  const managed = authSelect.value === "bearer";
  tokenInput.placeholder = managed ? "Host-provided Bearer token" : "dev-token";
  if (!managed && tokenInput.value.length === 0) tokenInput.value = "dev-token";
}

async function loadLocalDevelopmentCredential(signal: AbortSignal): Promise<string | null> {
  if (!LOCAL_DEVELOPMENT_ORIGINS.has(window.location.origin)) return null;
  const text = await fetchBoundedText("./woven.local-token", {
    signal,
    maxBytes: 4096,
    label: "Local Woven credential",
    allowNotFound: true,
  });
  if (text === null) return null;
  const token = text.trim();
  if (token.length === 0 || token.length > 4096 || /\s/.test(token)) {
    throw new Error("local Woven credential must contain 1–4096 non-whitespace characters");
  }
  return token;
}

async function boundedOperation<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_resolve, reject) => {
    timeout = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs);
  });
  try {
    return await Promise.race([promise, deadline]);
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
  }
}

function isCurrentAttempt(generation: number, abort: AbortController): boolean {
  return generation === connectionGeneration && !abort.signal.aborted && lifecycle.canJoin;
}

async function resolveConnectionInput(mode: JoinMode, signal: AbortSignal): Promise<ConnectionInput | null> {
  if (mode === "visitor") {
    if (lobbyConfigurationError !== null) throw new Error(lobbyConfigurationError);
    if (lobbyConfig === null) throw new Error("Anonymous visitor join is not configured");
    setNetworkStatus("Requesting anonymous room admission…");
    const lobby = await fetchLobbyBootstrap(lobbyConfig, signal);
    return {
      url: lobby.url,
      token: lobby.token,
      scope: { namespaceId: lobby.namespaceId, sessionId: lobby.sessionId, spaceId: lobby.spaceId,
        spaceEpoch: lobby.spaceEpoch, channelId: 1n },
      managed: true,
      hash: null,
    };
  }
  if (mode === "local") {
    setNetworkStatus("Loading local development credential…");
    const token = await loadLocalDevelopmentCredential(signal);
    if (signal.aborted || token === null) return null;
    authSelect.value = "bearer";
    tokenInput.value = token;
    setTokenLabel();
  }
  const url = requiredElement<HTMLInputElement>("woven-url").value.trim();
  const token = tokenInput.value;
  if (url.length === 0 || url.length > 512 || url.includes("@")) {
    throw new Error("Woven endpoint must be a bounded URL without credentials");
  }
  if (token.length === 0 || token.length > 4096) {
    throw new Error("Woven credential must contain 1–4096 characters");
  }
  return { url, token, scope: readScope(), managed: authSelect.value === "bearer", hash: certificateHash() };
}

async function establishConnection(mode: JoinMode): Promise<void> {
  if (connection !== null || connectingClient !== null || connectionAttemptActive) return;
  if (!lifecycle.canJoin) {
    setNetworkStatus(lifecycle.failed ? "Scene unavailable · reload the page to retry." :
      "Joining requires a ready scene in a visible tab.", lifecycle.failed);
    return;
  }
  if (!webTransportAvailable()) {
    showWebTransportUnavailable();
    return;
  }
  if (window.navigator.onLine === false) {
    updateControls("offline");
    return;
  }
  if (publishRateConfigurationError !== null) {
    setNetworkStatus(publishRateConfigurationError, true);
    return;
  }
  lifecycle.cancelAutoJoin();
  wasm.reset_host_chat();
  retryMode = mode;
  connectionAttemptActive = true;
  const generation = ++connectionGeneration;
  const abort = new AbortController();
  setupAbort = abort;
  updateControls("connecting");
  try {
    const input = await resolveConnectionInput(mode, abort.signal);
    if (!isCurrentAttempt(generation, abort)) return;
    if (input === null) {
      retryMode = "operator";
      setNetworkStatus("Offline · no local credential found. Open Settings to connect explicitly.");
      return;
    }
    await establishConnectionAttempt(input, generation, abort);
  } catch (error) {
    if (isCurrentAttempt(generation, abort)) {
      disconnect(`Unable to connect: ${connectionErrorMessage(error)} · Retry manually.`, true);
    }
  } finally {
    if (setupAbort === abort) setupAbort = null;
    connectionAttemptActive = false;
    updateControls(connection === null ? "offline" : "online");
  }
}

async function establishConnectionAttempt(input: ConnectionInput, generation: number, abort: AbortController): Promise<void> {
  const { url, token, scope, managed, hash } = input;
  setNetworkStatus("Connecting to Woven…");
  const client = await WovenClient.connect({
    url,
    token,
    authenticationScheme: managed
      ? AuthenticationScheme.Bearer
      : AuthenticationScheme.Development,
    webTransportOptions: {
      requireUnreliable: true,
      ...(hash === null
        ? {}
        : { serverCertificateHashes: [{ algorithm: "sha-256", value: hash }] }),
    },
    datagramMaxAgeMs: 250,
    maxFrameBytes: 1_048_576,
    maxPayloadBytes: 65_536,
    connectTimeoutMs: SETUP_OPERATION_TIMEOUT_MS,
  });
  if (!isCurrentAttempt(generation, abort)) {
    client.close(0, "connection cancelled");
    return;
  }
  if (!client.supportsPositionedState()) {
    client.close(1, "positioned state unavailable");
    throw new Error("Woven server did not negotiate positioned entity state");
  }
  connectingClient = client;

  try {
    if (managed) {
      setNetworkStatus("Waiting for Woven admission…");
      // Admission idempotency needs a fresh request ID, never the persisted guest UUID.
      const outcome = await client.admitWithCancellation(
        scope.namespaceId,
        scope.sessionId,
        wasm.GenerateUserID(),
        60_000,
        abort.signal,
      );
      const admitted =
        (outcome.kind === "admission" && outcome.result.status === AdmissionStatus.Admitted) ||
        (outcome.kind === "queue" && outcome.update.state === QueueState.Admitted);
      if (!admitted) throw new Error("Woven admission did not admit this visitor");
    } else {
      await boundedOperation(
        client.joinSession(scope.namespaceId, scope.sessionId),
        SETUP_OPERATION_TIMEOUT_MS,
        "Woven session join",
      );
    }

    if (!isCurrentAttempt(generation, abort)) return;
    await boundedOperation(
      client.subscribeSpace(
        scope.namespaceId,
        scope.sessionId,
        scope.spaceId,
        scope.spaceEpoch,
        scope.channelId,
      ),
      SETUP_OPERATION_TIMEOUT_MS,
      "Woven room subscription",
    );
    if (!isCurrentAttempt(generation, abort)) return;
    setNetworkStatus("Subscribing to the shared room…");
    const entityId = await receiveAssignedEntity(client, abort.signal);
    if (!isCurrentAttempt(generation, abort)) {
      client.close(0, "connection cancelled");
      return;
    }

    const active: Connection = {
      client,
      entityId,
      scope,
      cancelConnectionLog: null,
      publisher: { pending: [] },
      posePublisher: { pending: null },
      scheduler: new OutboundScheduler({
        rateHz: publishRateHz,
        isCurrent: () => connection === active && connectionGeneration === generation && lifecycle.canJoin,
        hasPending: () => active.publisher.pending.length > 0 || active.posePublisher.pending !== null,
        sendNext: () => sendNextOutbound(active),
        onError: (error) => disconnect(errorMessage(error), true),
      }),
    };
    connectingClient = null;
    setupAbort = null;
    connection = active;
    if (!wasm.realtime_connected(entityId)) {
      disconnect("WASM realtime inbox overflow during connection", true);
      return;
    }
    hasConnected = true;
    updateControls("online");
    setNetworkStatus(`Online as Woven entity ${entityId}`);
    active.cancelConnectionLog = scheduleConnectionLog(
      () => connection === active && connectionGeneration === generation,
      (message) => {
        if (active.publisher.pending.length >= MAX_PENDING_PUBLISHES) {
          console.warn("First-Person Lab delayed connection log skipped: publish queue is full.");
          return;
        }
        // Optional logs consume the same paced send slots as profiles/chat and poses.
        active.publisher.pending.push({ kind: "log", message });
        active.scheduler.kick();
      },
    );
    void receiveLoop(active, generation);
    void receivePoseLoop(active, generation);
  } catch (error) {
    if (connectingClient === client) connectingClient = null;
    if (setupAbort === abort) setupAbort = null;
    client.close(1, "connection setup failed");
    throw error;
  }
}

async function receiveAssignedEntity(client: WovenClient, signal: AbortSignal): Promise<bigint> {
  const deadline = performance.now() + SETUP_OPERATION_TIMEOUT_MS;
  let subscriptionAccepted = false;
  let received = 0;
  while (received < 64 && performance.now() < deadline) {
    if (signal.aborted) throw new Error("Woven connection cancelled");
    const remaining = Math.max(1, Math.ceil(deadline - performance.now()));
    const envelope = await client.recvTimeout(Math.min(1_000, remaining));
    if (signal.aborted) throw new Error("Woven connection cancelled");
    if (envelope === null) continue;
    received += 1;
    if (envelope.messageKind === MessageKind.ProtocolError) {
      throw new Error("Woven rejected the room subscription");
    }
    if (envelope.messageKind === MessageKind.SubscriptionRejected) {
      throw new Error("Woven rejected the requested room/channel");
    }
    if (envelope.messageKind === MessageKind.SubscriptionAccepted) {
      subscriptionAccepted = true;
      continue;
    }
    if (
      subscriptionAccepted &&
      envelope.messageKind === MessageKind.EntityEntered &&
      envelope.entityId !== null
    ) {
      return envelope.entityId;
    }
  }
  throw new Error("Woven did not assign an entity within the bounded startup exchange");
}

async function receiveLoop(active: Connection, generation: number): Promise<void> {
  try {
    await receiveWhileCurrent(
      () => connection === active && generation === connectionGeneration,
      () => active.client.recv(),
      (envelope) => {
        if (envelope === null) throw new Error("Woven receiver closed");
        processEnvelope(envelope, active);
      },
    );
  } catch (error) {
    if (connection === active) disconnect(`Woven receive failed: ${errorMessage(error)}`, true);
  }
}

async function receivePoseLoop(active: Connection, generation: number): Promise<void> {
  try {
    while (connection === active && generation === connectionGeneration) {
      const envelope = await active.client.recvDatagram();
      if (connection !== active || generation !== connectionGeneration) return;
      if (envelope === null) {
        throw new Error("Woven datagram receiver closed");
      }
      if (
        !matchesScope(envelope, active.scope) ||
        envelope.messageKind !== MessageKind.EntityState ||
        envelope.deliveryClass !== DeliveryClass.UnreliableSequenced ||
        envelope.channelId !== POSE_CHANNEL_ID ||
        envelope.payloadTypeId !== 1n ||
        envelope.entityId === null ||
        envelope.entityId === active.entityId ||
        !validPosePayload(envelope.payload)
      ) {
        continue;
      }
      if (!wasm.realtime_unreliable_payload(envelope.entityId, envelope.senderSequence, envelope.payload)) {
        throw new Error("WASM realtime inbox overflow while receiving a visitor pose");
      }
    }
  } catch (error) {
    if (connection === active) disconnect(`Woven pose receive failed: ${errorMessage(error)}`, true);
  }
}

function matchesScope(envelope: DecodedEnvelope, scope: Scope): boolean {
  return (
    envelope.namespaceId === scope.namespaceId &&
    envelope.sessionId === scope.sessionId &&
    envelope.spaceId === scope.spaceId &&
    envelope.spaceEpoch === scope.spaceEpoch
  );
}

function processEnvelope(envelope: DecodedEnvelope, active: Connection): void {
  if (envelope.messageKind === MessageKind.ProtocolError) {
    const control = envelope.control as { relatedMessageKind?: () => MessageKind } | null;
    if (
      typeof control?.relatedMessageKind === "function" &&
      control.relatedMessageKind() === MessageKind.ClientLog
    ) {
      console.warn("Woven rejected First-Person Lab's delayed connection log.");
      return;
    }
    throw new Error("Woven reported a protocol error");
  }
  if (!matchesScope(envelope, active.scope)) return;
  if (envelope.messageKind === MessageKind.EntityEntered && envelope.entityId !== null) {
    if (!wasm.realtime_entity_entered(envelope.entityId)) {
      throw new Error("WASM realtime inbox overflow while adding a visitor");
    }
    return;
  }
  if (envelope.messageKind === MessageKind.EntityLeft && envelope.entityId !== null) {
    if (!wasm.realtime_entity_left(envelope.entityId)) {
      throw new Error("WASM realtime inbox overflow while removing a visitor");
    }
    return;
  }
  if (
    envelope.messageKind === MessageKind.ReliableEvent &&
    envelope.channelId === active.scope.channelId &&
    envelope.payloadTypeId === 1n &&
    envelope.entityId !== null &&
    envelope.entityId !== active.entityId &&
    envelope.payload !== null
  ) {
    const payload = decodeApplicationPayload(envelope.payload);
    if (payload === null) return;
    if (!wasm.realtime_payload(envelope.entityId, envelope.senderSequence, payload)) {
      throw new Error("WASM realtime inbox overflow while receiving a room payload");
    }
  }
}

function disconnect(reason = "Disconnected from Woven", failed = false): void {
  reason = safeDiagnostic(reason);
  const active = connection;
  connection = null;
  if (active !== null) {
    active.cancelConnectionLog?.();
    active.cancelConnectionLog = null;
    active.scheduler.stop();
    active.publisher.pending.length = 0;
    active.posePublisher.pending = null;
  }
  active?.client.close(0, "visitor disconnected");
  setupAbort?.abort();
  setupAbort = null;
  connectingClient?.close(0, "visitor cancelled connection");
  connectingClient = null;
  connectionGeneration += 1;
  if (wasmReady) {
    wasm.reset_host_chat();
    if (active !== null) wasm.realtime_disconnected(reason);
  }
  enterHeld = false;
  clearHeldGameInputs();
  updateControls("offline");
  setNetworkStatus(reason, failed);
}

async function sendNextOutbound(active: Connection): Promise<void> {
  // Keep the ordered stream FIFO (including latest replacement) ahead of disposable poses.
  const publish = active.publisher.pending.shift();
  try {
    if (publish !== undefined) {
      if (publish.kind === "log") {
        try {
          await active.client.logger.info(publish.message);
        } catch {
          // Optional logging must not disconnect a healthy room or expose transport diagnostics.
          console.warn("First-Person Lab delayed connection log was not sent.");
        }
        return;
      }
      await active.client.publishEvent(
        active.scope.namespaceId,
        active.scope.sessionId,
        active.scope.spaceId,
        active.scope.spaceEpoch,
        active.scope.channelId,
        active.entityId,
        publish.sequence,
        1n,
        encoder.encode(publish.payload),
      );
      return;
    }
    // Select the newest pose only when its send slot is ready, not when the timer is armed.
    const pose = active.posePublisher.pending;
    if (pose === null) return;
    active.posePublisher.pending = null;
    await active.client.publishUnreliablePositionedState(
      active.scope.namespaceId,
      active.scope.sessionId,
      active.scope.spaceId,
      active.scope.spaceEpoch,
      POSE_CHANNEL_ID,
      active.entityId,
      pose.sequence,
      1n,
      pose.position,
      pose.payload,
    );
  } catch (error) {
    const label = publish === undefined ? "Woven pose publish failed" : "Woven publish failed";
    throw new Error(`${label}: ${errorMessage(error)}`);
  }
}

globalThis.weaverRealtimePublishUnreliable = (): void => {
  throw new Error("First-Person Lab rejects unpositioned pose publishing");
};

// Copy the WASM-backed view before retaining it beyond this synchronous bridge call.
globalThis.weaverRealtimePublishPositionedUnreliable = (
  sequence: bigint,
  x: number,
  y: number,
  z: number,
  payload: Uint8Array,
): void => {
  const active = connection;
  if (active === null) return;
  if (!validPosePayload(payload)) {
    throw new Error(`pose payload must be version 1 (25 bytes) or version 2 (26 bytes, seat code 0..${SEAT_COUNT})`);
  }
  if (![x, y, z].every(Number.isFinite)) {
    throw new Error("pose routing position must contain finite coordinates");
  }
  active.posePublisher.pending = {
    sequence,
    position: { x, y, z },
    payload: payload.slice(),
  };
  active.scheduler.kick();
};

globalThis.weaverRealtimePublishLatest = (sequence: bigint, payload: string): void => {
  const active = connection;
  if (active === null) return;
  active.publisher.pending = active.publisher.pending.filter((publish) => publish.kind !== "latest");
  if (active.publisher.pending.length === MAX_PENDING_PUBLISHES) return;
  active.publisher.pending.push({ kind: "latest", sequence, payload });
  active.scheduler.kick();
};

globalThis.weaverRealtimePublishReliable = (sequence: bigint, payload: string): void => {
  const active = connection;
  if (active === null) throw new Error("Woven is not connected");
  if (active.publisher.pending.length === MAX_PENDING_PUBLISHES) {
    throw new Error(`reliable publish queue exceeded ${MAX_PENDING_PUBLISHES} pending messages`);
  }
  active.publisher.pending.push({ kind: "reliable", sequence, payload });
  active.scheduler.kick();
};

globalThis.weaverRealtimeFatal = (reason: string): void => {
  disconnect(reason, true);
};

function showWebTransportUnavailable(): void {
  setNetworkStatus("Multiplayer unavailable · WebTransport is required. Use HTTPS (or localhost) in a browser with WebTransport support. The local room is still available.", true);
  updateControls(networkState);
}

function maybeAutoJoin(): void {
  if (publishRateConfigurationError !== null) {
    setNetworkStatus(publishRateConfigurationError, true);
    return;
  }
  if (!lifecycle.consumeAutoJoin()) {
    if (networkState === "offline" && networkStatus.textContent?.startsWith("Offline · waiting for the scene")) {
      setNetworkStatus("Offline · scene ready. Join / retry manually to return to the room.");
    }
    return;
  }
  if (lobbyConfigurationError !== null) {
    setNetworkStatus(`Visitor join configuration error: ${lobbyConfigurationError}`, true);
    return;
  }
  if (retryMode !== "operator") void establishConnection(retryMode);
  else setNetworkStatus("Offline · open Settings to connect explicitly. Chat is desktop-first.");
}

function syncSceneReadiness(): void {
  if (lifecycle.initialized && lifecycle.rendererReady && !lifecycle.failed) {
    clearTimeout(bootTimer);
    reportLoadProgress("scene");
    reloadLink.hidden = true;
  }
  updateControls(networkState);
  theaterPlayer.setSceneReady(lifecycle.initialized && lifecycle.rendererReady && !lifecycle.failed);
  theaterStartup?.setSceneReady(lifecycle.initialized && lifecycle.rendererReady && !lifecycle.failed);
  queueTheaterStart();
  lobbyLight.disabled = auditoriumLight.disabled = !lifecycle.canJoin || theaterPlayer.state.disposed;
  fullscreenControl.publish();
  syncNpcRuntime();
  if (!lifecycle.canJoin) return;
  if (!webTransportAvailable()) {
    lifecycle.cancelAutoJoin();
    showWebTransportUnavailable();
    return;
  }
  maybeAutoJoin();
}

// Rust owns renderer success/failure. init() completion alone never enables joining.
globalThis.weaverSceneReady = (): void => {
  lifecycle.markReady();
  syncSceneReadiness();
};
globalThis.weaverSceneFatal = (message: string): void => {
  loadFailed = true;
  bootAbort.abort();
  lifecycle.fail();
  globalThis.weaverDisposeTheaterVideo();
  clearTimeout(bootTimer);
  reloadLink.hidden = false;
  const reason = `Scene unavailable: ${safeDiagnostic(message)} · Reload the page to retry.`;
  const status = requiredElement<HTMLElement>("status");
  status.textContent = reason;
  status.classList.add("failed");
  disconnect(reason, true);
};

function suspendPage(): void {
  clearHeldGameInputs();
  controllerHud?.close(false);
  lifecycle.suspend();
  cancelTheaterStart();
  theaterStartup?.setVisible(false);
  syncNpcRuntime();
  globalThis.weaverClearNpcDialogue();
  publishNpcInteraction(null);
  theaterSchedule?.setVisible(false);
  theaterPlayer.setVisible(false);
  lobbyLight.disabled = auditoriumLight.disabled = true;
  disconnect(lifecycle.failed ? "Scene unavailable · reload the page to retry." :
    "Offline · disconnected while this page is hidden. Return and join again manually.", lifecycle.failed);
}

function restorePage(): void {
  if (document.visibilityState !== "visible") return;
  canvasInteractions?.reset();
  lifecycle.resume();
  syncNpcRuntime();
  theaterPlayer.setVisible(true);
  theaterSchedule?.setVisible(true);
  theaterStartup?.setVisible(true);
  queueTheaterStart();
  lobbyLight.disabled = auditoriumLight.disabled = !lifecycle.canJoin || theaterPlayer.state.disposed;
  updateControls(networkState);
  if (lifecycle.failed) return;
  if (!lifecycle.canJoin) {
    setNetworkStatus("Offline · waiting for the scene. Join manually once ready.");
  } else if (!webTransportAvailable()) {
    showWebTransportUnavailable();
  } else if (publishRateConfigurationError !== null) {
    setNetworkStatus(publishRateConfigurationError, true);
  } else {
    setNetworkStatus("Offline · page restored. Join / retry manually to return to the room.");
  }
}

authSelect.addEventListener("change", setTokenLabel);
displayNameInput.addEventListener("change", applyDisplayName);
form.addEventListener("submit", (event) => {
  event.preventDefault();
  lifecycle.cancelAutoJoin();
  void establishConnection("operator");
});
retryButton.addEventListener("click", () => {
  lifecycle.cancelAutoJoin();
  if (retryMode === "operator") {
    if (!form.checkValidity()) settings.open = true;
    form.requestSubmit();
  } else {
    void establishConnection(retryMode);
  }
});
disconnectButton.addEventListener("click", () => {
  lifecycle.cancelAutoJoin();
  disconnect("Offline · disconnected. Join / retry manually when ready.");
});
window.addEventListener("offline", () => {
  if (connection !== null || connectionAttemptActive) {
    disconnect("Offline · network unavailable. Join / retry manually when ready.");
  } else updateControls(networkState);
});
// Connectivity is only a hint: Host/manual retry still owns reconnect admission.
window.addEventListener("online", () => updateControls(networkState));
window.addEventListener("pagehide", suspendPage);
window.addEventListener("pageshow", (event) => {
  if (event.persisted) restorePage();
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") restorePage();
  else suspendPage();
});
setTokenLabel();

const bootstrapPath = document.querySelector<HTMLMetaElement>('meta[name="weaver-lobby-bootstrap"]')?.content ?? "";
if (bootstrapPath !== "" && bootstrapPath !== "off") retryMode = "visitor";
try {
  lobbyConfig = readLobbyConfig(bootstrapPath,
    document.querySelector<HTMLMetaElement>('meta[name="weaver-lobby-target"]')?.content ?? "",
    window.location.origin);
} catch (error) {
  lobbyConfigurationError = errorMessage(error);
  setNetworkStatus(`Visitor join configuration error: ${lobbyConfigurationError}`, true);
}
try {
  publishRateHz = parsePublishRateHz(document.documentElement.dataset.weaverPublishRateHz);
} catch {
  publishRateConfigurationError = PUBLISH_RATE_CONFIGURATION_ERROR;
  setNetworkStatus(publishRateConfigurationError, true);
}
updateControls("offline");
if (!webTransportAvailable()) showWebTransportUnavailable();

reportLoadProgress("shell");
try {
  wasm = await import("./pkg/first_person_lab.js") as FirstPersonWasm;
  reportLoadProgress("module");
  const response = await fetchWasmWithProgress(new URL("./pkg/first_person_lab_bg.wasm", import.meta.url), {
    signal: bootAbort.signal,
    onDownload: (loadedBytes, totalBytes) => notifyLoading({ event: "download", loadedBytes, totalBytes }),
    onActivity: bootActivity,
    onComplete: reportInitializing,
  });
  await wasm.default({ module_or_path: response });
  if (lifecycle.failed) throw new Error("Scene startup already failed");
  reportInitializing();
  reportLoadProgress("wasm");
  // Presentation must be applied before starting the scene, not on its first rendered frame.
  wasm.set_minimal_ui(document.documentElement.dataset.weaverPresentation === "minimal");
  userProfile = loadUserProfile(profileStorage, {
    generateGuestName: () => wasm.GenerateGuestName(),
    isLegacyUserName: (name) => wasm.IsLegacyUserName(name),
    generateUserId: wasm.GenerateUserID,
  });
  displayNameInput.value = userProfile.displayName;
  wasmReady = true;
  if (hostChatFocused || theaterControlsFocused || theaterStartupFocused) syncGameInputFocus();
  applyDisplayName();
  reportLoadProgress("profile");
  wasm.start();
  lifecycle.initialized = true;
  reportLoadProgress("started");
  syncSceneReadiness();
} catch (error) {
  if (!loadFailed) globalThis.weaverSceneFatal(`Unable to start Weaver: ${errorMessage(error)}`);
}
