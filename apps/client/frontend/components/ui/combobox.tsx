/**
 * Project-styled Base UI combobox primitives with Animate UI motion: a trigger opens a popup whose
 * search input filters the option list.
 */

import * as React from 'react';
import { Combobox as ComboboxPrimitive } from '@animate-ui/react/primitives/base-ui/combobox';
import { CheckIcon, Search } from 'lucide-react';

import { LIST_ITEM_CLASS, LIST_POPUP_CLASS } from '@/components/ui/multi-select';
import { PresenceBoundary } from '@/components/ui/motion';
import { RevealScope } from '@/components/ui/reveal';
import { cn } from '@/lib/utils';

const Combobox = ComboboxPrimitive.Root;
const ComboboxTrigger = ComboboxPrimitive.Trigger;
const ComboboxValue = ComboboxPrimitive.Value;

type ComboboxContentProps = React.ComponentProps<typeof ComboboxPrimitive.Popup> & {
  align?: 'start' | 'center' | 'end';
  sideOffset?: number;
};

/**
 * Render the positioned popup in a portal with its own reveal surface.
 *
 * @param props - Popup properties, alignment, and offset from the trigger.
 * @returns Portaled combobox popup.
 */
function ComboboxContent({
  align = 'start',
  sideOffset = 6,
  className,
  children,
  ...props
}: ComboboxContentProps) {
  return (
    <ComboboxPrimitive.Portal>
      <ComboboxPrimitive.Positioner align={align} sideOffset={sideOffset} className="z-50">
        <PresenceBoundary>
          <RevealScope>
            <ComboboxPrimitive.Popup className={cn(LIST_POPUP_CLASS, className)} {...props}>
              {children}
            </ComboboxPrimitive.Popup>
          </RevealScope>
        </PresenceBoundary>
      </ComboboxPrimitive.Positioner>
    </ComboboxPrimitive.Portal>
  );
}

/**
 * Render the search field at the top of the popup.
 *
 * @param props - Base UI input properties.
 * @returns Search input with a leading icon.
 */
function ComboboxInput({
  className,
  ...props
}: React.ComponentProps<typeof ComboboxPrimitive.Input>) {
  return (
    <div className="relative shrink-0 border-b p-2">
      <Search
        className="pointer-events-none absolute top-1/2 left-4.5 size-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
      <ComboboxPrimitive.Input
        autoComplete="off"
        spellCheck={false}
        className={cn(
          'h-9 w-full min-w-0 rounded-lg bg-transparent pr-3 pl-9 text-sm shadow-vercel-ring outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50',
          className,
        )}
        {...props}
      />
    </div>
  );
}

/**
 * Render the scrollable option list.
 *
 * @param props - Base UI list properties.
 * @returns Scrollable option list.
 */
function ComboboxList({
  className,
  ...props
}: React.ComponentProps<typeof ComboboxPrimitive.List>) {
  return (
    <ComboboxPrimitive.List
      className={cn(
        'min-h-0 flex-1 overflow-y-auto overscroll-contain py-1.5 pr-3 pl-1.5 empty:hidden',
        className,
      )}
      {...props}
    />
  );
}

/**
 * Render the message shown when no option matches the search.
 *
 * @param props - Base UI empty-state properties.
 * @returns Empty-state message.
 */
function ComboboxEmpty({
  className,
  ...props
}: React.ComponentProps<typeof ComboboxPrimitive.Empty>) {
  return (
    <ComboboxPrimitive.Empty
      className={cn('px-4 py-3 text-xs text-muted-foreground empty:hidden', className)}
      {...props}
    />
  );
}

type ComboboxItemProps = React.ComponentProps<typeof ComboboxPrimitive.Item> & {
  /** Trailing detail such as a count. */
  detail?: React.ReactNode;
};

/**
 * Render one option with its animated check indicator.
 *
 * @param props - Base UI item properties, label, and optional trailing detail.
 * @returns Selectable option row.
 */
function ComboboxItem({ className, children, detail, ...props }: ComboboxItemProps) {
  return (
    <ComboboxPrimitive.Item className={cn(LIST_ITEM_CLASS, className)} {...props}>
      <ComboboxPrimitive.ItemIndicator className="flex size-4 shrink-0 items-center justify-center">
        <CheckIcon className="size-4" aria-hidden="true" />
      </ComboboxPrimitive.ItemIndicator>
      <span className="min-w-0 flex-1 break-words">{children}</span>
      {detail !== undefined && (
        <span className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground">
          <span className="sr-only"> · </span>
          {detail}
        </span>
      )}
    </ComboboxPrimitive.Item>
  );
}

export {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
  ComboboxValue,
};
