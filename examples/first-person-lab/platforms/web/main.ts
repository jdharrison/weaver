import {
  AdmissionStatus,
  AuthenticationScheme,
  MessageKind,
  QueueState,
  WovenClient,
  type DecodedEnvelope,
} from "@signalweave/woven-client";
import init, {
  realtime_connected,
  realtime_disconnected,
  realtime_entity_left,
  realtime_payload,
} from "./pkg/first_person_lab.js";

type Scope = {
  namespaceId: bigint;
  sessionId: bigint;
  spaceId: bigint;
  spaceEpoch: bigint;
  channelId: bigint;
};

type PendingPublish = {
  sequence: bigint;
  payload: string;
};

type Connection = {
  client: WovenClient;
  entityId: bigint;
  scope: Scope;
  publisher: {
    inFlight: boolean;
    pending: PendingPublish | null;
  };
};

type ConnectionState = "offline" | "connecting" | "online";

declare global {
  var weaverRealtimePublish: (sequence: bigint, payload: string) => void;
  var weaverRealtimeFatal: (reason: string) => void;
}

const MAX_U64 = 18_446_744_073_709_551_615n;
const SETUP_OPERATION_TIMEOUT_MS = 10_000;
const LOCAL_DEVELOPMENT_ORIGINS = new Set([
  "http://127.0.0.1:8000",
  "http://localhost:8000",
]);
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
let connection: Connection | null = null;
let connectingClient: WovenClient | null = null;
let setupAbort: AbortController | null = null;
let connectionGeneration = 0;
let connectionAttemptActive = false;
let wasmReady = false;

const form = requiredElement<HTMLFormElement>("network-form");
const connectButton = requiredElement<HTMLButtonElement>("connect-network");
const disconnectButton = requiredElement<HTMLButtonElement>("disconnect-network");
const networkStatus = requiredElement<HTMLElement>("network-status");
const authSelect = requiredElement<HTMLSelectElement>("woven-auth");
const tokenInput = requiredElement<HTMLInputElement>("woven-token");

function requiredElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) throw new Error(`missing required element #${id}`);
  return element as T;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
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
  return {
    namespaceId: positiveId("woven-namespace"),
    sessionId: positiveId("woven-session"),
    spaceId: positiveId("woven-space"),
    spaceEpoch: positiveId("woven-epoch"),
    channelId: 1n,
  };
}

function updateControls(state: ConnectionState): void {
  connectButton.disabled = state !== "offline";
  disconnectButton.disabled = state === "offline";
}

function setNetworkStatus(message: string, failed = false): void {
  networkStatus.textContent = message;
  networkStatus.classList.toggle("failed", failed);
}

function setTokenLabel(): void {
  const managed = authSelect.value === "bearer";
  tokenInput.placeholder = managed ? "Host-provided Bearer token" : "dev-token";
  if (!managed && tokenInput.value.length === 0) tokenInput.value = "dev-token";
}

async function loadLocalDevelopmentCredential(): Promise<boolean> {
  if (!LOCAL_DEVELOPMENT_ORIGINS.has(window.location.origin)) return false;

  const response = await fetch("./woven.local-token", {
    cache: "no-store",
    credentials: "omit",
  });
  if (response.status === 404) return false;
  if (!response.ok) {
    throw new Error(`local Woven credential request failed with HTTP ${response.status}`);
  }

  const token = (await response.text()).trim();
  if (token.length === 0 || token.length > 4096 || /\s/.test(token)) {
    throw new Error("local Woven credential must contain 1–4096 non-whitespace characters");
  }
  authSelect.value = "bearer";
  tokenInput.value = token;
  setTokenLabel();
  return true;
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

async function establishConnection(): Promise<void> {
  if (connection !== null || connectingClient !== null || connectionAttemptActive) return;
  connectionAttemptActive = true;
  try {
    await establishConnectionAttempt();
  } finally {
    connectionAttemptActive = false;
    if (connection === null && connectingClient === null) {
      updateControls("offline");
      if (networkStatus.textContent?.startsWith("Cancelling")) {
        setNetworkStatus("Offline · no remote visitors");
      }
    }
  }
}

async function establishConnectionAttempt(): Promise<void> {
  const generation = ++connectionGeneration;
  updateControls("connecting");
  setNetworkStatus("Connecting to Woven…");

  const url = requiredElement<HTMLInputElement>("woven-url").value.trim();
  const token = tokenInput.value;
  if (url.length === 0 || url.length > 512 || url.includes("@")) {
    throw new Error("Woven endpoint must be a bounded URL without credentials");
  }
  if (token.length === 0 || token.length > 4096) {
    throw new Error("Woven credential must contain 1–4096 characters");
  }
  const scope = readScope();
  const managed = authSelect.value === "bearer";
  const hash = certificateHash();
  const client = await WovenClient.connect({
    url,
    token,
    authenticationScheme: managed
      ? AuthenticationScheme.Bearer
      : AuthenticationScheme.Development,
    webTransportOptions:
      hash === null
        ? undefined
        : { serverCertificateHashes: [{ algorithm: "sha-256", value: hash }] },
    maxFrameBytes: 65_536,
    maxPayloadBytes: 65_536,
    connectTimeoutMs: SETUP_OPERATION_TIMEOUT_MS,
  });
  if (generation !== connectionGeneration) {
    client.close(0, "connection cancelled");
    return;
  }
  connectingClient = client;
  setupAbort = new AbortController();

  try {
    if (managed) {
      setNetworkStatus("Waiting for Woven admission…");
      const outcome = await client.admitWithCancellation(
        scope.namespaceId,
        scope.sessionId,
        crypto.randomUUID(),
        60_000,
        setupAbort.signal,
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
    setNetworkStatus("Subscribing to the shared room…");
    const entityId = await receiveAssignedEntity(client, setupAbort.signal);
    if (generation !== connectionGeneration || setupAbort.signal.aborted) {
      client.close(0, "connection cancelled");
      return;
    }

    const active: Connection = {
      client,
      entityId,
      scope,
      publisher: { inFlight: false, pending: null },
    };
    connectingClient = null;
    setupAbort = null;
    connection = active;
    if (!realtime_connected(entityId)) {
      disconnect("WASM realtime inbox overflow during connection", true);
      return;
    }
    updateControls("online");
    setNetworkStatus(`Online as Woven entity ${entityId}`);
    void receiveLoop(active, generation);
  } catch (error) {
    if (connectingClient === client) connectingClient = null;
    setupAbort = null;
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
    while (connection === active && generation === connectionGeneration) {
      processEnvelope(await active.client.recv(), active);
    }
  } catch (error) {
    if (connection === active) disconnect(`Woven receive failed: ${errorMessage(error)}`, true);
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
    throw new Error("Woven reported a protocol error");
  }
  if (!matchesScope(envelope, active.scope)) return;
  if (envelope.messageKind === MessageKind.EntityLeft && envelope.entityId !== null) {
    if (!realtime_entity_left(envelope.entityId)) {
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
    envelope.payload !== null &&
    !realtime_payload(
      envelope.entityId,
      envelope.senderSequence,
      decoder.decode(envelope.payload),
    )
  ) {
    throw new Error("WASM realtime inbox overflow while receiving a visitor pose");
  }
}

function disconnect(reason = "Disconnected from Woven", failed = false): void {
  const waitingForInitialHandshake =
    connectionAttemptActive && connection === null && connectingClient === null;
  const active = connection;
  connection = null;
  if (active !== null) active.publisher.pending = null;
  active?.client.close(0, "visitor disconnected");
  setupAbort?.abort();
  setupAbort = null;
  connectingClient?.close(0, "visitor cancelled connection");
  connectingClient = null;
  connectionGeneration += 1;
  if (wasmReady) realtime_disconnected(reason);
  if (waitingForInitialHandshake) {
    updateControls("connecting");
    disconnectButton.disabled = true;
    setNetworkStatus("Cancelling the bounded Woven handshake…", failed);
  } else {
    updateControls("offline");
    setNetworkStatus(reason, failed);
  }
}

async function pumpPublisher(active: Connection): Promise<void> {
  if (active.publisher.inFlight) return;
  active.publisher.inFlight = true;
  try {
    while (connection === active && active.publisher.pending !== null) {
      const publish = active.publisher.pending;
      active.publisher.pending = null;
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
    if (connection === active && active.publisher.pending !== null) void pumpPublisher(active);
  }
}

globalThis.weaverRealtimePublish = (sequence: bigint, payload: string): void => {
  const active = connection;
  if (active === null) return;
  active.publisher.pending = { sequence, payload };
  void pumpPublisher(active);
};

globalThis.weaverRealtimeFatal = (reason: string): void => {
  disconnect(reason, true);
};

try {
  await init();
  wasmReady = true;
} catch (error) {
  setNetworkStatus(`Unable to start Weaver: ${errorMessage(error)}`, true);
  throw error;
}

authSelect.addEventListener("change", setTokenLabel);
form.addEventListener("submit", (event) => {
  event.preventDefault();
  void establishConnection().catch((error: unknown) => {
    disconnect(`Unable to connect: ${connectionErrorMessage(error)}`, true);
  });
});
disconnectButton.addEventListener("click", () => disconnect());
window.addEventListener("pagehide", () => {
  connection?.client.close(0, "page hidden");
  connectingClient?.close(0, "page hidden");
});
setTokenLabel();
updateControls("offline");

try {
  if (await loadLocalDevelopmentCredential()) {
    setNetworkStatus("Local development credential loaded · connecting…");
    await establishConnection();
  }
} catch (error) {
  disconnect(`Unable to connect: ${connectionErrorMessage(error)}`, true);
}
