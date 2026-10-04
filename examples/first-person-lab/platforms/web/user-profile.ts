export type UserProfile = Readonly<{
  userId: string;
  displayName: string;
}>;

export type ProfileStorage = Pick<Storage, "getItem" | "setItem">;

export const USER_PROFILE_KEY = "weaver.user-profile.v1";
const MAX_STORED_PROFILE_BYTES = 512;
const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function validDisplayName(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value === value.trim() &&
    [...value].length >= 1 &&
    [...value].length <= 24 &&
    !/[\u0000-\u001f\u007f-\u009f]/u.test(value)
  );
}

/** Keep a local guest profile stable without treating it as authenticated identity. */
export function loadUserProfile(
  storage: ProfileStorage | null,
  generators: { generateUserName: () => string; generateUserId: () => string },
): UserProfile {
  let stored: Record<string, unknown> = {};
  try {
    const text = storage?.getItem(USER_PROFILE_KEY);
    if (text && new TextEncoder().encode(text).byteLength <= MAX_STORED_PROFILE_BYTES) {
      const parsed: unknown = JSON.parse(text);
      if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
        stored = parsed as Record<string, unknown>;
      }
    }
  } catch {
    // Disabled storage or malformed data must not prevent the lab from starting.
  }
  const profile: UserProfile = {
    userId:
      typeof stored.userId === "string" && UUID_V4.test(stored.userId)
        ? stored.userId
        : generators.generateUserId(),
    displayName: validDisplayName(stored.displayName)
      ? stored.displayName
      : generators.generateUserName(),
  };
  saveUserProfile(storage, profile);
  return profile;
}

/** Storage may be unavailable; the caller still retains the profile for this page. */
export function saveUserProfile(storage: ProfileStorage | null, profile: UserProfile): boolean {
  if (storage === null || !UUID_V4.test(profile.userId) || !validDisplayName(profile.displayName)) {
    return false;
  }
  try {
    storage.setItem(USER_PROFILE_KEY, JSON.stringify(profile));
    return true;
  } catch {
    return false;
  }
}
