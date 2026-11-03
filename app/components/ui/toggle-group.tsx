'use client';

/**
 * Project-styled Radix toggle group whose selected surface slides between items (Animate UI).
 */

import * as React from 'react';
import * as ToggleGroupPrimitive from '@animate-ui/react/primitives/radix-ui/toggle-group';

import { cn } from '@/lib/utils';

/**
 * Render the segmented control root.
 *
 * @param props - Radix toggle group properties.
 * @returns Segmented control root.
 */
function ToggleGroup({
  className,
  ...props
}: React.ComponentProps<typeof ToggleGroupPrimitive.Root>) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      className={cn('flex w-fit items-center rounded-lg border p-0.5', className)}
      {...props}
    />
  );
}

/**
 * Render one segment; the selected segment's background becomes the sliding surface.
 *
 * @param props - Radix toggle group item properties.
 * @returns Segment button.
 */
function ToggleGroupItem({
  className,
  ...props
}: React.ComponentProps<typeof ToggleGroupPrimitive.Item>) {
  return (
    <ToggleGroupPrimitive.Item
      data-slot="toggle-group-item"
      className={cn(
        'motion-control flex h-10 items-center gap-1 rounded-md px-3 text-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[state=on]:bg-foreground data-[state=on]:text-background data-[state=on]:hover:text-background md:h-7 md:px-2.5',
        className,
      )}
      {...props}
    />
  );
}

export { ToggleGroup, ToggleGroupItem };
