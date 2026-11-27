import { useEffect } from "react";
import { type PanelSize, useDefaultLayout } from "react-resizable-panels";

import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Inspector } from "@/features/inspector/Inspector";
import { MediaBin } from "@/features/media-bin/MediaBin";
import { PreviewArea } from "@/features/preview/PreviewArea";
import { exitFullScreen } from "@/features/preview/previewStore";
import { Timeline } from "@/features/timeline/Timeline";
import { type PanelKey, useUiStore } from "@/stores/uiStore";

import { HistoryPanel } from "./HistoryPanel";
import { LAYOUT_IDS, layoutStorage, PANEL_REFS } from "./layout";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";

/** A collapsed panel has its `collapsedSize` (0); open ones never go below their `minSize`. */
const COLLAPSED_BELOW_PERCENT = 0.5;

function trackCollapsed(panel: PanelKey) {
  return (size: PanelSize) =>
    useUiStore.getState().setPanelCollapsed(panel, size.asPercentage < COLLAPSED_BELOW_PERCENT);
}

const onMediaBinResize = trackCollapsed("mediaBin");
const onInspectorResize = trackCollapsed("inspector");

export function EditorLayout() {
  // Reset panel layout bumps the version: the groups remount and read the (now empty) storage.
  const layoutVersion = useUiStore((s) => s.layoutVersion);
  // Leaving the editor (Home) leaves full screen too: the window must not stay full screen
  // without a preview in it.
  useEffect(() => exitFullScreen, []);
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <PanelGroups key={layoutVersion} />
      <StatusBar />
    </div>
  );
}

/**
 * Media bin, inspector and timeline collapse when dragged past their minimum size; sizes and
 * collapsed state are kept in `localStorage` (design `preferences.md` §6).
 */
function PanelGroups() {
  const historyPanelOpen = useUiStore((s) => s.historyPanelOpen);
  const rows = useDefaultLayout({ id: LAYOUT_IDS.rows, storage: layoutStorage });
  const columns = useDefaultLayout({ id: LAYOUT_IDS.columns, storage: layoutStorage });

  return (
    <ResizablePanelGroup
      orientation="vertical"
      className="min-h-0 flex-1"
      defaultLayout={rows.defaultLayout}
      onLayoutChanged={rows.onLayoutChanged}
    >
      <ResizablePanel id="panel-top" defaultSize="62%" minSize="30%">
        <ResizablePanelGroup
          orientation="horizontal"
          defaultLayout={columns.defaultLayout}
          onLayoutChanged={columns.onLayoutChanged}
        >
          <ResizablePanel
            id="panel-media-bin"
            defaultSize="22%"
            minSize="14%"
            maxSize="40%"
            collapsible
            panelRef={PANEL_REFS.mediaBin}
            onResize={onMediaBinResize}
          >
            <MediaBin />
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel id="panel-preview" defaultSize="56%" minSize="30%">
            <PreviewArea />
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel
            id="panel-inspector"
            defaultSize="22%"
            minSize="14%"
            maxSize="40%"
            collapsible
            panelRef={PANEL_REFS.inspector}
            onResize={onInspectorResize}
          >
            {/* Beside the preview, never over it: no preview occluder needed. */}
            <div className="flex h-full flex-col">
              <div className="min-h-0 flex-1">
                <Inspector />
              </div>
              {historyPanelOpen && (
                <div className="flex h-2/5 min-h-32 flex-col">
                  <HistoryPanel />
                </div>
              )}
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </ResizablePanel>
      <ResizableHandle />
      <ResizablePanel id="panel-timeline" defaultSize="38%" minSize="20%" collapsible>
        <Timeline />
      </ResizablePanel>
    </ResizablePanelGroup>
  );
}
