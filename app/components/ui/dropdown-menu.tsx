'use client';

/**
 * Project-styled Radix dropdown menu primitives.
 */

import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import * as React from 'react';

import { cn } from '@/lib/utils';

/** Shared menu-item styles for dropdown menus. */
const DROPDOWN_MENU_ITEM_CLASS =
  "motion-control focus:bg-accent focus:text-accent-foreground data-[highlighted]:bg-accent active:bg-accent-pressed [&_svg:not([class*='text-'])]:text-muted-foreground relative flex min-h-9 w-full cursor-default items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm transition-[background-color,color] outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

/**
 * Render the dropdown menu state root.
 *
 * @param props - Radix root properties.
 * @returns Dropdown menu root.
 */
function DropdownMenu(props: React.ComponentProps<typeof DropdownMenuPrimitive.Root>) {
  return <DropdownMenuPrimitive.Root data-slot="dropdown-menu" {...props} />;
}

/**
 * Render the control that toggles the menu.
 *
 * @param props - Radix trigger properties.
 * @returns Dropdown menu trigger.
 */
function DropdownMenuTrigger(props: React.ComponentProps<typeof DropdownMenuPrimitive.Trigger>) {
  return <DropdownMenuPrimitive.Trigger data-slot="dropdown-menu-trigger" {...props} />;
}

/**
 * Render positioned menu content in a portal.
 *
 * @param props - Radix content properties.
 * @returns Portaled menu content.
 */
function DropdownMenuContent({
  className,
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Content>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        data-slot="dropdown-menu-content"
        sideOffset={sideOffset}
        collisionPadding={8}
        className={cn(
          'motion-popover bg-popover text-popover-foreground z-50 min-w-48 origin-(--radix-dropdown-menu-content-transform-origin) rounded-xl p-1.5 shadow-overlay outline-hidden',
          className,
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}

/**
 * Render one selectable menu item.
 *
 * @param props - Radix item properties and destructive tone.
 * @returns Menu item.
 */
function DropdownMenuItem({
  className,
  isDestructive = false,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Item> & { isDestructive?: boolean }) {
  return (
    <DropdownMenuPrimitive.Item
      data-slot="dropdown-menu-item"
      className={cn(
        DROPDOWN_MENU_ITEM_CLASS,
        isDestructive &&
          'text-destructive focus:bg-destructive/10 focus:text-destructive data-[highlighted]:bg-destructive/10 [&_svg:not([class*=text-])]:text-destructive',
        className,
      )}
      {...props}
    />
  );
}

/**
 * Render a non-interactive menu label.
 *
 * @param props - Radix label properties.
 * @returns Menu label.
 */
function DropdownMenuLabel({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Label>) {
  return (
    <DropdownMenuPrimitive.Label
      data-slot="dropdown-menu-label"
      className={cn('px-2.5 py-1.5 text-xs text-muted-foreground', className)}
      {...props}
    />
  );
}

/**
 * Render a visual separator between menu groups.
 *
 * @param props - Radix separator properties.
 * @returns Menu separator.
 */
function DropdownMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Separator>) {
  return (
    <DropdownMenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      className={cn('-mx-1.5 my-1.5 h-px bg-border', className)}
      {...props}
    />
  );
}

export {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
};
