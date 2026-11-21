import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Inspector } from "@/features/inspector/Inspector";
import { MediaBin } from "@/features/media-bin/MediaBin";
import { PreviewArea } from "@/features/preview/PreviewArea";
import { Timeline } from "@/features/timeline/Timeline";

import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";

export function EditorLayout() {
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
        <ResizablePanel defaultSize="62%" minSize="30%">
          <ResizablePanelGroup orientation="horizontal">
            <ResizablePanel defaultSize="22%" minSize="14%" maxSize="40%">
              <MediaBin />
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel defaultSize="56%" minSize="30%">
              <PreviewArea />
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel defaultSize="22%" minSize="14%" maxSize="40%">
              <Inspector />
            </ResizablePanel>
          </ResizablePanelGroup>
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="38%" minSize="20%">
          <Timeline />
        </ResizablePanel>
      </ResizablePanelGroup>
      <StatusBar />
    </div>
  );
}
