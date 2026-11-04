"use client";

import { useState } from "react";
import {
  Sidebar,
  SidebarHeader,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarGroupActions,
  SidebarGroupAction,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarMenuAction,
  type SidebarProps,
} from "@/components/ui/sidebar";
import { Tooltip } from "@/components/ui/tooltip";
import { DropdownMenu, DropdownTrigger, DropdownContent } from "@/components/ui/dropdown";
import { MenuItem } from "@/components/ui/menu-item";
import { useIcons } from "@/lib/icon-context";
import { SidebarWorkspaceHeader, WorkspaceTile } from "@/components/sidebar-app/workspace-header";
import { SidebarUserFooter } from "@/components/sidebar-app/user-footer";
import { SidebarSearchField } from "@/components/sidebar-app/search-field";
import { NAV_SECTIONS } from "@/components/sidebar-app/nav-data";

export function AppSidebar(props: Omit<SidebarProps, "children">) {
  const [active, setActive] = useState("New pricing page exploration");
  const icons = useIcons();
  const PlusIcon = icons.plus;
  const PencilIcon = icons.pencil;
  const MoreVerticalIcon = icons["more-vertical"];
  const LinkIcon = icons.link;
  const UserIcon = icons.user;
  const SettingsIcon = icons.settings;
  const ArrowLeftIcon = icons["arrow-left"];
  const FooterSettingsIcon = icons.settings;
  const MoonIcon = icons.moon;

  return (
    <Sidebar variant="inset" {...props}>
      <SidebarHeader>
        <SidebarWorkspaceHeader
          name="Acme Inc"
          tile={<WorkspaceTile>A</WorkspaceTile>}
          checkedIndex={0}
          menu={
            <>
              <MenuItem index={0} label="Acme Inc" checked onSelect={() => {}} />
              <MenuItem index={1} label="Personal" onSelect={() => {}} />
              <MenuItem index={2} icon={PlusIcon} label="New workspace" onSelect={() => {}} />
            </>
          }
        />
        {/* search + action rows are ONE block on the menu rows' rhythm */}
        <div className="flex flex-col gap-0.5">
          <SidebarSearchField />
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton icon={PlusIcon}>
                New
                {/* shortcut chip, revealed on row hover */}
                <span className="ml-auto inline-flex opacity-0 transition-opacity duration-80
                  group-hover/menu-item:opacity-100 group-focus-within/menu-item:opacity-100">
                  <kbd className="font-sans text-[11px] text-muted-foreground">⇧⌘O</kbd>
                </span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </div>
      </SidebarHeader>

      <SidebarContent>
        {NAV_SECTIONS.map((section) => (
          <SidebarGroup key={section.label} collapsible>
            <SidebarGroupLabel>{section.label}</SidebarGroupLabel>
            <SidebarGroupActions>
              <Tooltip content="Add item" side="top">
                <SidebarGroupAction aria-label="Add item">
                  <PlusIcon />
                </SidebarGroupAction>
              </Tooltip>
            </SidebarGroupActions>
            <SidebarMenu>
              {section.items.map((item) => (
                <SidebarMenuItem key={item.label}>
                  {/* status drives the dot and the screen-reader "unread" text */}
                  <SidebarMenuButton
                    status={item.status}
                    isActive={item.label === active}
                    onClick={() => setActive(item.label)}
                  >
                    {item.label}
                  </SidebarMenuButton>
                  <DropdownMenu>
                    <DropdownTrigger render={
                      <SidebarMenuAction showOnHover aria-label="More options">
                        <MoreVerticalIcon />
                      </SidebarMenuAction>
                    } />
                    {/* 240px — the header/footer trigger width */}
                    <DropdownContent className="min-w-0 w-[240px]" align="start" sideOffset={4}>
                      <MenuItem index={0} icon={PencilIcon} label="Rename" onSelect={() => {}} />
                      <MenuItem index={1} icon={LinkIcon} label="Share" onSelect={() => {}} />
                    </DropdownContent>
                  </DropdownMenu>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroup>
        ))}
      </SidebarContent>

      <SidebarFooter>
        <div className="flex items-center gap-1 pr-1.5">
          <SidebarUserFooter
            name="Jane Doe"
            avatar={
              <span className="flex size-5 items-center justify-center rounded-full bg-muted-foreground text-[10px] text-background">
                J
              </span>
            }
            className="min-w-0 flex-1"
            menu={
              <>
                <MenuItem index={0} icon={UserIcon} label="Profile" onSelect={() => {}} />
                <MenuItem index={1} icon={SettingsIcon} label="Settings" onSelect={() => {}} />
                <MenuItem index={2} icon={ArrowLeftIcon} label="Log out" onSelect={() => {}} />
              </>
            }
          />
          <Tooltip content="Settings" side="top">
            <button type="button" aria-label="Settings"
              className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground
                outline-none hover:bg-hover hover:text-foreground transition-colors duration-80
                focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring,#6B97FF)]">
              <FooterSettingsIcon size={16} strokeWidth={1.5} />
            </button>
          </Tooltip>
          <Tooltip content="Theme" side="top">
            <button type="button" aria-label="Theme"
              className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground
                outline-none hover:bg-hover hover:text-foreground transition-colors duration-80
                focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring,#6B97FF)]">
              <MoonIcon size={16} strokeWidth={1.5} />
            </button>
          </Tooltip>
        </div>
      </SidebarFooter>
    </Sidebar>
  );
}
