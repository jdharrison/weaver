// Hand-maintained type shim for `npm run typecheck`; packaged bindings are generated without TypeScript.
export default function init(input?: unknown): Promise<unknown>;
export function realtime_connected(entityId: bigint): boolean;
export function realtime_entity_left(entityId: bigint): boolean;
export function realtime_payload(entityId: bigint, sequence: bigint, payload: string): boolean;
export function realtime_disconnected(reason: string): boolean;
