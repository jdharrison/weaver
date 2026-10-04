// Hand-maintained type shim for `npm run typecheck`; packaged bindings are generated without TypeScript.
export default function init(input?: unknown): Promise<unknown>;
export function realtime_connected(entityId: bigint): boolean;
export function realtime_entity_left(entityId: bigint): boolean;
export function realtime_entity_entered(entityId: bigint): boolean;
export function realtime_unreliable_payload(entityId: bigint, sequence: bigint, payload: Uint8Array): boolean;
export function realtime_payload(entityId: bigint, sequence: bigint, payload: string): boolean;
export function realtime_disconnected(reason: string): boolean;
export function set_display_name(value: string): boolean;
export function GenerateUserName(adjectives?: number, prefix?: string, postfix?: string): string;
export function GenerateUserID(): string;
