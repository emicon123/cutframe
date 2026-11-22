import type { LayoutStorage, PanelImperativeHandle } from "react-resizable-panels";

import { type PanelKey, useUiStore } from "@/stores/uiStore";

/**
 * `useDefaultLayout` ids of the editor's panel groups (design `preferences.md` §6). Bump `v1`
 * when the panels change, so an old layout is not applied to new panels.
 */
export const LAYOUT_IDS = {
  rows: "layout.editor.v1.rows",
  columns: "layout.editor.v1.columns",
} as const;

/** Every storage key that holds an editor layout (any version) contains this. */
const LAYOUT_KEY_MARKER = "layout.editor.";

/** A saved layout is an object of panel id → size; anything else is ignored. */
function isLayout(text: string): boolean {
  try {
    const value: unknown = JSON.parse(text);
    return (
      typeof value === "object" &&
      value !== null &&
      !Array.isArray(value) &&
      Object.values(value).every((size) => typeof size === "number" && Number.isFinite(size))
    );
  } catch {
    return false;
  }
}

/**
 * Wraps a storage so that a blocked or throwing storage, or a damaged entry, gives the default
 * layout instead of an error: `getItem` returns `null`, `setItem` does nothing. `backing` is a
 * function because reading `window.localStorage` itself can throw.
 */
export function safeLayoutStorage(backing: () => Storage): LayoutStorage {
  return {
    getItem: (key) => {
      try {
        const text = backing().getItem(key);
        return text !== null && isLayout(text) ? text : null;
      } catch {
        return null;
      }
    },
    setItem: (key, value) => {
      try {
        backing().setItem(key, value);
      } catch {
        // Blocked or full storage: the layout is simply not remembered.
      }
    },
  };
}

/** Removes every saved editor layout. Returns `false` when the storage could not be used. */
export function removeSavedLayouts(backing: () => Storage): boolean {
  try {
    const storage = backing();
    const keys: string[] = [];
    for (let i = 0; i < storage.length; i++) {
      const key = storage.key(i);
      if (key?.includes(LAYOUT_KEY_MARKER)) keys.push(key);
    }
    for (const key of keys) storage.removeItem(key);
    return true;
  } catch {
    return false;
  }
}

const browserStorage = () => window.localStorage;

/** The storage the editor layout uses. */
export const layoutStorage = safeLayoutStorage(browserStorage);

/** Drops the saved layout and remounts the panels with their default sizes. */
export function resetPanelLayout(): void {
  removeSavedLayouts(browserStorage);
  useUiStore.getState().bumpLayoutVersion();
}

const handles = new Map<PanelKey, PanelImperativeHandle>();

function panelRef(key: PanelKey) {
  return (handle: PanelImperativeHandle | null) => {
    if (!handle) return;
    handles.set(key, handle);
    return () => {
      if (handles.get(key) === handle) handles.delete(key);
    };
  };
}

/** Stable ref callbacks for `<Panel panelRef>`; they register the handle the toggles use. */
export const PANEL_REFS: Readonly<Record<PanelKey, ReturnType<typeof panelRef>>> = {
  mediaBin: panelRef("mediaBin"),
  inspector: panelRef("inspector"),
};

/** Collapses an open panel or expands a collapsed one (View → Media bin / Inspector). */
export function togglePanel(key: PanelKey): void {
  const handle = handles.get(key);
  if (!handle) return;
  if (handle.isCollapsed()) handle.expand();
  else handle.collapse();
}
