import type { HistoryLabel, IpcError } from "@/bindings";

const APP_NAME = "Cutframe";

/** All UI strings. Add a key here instead of writing text in JSX. */
export const en = {
  app: {
    name: APP_NAME,
    /** Window title; `*` marks unsaved changes. */
    windowTitle: (name: string, dirty: boolean) => `${dirty ? "*" : ""}${name} - ${APP_NAME}`,
  },
  /** Key names used to display shortcuts, e.g. "Ctrl+Shift+S". */
  keys: { ctrl: "Ctrl", shift: "Shift", separator: "+" },
  file: {
    menu: "File",
    new: "New",
    open: "Open…",
    save: "Save",
    saveAs: "Save As…",
    projectSettings: "Project settings…",
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
  topBar: {
    undo: "Undo",
    redo: "Redo",
    undoAction: (label: string) => `Undo ${label}`,
    redoAction: (label: string) => `Redo ${label}`,
    nothingToUndo: "Nothing to undo",
    nothingToRedo: "Nothing to redo",
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
    } satisfies Record<IpcError["kind"], string>,
    /** Shown for an unknown kind or a failure that is not an `IpcError`. */
    unknown: "Something went wrong",
  },
  mediaBin: {
    title: "Media",
    empty: "No media yet",
    emptyHint: "Importing video, audio and images arrives in stage 1.",
    import: "Import",
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
