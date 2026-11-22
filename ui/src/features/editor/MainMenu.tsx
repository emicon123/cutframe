import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";

import { getAction, menuModel, type MenuView, useActionContext } from "./actions";

/** File, Edit, View and Help, generated from the action registry. */
export function MainMenu() {
  const menus = menuModel(useActionContext());
  return (
    <nav aria-label={t.menu.bar} className="flex items-center">
      {menus.map((menu) => (
        <MenuDropdown key={menu.id} menu={menu} />
      ))}
    </nav>
  );
}

function MenuDropdown({ menu }: { menu: MenuView }) {
  const [open, setOpen] = useState(false);
  usePreviewOccluder(open);

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm">
          {menu.label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        {menu.items.map((item, index) => {
          if (item.kind === "separator") return <DropdownMenuSeparator key={`sep-${index}`} />;
          const onSelect = () => getAction(item.id).run();
          const shortcut = item.shortcut && <DropdownMenuShortcut>{item.shortcut}</DropdownMenuShortcut>;
          return item.checked === undefined ? (
            <DropdownMenuItem key={item.id} disabled={item.disabled} onSelect={onSelect}>
              {item.label}
              {shortcut}
            </DropdownMenuItem>
          ) : (
            <DropdownMenuCheckboxItem
              key={item.id}
              checked={item.checked}
              disabled={item.disabled}
              onSelect={onSelect}
            >
              {item.label}
              {shortcut}
            </DropdownMenuCheckboxItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
