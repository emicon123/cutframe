import {
  type GpuDeviceType,
  type GpuFallback,
  type HdrTransfer,
  type HistoryLabel,
  type IpcError,
  type JobStage,
  MAX_IMPORT_FILES,
  type MediaErrorKind,
  type MediaKind,
  type MediaStatus,
  type UiLanguage,
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
    /** File → Home: closes the project (unsaved prompt) and shows the home screen. */
    home: "Home",
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
  home: {
    title: "Projects",
    newProject: "New project",
    open: "Open…",
    recent: "Recent projects",
    empty: "No recent projects yet",
    emptyHint: "Create a new project or open one from a folder.",
    /** Placeholder label of a card whose cover could not be loaded. */
    noCover: "No cover",
    missing: "Missing",
    missingHint: "The file was moved or deleted",
    modified: (date: string) => `Modified ${date}`,
    /** Name of a duplicated project, passed to `duplicateProjectFile`. */
    copyName: (name: string) => `${name} copy`,
    cardMenu: (name: string) => `Actions for “${name}”`,
    openItem: (name: string) => `Open “${name}”`,
    actions: {
      open: "Open",
      rename: "Rename…",
      duplicate: "Duplicate",
      reveal: "Show in Explorer",
      delete: "Delete…",
      remove: "Remove from list",
    },
    rename: {
      title: "Rename project",
      description: "Renames the project and its file.",
      label: "Name",
      apply: "Rename",
      cancel: "Cancel",
      /** `invalidEdit` from `renameProjectFile`. */
      conflict: "A project with this name already exists or the name is empty",
    },
    delete: {
      title: (name: string) => `Move “${name}” to the Recycle Bin?`,
      description: "You can restore it from the Recycle Bin.",
      confirm: "Move to Recycle Bin",
      cancel: "Cancel",
    },
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
    preferences: "Preferences…",
    undo: "Undo",
    redo: "Redo",
    /** Menu item and tooltip naming the entry, e.g. "Undo Add Track". */
    undoAction: (label: string) => `Undo ${label}`,
    redoAction: (label: string) => `Redo ${label}`,
  },
  view: {
    historyPanel: "History panel",
    mediaBin: "Media bin",
    inspector: "Inspector",
    resetPanelLayout: "Reset panel layout",
    keyboardShortcuts: "Keyboard shortcuts",
  },
  preferences: {
    title: "Preferences",
    description: "Settings for this computer. They are not saved in the project.",
    tabs: {
      general: "General",
      editing: "Editing",
      performance: "Performance",
      cache: "Cache",
      shortcuts: "Shortcuts",
    },
    loading: "Loading preferences…",
    loadFailed: "The preferences could not be loaded",
    save: "Save",
    cancel: "Cancel",
    close: "Close",
    seconds: "seconds",
    general: {
      language: "Language",
      languages: { en: "English" } satisfies Record<UiLanguage, string>,
      autosave: "Autosave for crash recovery",
      autosaveInterval: "Autosave every",
      autosaveOffHint: "Off: unsaved work cannot be recovered after a crash.",
      panelLayout: "Panel layout",
      resetPanelLayout: "Reset panel layout",
    },
    editing: {
      imageDuration: "Default image duration",
      transitionDuration: "Default transition duration",
      hint: "Used when an image or a transition is added to the timeline.",
    },
    fieldErrors: {
      notNumber: "Enter a number",
      notWhole: "Enter a whole number of seconds",
      outOfRange: (min: string, max: string) => `Must be between ${min} and ${max} seconds`,
      rejected: "This value was not accepted",
    },
    performance: {
      noGpu: "No GPU could be initialized. The preview is not available.",
      adapter: "Adapter",
      backend: "Graphics API",
      deviceType: "Device type",
      driver: "Driver",
      notReported: "Not reported",
      software: "Software rendering",
      presentsToWindow: "Native preview window",
      fallback: "Fallback",
      noFallback: "None",
      previewMode: "Preview",
      videoAcceleration: "Hardware video acceleration",
      videoAccelerationUnknown: "Not detected in this version",
      decoders: (list: string) => `Decoders: ${list}`,
      encoders: (list: string) => `Encoders: ${list}`,
      none: "none",
      yes: "Yes",
      no: "No",
    },
    cache: {
      folder: "Cache folder",
      size: "Size",
      files: (n: number) => (n === 1 ? "1 file" : `${n} files`),
      sizeUnknown: "Unknown",
      change: "Change…",
      resetFolder: "Use default folder",
      clear: "Clear cache…",
      confirmClear: "Delete all cached files? They are recreated when needed.",
      confirm: "Clear cache",
      cancel: "Keep files",
      hint: "Thumbnails, waveforms and proxies are stored here. Only files inside this folder are ever deleted.",
      pickTitle: "Choose the cache folder",
    },
    shortcuts: {
      description: "Shortcuts cannot be changed yet.",
      show: "Show keyboard shortcuts",
    },
  },
  /** Byte sizes, 1024-based like Explorer. */
  units: {
    bytes: (n: string) => `${n} B`,
    scaled: (value: string, unit: "KB" | "MB" | "GB" | "TB") => `${value} ${unit}`,
  },
  gpu: {
    deviceTypes: {
      discrete: "Discrete GPU",
      integrated: "Integrated GPU",
      virtual: "Virtual GPU",
      software: "Software (CPU)",
      other: "Other",
    } satisfies Record<GpuDeviceType, string>,
    fallbacks: {
      forcedByEnv: "Software rendering was forced by an environment variable.",
      preferredCannotPresent: "The preferred GPU cannot show the preview window, so another one is used.",
      noHardwareAdapter: "No hardware GPU was found: software rendering (slower).",
      deviceLost: "The GPU stopped responding and the preview was restarted.",
    } satisfies Record<GpuFallback, string>,
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
      relinkMedia: "Relink Media",
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
        checkingMedia: "Checking media files",
        searchingFolder: "Searching folder",
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
      kindMismatch: "the file is a different kind of media (for example audio instead of video)",
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
    status: {
      badge: { offline: "Missing", changed: "Changed" } satisfies Record<
        Exclude<MediaStatus, "online">,
        string
      >,
      tip: {
        offline: "The file is missing: it was moved, renamed or deleted, or its drive is not connected",
        changed: "The file has different content than when it was imported",
      } satisfies Record<Exclude<MediaStatus, "online">, string>,
    },
    notice: {
      text: (offline: number, changed: number) =>
        [
          offline > 0 && (offline === 1 ? "1 file missing" : `${offline} files missing`),
          changed > 0 && (changed === 1 ? "1 file changed" : `${changed} files changed`),
        ]
          .filter((part) => typeof part === "string")
          .join(", "),
      relink: "Relink…",
      dismiss: "Hide notice",
    },
    relink: {
      action: "Relink…",
      titleOne: (name: string) => `Relink “${name}”`,
      titleAll: "Relink media",
      description:
        "Locate each file, or search a folder: every missing file found there is relinked by its content.",
      allFound: "All media files are found.",
      locate: "Locate…",
      locateItem: (name: string) => `Locate “${name}”`,
      locateTitle: (name: string) => `Locate ${name}`,
      keepAsIs: "Use as it is now",
      keepAsIsItem: (name: string) => `Use “${name}” as it is now`,
      searchFolder: "Search folder…",
      folderTitle: "Search a folder for missing media",
      close: "Close",
      cancel: "Cancel",
      mismatch: "This file has different content than the original. Use it anyway?",
      useAnyway: "Use anyway",
      outcomes: {
        relinked: (n: number) => (n === 1 ? "1 file relinked" : `${n} files relinked`),
        notFound: (n: number) => (n === 1 ? "1 file not found" : `${n} files not found`),
        fingerprintMismatch: "The chosen file has different content",
        nothingMissing: "No files are missing",
        cancelled: "Relink cancelled",
      },
      jobFailed: (reason: string) => `Relink failed: ${reason}`,
    },
  },
  preview: {
    unavailable: "Preview unavailable: no usable GPU",
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
    /** After a runtime GPU loss; `adapter` is the one the preview now runs on. */
    gpuRestarted: (adapter: string) => `The GPU stopped responding. The preview was restarted on ${adapter}.`,
    gpuRestartedSoftware: "The GPU stopped responding. The preview now uses software rendering (slower).",
    previewLost: "The GPU stopped responding repeatedly. The preview was turned off.",
    dismiss: "Dismiss",
    software: "software rendering",
    noGpu: "No GPU",
    previewMode: { native: "native preview", frames: "frame preview (dev)", unavailable: "no preview" },
    version: (v: string) => `v${v}`,
  },
} as const;
