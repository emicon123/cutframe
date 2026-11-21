import type { HistoryLabel, IpcError } from "@/bindings";

/** All UI strings. Add a key here instead of writing text in JSX. */
export const en = {
  app: { name: "Cutframe" },
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
