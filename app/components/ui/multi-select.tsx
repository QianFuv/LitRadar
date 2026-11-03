'use client';

/**
 * Project-styled Base UI select primitives with Animate UI motion, for single or multiple values.
 */

import * as React from 'react';
import { Select as SelectPrimitive } from '@animate-ui/react/primitives/base-ui/select';
import { CheckIcon } from 'lucide-react';

import { RevealScope } from '@/components/ui/reveal';
import { cn } from '@/lib/utils';

/** Shared option row styles for Base UI list items. */
export const LIST_ITEM_CLASS =
  'motion-control relative flex min-h-10 w-full cursor-default items-center gap-2 rounded-lg px-2.5 py-2 text-[13px] leading-5 outline-hidden select-none transition-[background-color,color] data-[highlighted]:bg-accent active:bg-accent-pressed data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[selected]:font-bold md:min-h-9';

/** Shared popup surface styles for Base UI lists. */
export const LIST_POPUP_CLASS =
  'bg-popover text-popover-foreground flex max-h-(--available-height) w-(--anchor-width) min-w-56 origin-(--transform-origin) flex-col overflow-hidden rounded-xl shadow-overlay outline-hidden';

const MultiSelect = SelectPrimitive.Root;
const MultiSelectTrigger = SelectPrimitive.Trigger;
const MultiSelectValue = SelectPrimitive.Value;
const MultiSelectGroup = SelectPrimitive.Group;

type MultiSelectContentProps = React.ComponentProps<typeof SelectPrimitive.Popup> & {
  align?: 'start' | 'center' | 'end';
  sideOffset?: number;
};

/**
 * Render the positioned option popup in a portal with its own reveal surface.
 *
 * @param props - Popup properties, alignment, and offset from the trigger.
 * @returns Portaled select popup.
 */
function MultiSelectContent({
  align = 'start',
  sideOffset = 6,
  className,
  children,
  ...props
}: MultiSelectContentProps) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Positioner
        align={align}
        sideOffset={sideOffset}
        alignItemWithTrigger={false}
        className="z-50"
      >
        <RevealScope>
          <SelectPrimitive.Popup className={cn(LIST_POPUP_CLASS, className)} {...props}>
            {children}
          </SelectPrimitive.Popup>
        </RevealScope>
      </SelectPrimitive.Positioner>
    </SelectPrimitive.Portal>
  );
}

/**
 * Render the scrollable option list.
 *
 * @param props - Base UI list properties.
 * @returns Scrollable option list.
 */
function MultiSelectList({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.List>) {
  return (
    <SelectPrimitive.List
      className={cn(
        'min-h-0 flex-1 overflow-y-auto overscroll-contain py-1.5 pr-3 pl-1.5',
        className,
      )}
      {...props}
    />
  );
}

/**
 * Render one group heading.
 *
 * @param props - Base UI group label properties.
 * @returns Group heading.
 */
function MultiSelectGroupLabel({
  className,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.GroupLabel>) {
  return (
    <SelectPrimitive.GroupLabel
      className={cn('px-2.5 pt-2 pb-1 text-xs font-bold text-muted-foreground', className)}
      {...props}
    />
  );
}

type MultiSelectItemProps = React.ComponentProps<typeof SelectPrimitive.Item> & {
  /** Trailing detail such as a count, kept out of the item's typeahead text. */
  detail?: React.ReactNode;
};

/**
 * Render one option with its animated check indicator.
 *
 * @param props - Base UI item properties, label, and optional trailing detail.
 * @returns Selectable option row.
 */
function MultiSelectItem({ className, children, detail, ...props }: MultiSelectItemProps) {
  return (
    <SelectPrimitive.Item className={cn(LIST_ITEM_CLASS, className)} {...props}>
      <SelectPrimitive.ItemIndicator className="flex size-4 shrink-0 items-center justify-center">
        <CheckIcon className="size-4" aria-hidden="true" />
      </SelectPrimitive.ItemIndicator>
      <SelectPrimitive.ItemText className="min-w-0 flex-1 break-words">
        {children}
      </SelectPrimitive.ItemText>
      {detail !== undefined && (
        <span className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground">
          <span className="sr-only"> · </span>
          {detail}
        </span>
      )}
    </SelectPrimitive.Item>
  );
}

export {
  MultiSelect,
  MultiSelectContent,
  MultiSelectGroup,
  MultiSelectGroupLabel,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectTrigger,
  MultiSelectValue,
};
