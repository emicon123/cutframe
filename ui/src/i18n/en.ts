import {
  type HdrTransfer,
  type HistoryLabel,
  type IpcError,
  type JobStage,
  MAX_IMPORT_FILES,
  type MediaErrorKind,
  type MediaKind,
} from "@/bindings";

const APP_NAME = "Cutframe";

/** All UI strings. Add a key here instead of writing text in JSX. */
export const en = {
  app: {
    name: APP_NAME,
    /** Window title; `*` marks unsaved changes. */
    windowTitle: (name: string, dirty: boolean) => `${dirty ? "*" : ""}${name} - ${APP_NAME}`,
  },
  /** Key names used to display shortcuts, e.g. "Ctrl+Shift+S". */
  keys: { ctrl: "Ctrl", shift: "Shift", separator: "+", delete: "Del" },
  /** Main menu bar titles. */
  menu: {
    bar: "Main menu",
    file: "File",
    edit: "Edit",
    view: "View",
    help: "Help",
  },
  file: {
    new: "New",
    open: "Open…",
    save: "Save",
    saveAs: "Save As…",
    importMedia: "Import Media…",
    projectSettings: "Project settings…",
    exit: "Exit",
    /** Name of the project file type in the native open/save dialogs. */
    filterName: `${APP_NAME} project`,
    /** Save As file name when the project name has no usable characters. */
    untitled: "Untitled",
  },
  unsaved: {
    title: (name: string) => `Save changes to “${name}”?`,
    description: "Your changes will be lost if you don't save them.",
    save: "Save",
    discard: "Don't save",
    cancel: "Cancel",
  },
  recovery: {
    title: "Recover unsaved work",
    description: `${APP_NAME} did not close properly. These projects have changes that were not saved.`,
    notSavedYet: "Not saved yet",
    savedAt: (time: string) => `Autosaved ${time}`,
    restore: "Restore",
    discard: "Discard",
    restoreItem: (name: string) => `Restore “${name}”`,
    discardItem: (name: string) => `Discard “${name}”`,
    later: "Later",
  },
  projectSettings: {
    title: "Project settings",
    description: "Canvas size and frame rate. Clip times are kept exactly.",
    resolution: "Resolution",
    presets: {
      landscapeHd: "1920 × 1080 (16:9)",
      landscape4k: "3840 × 2160 (16:9 4K)",
      shorts: "1080 × 1920 (9:16 Shorts)",
      square: "1080 × 1080 (1:1)",
      custom: "Custom",
    },
    width: "Width",
    height: "Height",
    aspectRatio: "Aspect ratio",
    frameRate: "Frame rate",
    sizeErrors: {
      notInteger: "Enter a whole number",
      odd: "Must be an even number",
      outOfRange: (min: number, max: number) => `Must be between ${min} and ${max}`,
    },
    apply: "Apply",
    cancel: "Cancel",
    close: "Close",
  },
  edit: {
    undo: "Undo",
    redo: "Redo",
    /** Menu item and tooltip naming the entry, e.g. "Undo Add Track". */
    undoAction: (label: string) => `Undo ${label}`,
    redoAction: (label: string) => `Redo ${label}`,
  },
  view: {
    historyPanel: "History panel",
    keyboardShortcuts: "Keyboard shortcuts",
  },
  /** Tooltip text: the action, then its shortcut, e.g. "Undo Add Track (Ctrl+Z)". */
  tooltip: (label: string, shortcut: string) => (shortcut ? `${label} (${shortcut})` : label),
  shortcutsDialog: {
    title: "Keyboard shortcuts",
    description: "Shortcuts do nothing while you type in a text field or a dialog is open.",
    close: "Close",
    /** Between alternative shortcuts of one action, e.g. "Ctrl+Shift+Z or Ctrl+Y". */
    or: "or",
  },
  historyPanel: {
    title: "History",
    /** First row: the oldest state the history can go back to. */
    start: "Start of history",
    close: "Close history panel",
    /** Screen-reader suffix of an undone entry that redo can re-apply. */
    undone: "(undone)",
  },
  topBar: {
    export: "Export",
    exportSoon: "Export arrives in a later stage",
  },
  /** Undo/redo entries keyed by the backend's `HistoryLabel`, shown as "Undo Add Track". */
  history: {
    labels: {
      addTrack: "Add Track",
      removeTrack: "Remove Track",
      renameTrack: "Rename Track",
      renameProject: "Rename Project",
      setProjectSettings: "Change Project Settings",
      importMedia: "Import Media",
      removeMedia: "Remove Media",
    } satisfies Record<HistoryLabel, string>,
    /** Shown for a key this UI build does not know. */
    unknown: "Last Action",
  },
  /** Command failures keyed by `IpcError.kind`. `IpcError.message` is never shown. */
  errors: {
    kinds: {
      invalidEdit: "That edit could not be applied",
      unavailable: "This feature is not available on this computer",
      internal: "Something went wrong",
      fileNotFound: "The file could not be found",
      malformedProject: "This file is not a project file or it is damaged",
      projectTooNew: `This project was saved by a newer version of ${APP_NAME}`,
      invalidProject: "This project file contains errors and cannot be opened",
      io: "The file could not be read or written",
      cancelled: "Cancelled",
    } satisfies Record<IpcError["kind"], string>,
    /** Shown for an unknown kind or a failure that is not an `IpcError`. */
    unknown: "Something went wrong",
  },
  mediaBin: {
    title: "Media",
    empty: "No media yet",
    emptyHint: "Import video, audio and images, or drop files and folders here.",
    import: "Import",
    importTip: "Import media (Ctrl+I)",
    /** Name of the file-type filter in the native file dialog. */
    filterLabel: "Media files",
    dropHint: "Drop to import",
    list: "Imported media",
    kinds: { video: "Video", audio: "Audio", image: "Image" } satisfies Record<MediaKind, string>,
    resolution: (width: number, height: number) => `${width}×${height}`,
    /** Joins the details of a list row, e.g. "Video · 0:42 · 1080×1920". */
    detailSeparator: " · ",
    job: {
      stages: {
        probing: "Importing media",
        generatingPreviews: "Generating previews",
      } satisfies Record<JobStage, string>,
      /** Shown for a stage this UI build does not know. */
      unknownStage: "Working",
      percent: (value: number) => `${value}%`,
      cancel: "Cancel",
      cancelTip: "Cancel import (Esc)",
      cancelling: "Cancelling…",
    },
    result: {
      added: (n: number) => (n === 1 ? "1 file imported" : `${n} files imported`),
      alreadyImported: (n: number) => (n === 1 ? "1 file already imported" : `${n} files already imported`),
      failed: (n: number) => (n === 1 ? "1 file could not be imported" : `${n} files could not be imported`),
      nothing: "No media files found",
      jobFailed: (reason: string) => `Import failed: ${reason}`,
      fileReason: (fileName: string, reason: string) => `${fileName}: ${reason}`,
      dismiss: "Dismiss",
    },
    /** Per-file import failures keyed by `MediaErrorKind`. The English `detail` is never shown. */
    errors: {
      fileNotFound: "file not found",
      accessDenied: "access denied",
      unsupportedPath: "this path cannot be used",
      unsupportedFormat: "unsupported format",
      unsupportedCodec: "unsupported codec",
      damaged: "the file is damaged",
      tooManyFiles: `more than ${MAX_IMPORT_FILES} media files in this folder`,
    } satisfies Record<MediaErrorKind, string>,
    unknownError: "could not be read",
    layout: {
      grid: "Grid view",
      list: "List view",
    },
    search: "Search media by name",
    searchPlaceholder: "Search",
    clearSearch: "Clear search",
    sort: {
      label: "Sort media",
      keys: { imported: "Date added", name: "Name", duration: "Duration", kind: "Type" },
      ascending: "Ascending",
      descending: "Descending",
    },
    filter: {
      label: "Filter by type",
      active: (n: number) => `Filter by type (${n} selected)`,
    },
    noMatches: "No media matches the search or filter",
    clearFilters: "Clear search and filter",
    hdr: {
      badge: { pq: "HDR", hlg: "HDR", dolbyVisionOnly: "DV" } satisfies Record<HdrTransfer, string>,
      tip: {
        pq: "HDR (PQ)",
        hlg: "HDR (HLG)",
        dolbyVisionOnly: "Dolby Vision without a compatible base layer: colours may look wrong",
      } satisfies Record<HdrTransfer, string>,
    },
    previewUnavailable: "No preview",
    actions: {
      reveal: "Show in Explorer",
      remove: "Remove from project",
      removeMany: (n: number) => `Remove ${n} items from project`,
    },
    remove: {
      title: (n: number, name: string) => (n === 1 ? `Remove “${name}”?` : `Remove ${n} media items?`),
      description: "The files stay on disk. Undo (Ctrl+Z) brings them back.",
      more: (n: number) => `and ${n} more`,
      confirm: "Remove",
      cancel: "Cancel",
    },
    background: "Generating previews in the background",
  },
  preview: {
    unavailable: "Preview unavailable: no GPU adapter found",
    loading: "Starting preview…",
    play: "Play",
    playSoon: "Playback arrives with media import",
  },
  inspector: {
    title: "Project",
    name: "Name",
    resolution: "Resolution",
    frameRate: "Frame rate",
    sampleRate: "Sample rate",
    duration: "Duration",
    tracks: "Tracks",
  },
  timeline: {
    addVideoTrack: "Add video track",
    addAudioTrack: "Add audio track",
    addTextTrack: "Add text track",
    removeTrack: (name: string) => `Remove ${name}`,
    zoomHint: "Ctrl + wheel to zoom, wheel to scroll",
    kinds: { video: "Video", audio: "Audio", text: "Text" },
  },
  status: {
    gpu: (adapter: string, backend: string) => `${adapter} · ${backend}`,
    software: "software rendering",
    noGpu: "No GPU",
    previewMode: { native: "native preview", frames: "frame preview (dev)", unavailable: "no preview" },
    version: (v: string) => `v${v}`,
  },
} as const;
