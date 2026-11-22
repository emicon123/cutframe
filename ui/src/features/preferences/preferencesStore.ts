import { create } from "zustand";

import { type CacheInfo, commands } from "@/bindings";
import { setLocale } from "@/i18n";
import { logError } from "@/ipc/errors";

import type { Preferences } from "./preferences";

type PreferencesStore = {
  /** What Rust has in effect; `null` until loaded. Only `setPreferences` changes it. */
  preferences: Preferences | null;
  /** The last `getCacheInfo`/`clearCache` result; `null` until the cache tab asked. */
  cacheInfo: CacheInfo | null;
};

/** Mirror of the Rust preferences (design `preferences.md` §5). Not the project model. */
export const usePreferencesStore = create<PreferencesStore>()(() => ({ preferences: null, cacheInfo: null }));

function applied(preferences: Preferences): Preferences {
  usePreferencesStore.setState({ preferences });
  setLocale(preferences.language);
  return preferences;
}

/** Loads the preferences (startup, or a dialog opened before that finished). Logs a failure. */
export async function loadPreferences(): Promise<Preferences | null> {
  try {
    return applied(await commands.getPreferences());
  } catch (error) {
    logError(error);
    return null;
  }
}

/**
 * Sends the full preferences object (never a partial one: a missing field takes its Rust default)
 * and stores what Rust put in effect. Rejects with the `IpcError`; nothing changed then.
 */
export async function savePreferences(preferences: Preferences): Promise<Preferences> {
  return applied(await commands.setPreferences(preferences));
}

export async function refreshCacheInfo(): Promise<CacheInfo> {
  const cacheInfo = await commands.getCacheInfo();
  usePreferencesStore.setState({ cacheInfo });
  return cacheInfo;
}

export async function clearCache(): Promise<CacheInfo> {
  const cacheInfo = await commands.clearCache();
  usePreferencesStore.setState({ cacheInfo });
  return cacheInfo;
}
