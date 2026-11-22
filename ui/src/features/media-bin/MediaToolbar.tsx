import { ArrowDownWideNarrow, Filter, LayoutGrid, List, Search, X } from "lucide-react";
import { useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";
import { type MediaLayout, useUiStore } from "@/stores/uiStore";

import { MEDIA_KINDS, MEDIA_SORT_KEYS, toggleKind } from "./browse";

/** Search, type filter, sort and grid/list toggle of the media panel. */
export function MediaToolbar({
  query,
  onQueryChange,
}: {
  query: string;
  onQueryChange: (query: string) => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-1 border-b px-2 py-1">
      <div className="relative min-w-0 flex-1">
        <Search
          className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
          aria-hidden
        />
        <Input
          type="search"
          value={query}
          placeholder={t.mediaBin.searchPlaceholder}
          aria-label={t.mediaBin.search}
          className="h-7 pr-7 pl-7 text-xs md:text-xs [&::-webkit-search-cancel-button]:hidden"
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && query !== "") {
              event.stopPropagation();
              onQueryChange("");
            }
          }}
        />
        {query !== "" && (
          <Button
            variant="ghost"
            size="icon-xs"
            className="absolute top-1/2 right-0.5 -translate-y-1/2"
            aria-label={t.mediaBin.clearSearch}
            onClick={() => onQueryChange("")}
          >
            <X aria-hidden />
          </Button>
        )}
      </div>
      <KindFilter />
      <SortMenu />
      <LayoutToggle />
    </div>
  );
}

function useOccludingMenu() {
  const [open, setOpen] = useState(false);
  usePreviewOccluder(open);
  return { open, onOpenChange: setOpen };
}

function KindFilter() {
  const menu = useOccludingMenu();
  const kinds = useUiStore(useShallow((s) => s.mediaKinds));
  const setKinds = useUiStore((s) => s.setMediaKinds);
  const label = kinds.length > 0 ? t.mediaBin.filter.active(kinds.length) : t.mediaBin.filter.label;
  return (
    <DropdownMenu {...menu}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={label}
          title={label}
          className={cn(kinds.length > 0 && "text-primary")}
        >
          <Filter aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {MEDIA_KINDS.map((kind) => (
          <DropdownMenuCheckboxItem
            key={kind}
            checked={kinds.includes(kind)}
            onSelect={(event) => event.preventDefault()}
            onCheckedChange={() => setKinds(toggleKind(kinds, kind))}
          >
            {t.mediaBin.kinds[kind]}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SortMenu() {
  const menu = useOccludingMenu();
  const { sort, direction } = useUiStore(
    useShallow((s) => ({ sort: s.mediaSort, direction: s.mediaSortDirection })),
  );
  const setSort = useUiStore((s) => s.setMediaSort);
  return (
    <DropdownMenu {...menu}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t.mediaBin.sort.label}
          title={t.mediaBin.sort.label}
        >
          <ArrowDownWideNarrow aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {MEDIA_SORT_KEYS.map((key) => (
          <DropdownMenuCheckboxItem
            key={key}
            checked={sort === key}
            onCheckedChange={() => setSort(key, direction)}
          >
            {t.mediaBin.sort.keys[key]}
          </DropdownMenuCheckboxItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuCheckboxItem checked={direction === "asc"} onCheckedChange={() => setSort(sort, "asc")}>
          {t.mediaBin.sort.ascending}
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem
          checked={direction === "desc"}
          onCheckedChange={() => setSort(sort, "desc")}
        >
          {t.mediaBin.sort.descending}
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const LAYOUTS = [
  { layout: "grid", Icon: LayoutGrid },
  { layout: "list", Icon: List },
] as const satisfies readonly { layout: MediaLayout; Icon: unknown }[];

function LayoutToggle() {
  const current = useUiStore((s) => s.mediaLayout);
  const setLayout = useUiStore((s) => s.setMediaLayout);
  return (
    <div className="flex">
      {LAYOUTS.map(({ layout, Icon }) => (
        <Button
          key={layout}
          variant="ghost"
          size="icon-xs"
          aria-label={t.mediaBin.layout[layout]}
          title={t.mediaBin.layout[layout]}
          aria-pressed={current === layout}
          className={cn(current === layout && "bg-accent text-accent-foreground")}
          onClick={() => setLayout(layout)}
        >
          <Icon aria-hidden />
        </Button>
      ))}
    </div>
  );
}
