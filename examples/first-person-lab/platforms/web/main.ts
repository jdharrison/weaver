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
  publisher: {
    inFlight: boolean;
    pending: PendingPublish[];
  };
  posePublisher: {
    inFlight: boolean;
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
  var weaverSceneReady: () => void;
  var weaverSceneFatal: (message: string) => void;
}

const MAX_U64 = 18_446_744_073_709_551_615n;
const SETUP_OPERATION_TIMEOUT_MS = 10_000;
const MAX_PENDING_PUBLISHES = 32;
const POSE_CHANNEL_ID = 4n;
const POSE_PAYLOAD_BYTES = 25;
const LOCAL_DEVELOPMENT_ORIGINS = new Set([
  "http://127.0.0.1:8000",
  "http://localhost:8000",
]);
const encoder = new TextEncoder();
const lifecycle = new BrowserLifecycle(document.visibilityState === "visible");
let wasm: typeof import("./pkg/first_person_lab.js");
let networkState: ConnectionState = "offline";
let lobbyConfig: LobbyConfig | null = null;
let lobbyConfigurationError: string | null = null;
let retryMode: JoinMode = LOCAL_DEVELOPMENT_ORIGINS.has(window.location.origin) ? "local" : "operator";
let bootTimer: ReturnType<typeof setTimeout> | undefined;
let connection: Connection | null = null;
let connectingClient: WovenClient | null = null;
let setupAbort: AbortController | null = null;
let connectionGeneration = 0;
let connectionAttemptActive = false;
let wasmReady = false;
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
const authSelect = requiredElement<HTMLSelectElement>("woven-auth");
const tokenInput = requiredElement<HTMLInputElement>("woven-token");
const displayNameInput = requiredElement<HTMLInputElement>("display-name");
const retryButton = requiredElement<HTMLButtonElement>("join-network");
const reloadLink = requiredElement<HTMLAnchorElement>("reload-scene");
const settings = requiredElement<HTMLDetailsElement>("multiplayer-settings");

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
  const canJoin = lifecycle.canJoin && webTransportAvailable() && !connectionAttemptActive;
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
  lifecycle.cancelAutoJoin();
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
      publisher: { inFlight: false, pending: [] },
      posePublisher: { inFlight: false, pending: null },
    };
    connectingClient = null;
    setupAbort = null;
    connection = active;
    if (!wasm.realtime_connected(entityId)) {
      disconnect("WASM realtime inbox overflow during connection", true);
      return;
    }
    updateControls("online");
    setNetworkStatus(`Online as Woven entity ${entityId}`);
    active.cancelConnectionLog = scheduleConnectionLog(
      () => connection === active && connectionGeneration === generation,
      (message) => {
        if (active.publisher.pending.length >= MAX_PENDING_PUBLISHES) {
          console.warn("First-Person Lab delayed connection log skipped: publish queue is full.");
          return;
        }
        // Log writes share the ordered stream with profiles/chat, so serialize them together.
        active.publisher.pending.push({ kind: "log", message });
        void pumpPublisher(active);
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
      (envelope) => processEnvelope(envelope, active),
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
        envelope.payload?.byteLength !== POSE_PAYLOAD_BYTES
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
    active.publisher.pending.length = 0;
    active.posePublisher.pending = null;
  }
  active?.client.close(0, "visitor disconnected");
  setupAbort?.abort();
  setupAbort = null;
  connectingClient?.close(0, "visitor cancelled connection");
  connectingClient = null;
  connectionGeneration += 1;
  if (wasmReady && active !== null) wasm.realtime_disconnected(reason);
  updateControls("offline");
  setNetworkStatus(reason, failed);
}

async function pumpPublisher(active: Connection): Promise<void> {
  if (active.publisher.inFlight) return;
  active.publisher.inFlight = true;
  try {
    while (connection === active && active.publisher.pending.length > 0) {
      const publish = active.publisher.pending.shift();
      if (publish === undefined) break;
      if (publish.kind === "log") {
        try {
          await active.client.logger.info(publish.message);
        } catch {
          // Optional logging must not disconnect a healthy room or expose transport diagnostics.
          console.warn("First-Person Lab delayed connection log was not sent.");
        }
        continue;
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
    }
  } catch (error) {
    if (connection === active) disconnect(`Woven publish failed: ${errorMessage(error)}`, true);
  } finally {
    active.publisher.inFlight = false;
    if (connection === active && active.publisher.pending.length > 0) void pumpPublisher(active);
  }
}

async function pumpPosePublisher(active: Connection): Promise<void> {
  if (active.posePublisher.inFlight) return;
  active.posePublisher.inFlight = true;
  try {
    while (connection === active && active.posePublisher.pending !== null) {
      const publish = active.posePublisher.pending;
      active.posePublisher.pending = null;
      await active.client.publishUnreliablePositionedState(
        active.scope.namespaceId,
        active.scope.sessionId,
        active.scope.spaceId,
        active.scope.spaceEpoch,
        POSE_CHANNEL_ID,
        active.entityId,
        publish.sequence,
        1n,
        publish.position,
        publish.payload,
      );
    }
  } catch (error) {
    if (connection === active) disconnect(`Woven pose publish failed: ${errorMessage(error)}`, true);
  } finally {
    active.posePublisher.inFlight = false;
    if (connection === active && active.posePublisher.pending !== null) void pumpPosePublisher(active);
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
  if (payload.byteLength !== POSE_PAYLOAD_BYTES) {
    throw new Error(`pose payload must be exactly ${POSE_PAYLOAD_BYTES} bytes`);
  }
  if (![x, y, z].every(Number.isFinite)) {
    throw new Error("pose routing position must contain finite coordinates");
  }
  active.posePublisher.pending = {
    sequence,
    position: { x, y, z },
    payload: payload.slice(),
  };
  void pumpPosePublisher(active);
};

globalThis.weaverRealtimePublishLatest = (sequence: bigint, payload: string): void => {
  const active = connection;
  if (active === null) return;
  active.publisher.pending = active.publisher.pending.filter((publish) => publish.kind !== "latest");
  if (active.publisher.pending.length === MAX_PENDING_PUBLISHES) return;
  active.publisher.pending.push({ kind: "latest", sequence, payload });
  void pumpPublisher(active);
};

globalThis.weaverRealtimePublishReliable = (sequence: bigint, payload: string): void => {
  const active = connection;
  if (active === null) throw new Error("Woven is not connected");
  if (active.publisher.pending.length === MAX_PENDING_PUBLISHES) {
    throw new Error(`reliable publish queue exceeded ${MAX_PENDING_PUBLISHES} pending messages`);
  }
  active.publisher.pending.push({ kind: "reliable", sequence, payload });
  void pumpPublisher(active);
};

globalThis.weaverRealtimeFatal = (reason: string): void => {
  disconnect(reason, true);
};

function showWebTransportUnavailable(): void {
  setNetworkStatus("Multiplayer unavailable · WebTransport is required. Use HTTPS (or localhost) in a browser with WebTransport support. The local room is still available.", true);
  updateControls(networkState);
}

function maybeAutoJoin(): void {
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
    reloadLink.hidden = true;
  }
  updateControls(networkState);
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
  lifecycle.fail();
  clearTimeout(bootTimer);
  reloadLink.hidden = false;
  const reason = `Scene unavailable: ${safeDiagnostic(message)} · Reload the page to retry.`;
  const status = requiredElement<HTMLElement>("status");
  status.textContent = reason;
  status.classList.add("failed");
  disconnect(reason, true);
};

function suspendPage(): void {
  lifecycle.suspend();
  disconnect(lifecycle.failed ? "Scene unavailable · reload the page to retry." :
    "Offline · disconnected while this page is hidden. Return and join again manually.", lifecycle.failed);
}

function restorePage(): void {
  if (document.visibilityState !== "visible") return;
  lifecycle.resume();
  updateControls(networkState);
  if (lifecycle.failed) return;
  if (!lifecycle.canJoin) {
    setNetworkStatus("Offline · waiting for the scene. Join manually once ready.");
  } else if (!webTransportAvailable()) {
    showWebTransportUnavailable();
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
updateControls("offline");
if (!webTransportAvailable()) showWebTransportUnavailable();

bootTimer = setTimeout(() => {
  globalThis.weaverSceneFatal("Scene startup timed out. Check browser graphics support and the JS/WASM assets.");
}, 20_000);
try {
  wasm = await import("./pkg/first_person_lab.js");
  await wasm.default();
  userProfile = loadUserProfile(profileStorage, {
    generateUserName: () => wasm.GenerateUserName(),
    generateUserId: wasm.GenerateUserID,
  });
  displayNameInput.value = userProfile.displayName;
  wasmReady = true;
  lifecycle.initialized = true;
  applyDisplayName();
  syncSceneReadiness();
} catch (error) {
  globalThis.weaverSceneFatal(`Unable to start Weaver: ${errorMessage(error)}`);
}
