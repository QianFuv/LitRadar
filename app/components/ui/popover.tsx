'use client';

/**
 * Project-styled Radix Popover primitives with shared surface motion.
 */

import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';

import { cn } from '@/lib/utils';

/**
 * Render the Popover state root.
 *
 * @param props - Radix Popover root properties.
 * @returns Popover root.
 */
function Popover({ ...props }: React.ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root data-slot="popover" {...props} />;
}

/**
 * Render a control that opens its Popover.
 *
 * @param props - Radix Popover trigger properties.
 * @returns Popover trigger.
 */
function PopoverTrigger({ ...props }: React.ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

/**
 * Render positioned Popover content in a portal.
 *
 * @param props - Radix Popover content properties and positioning defaults.
 * @returns Portaled Popover content.
 */
function PopoverContent({
  className,
  align = 'center',
  sideOffset = 4,
  ...props
}: React.ComponentProps<typeof PopoverPrimitive.Content>) {
  const content = (
    <PopoverPrimitive.Content
      data-slot="popover-content"
      align={align}
      sideOffset={sideOffset}
      className={cn(
        'motion-popover bg-popover text-popover-foreground z-50 w-72 origin-(--radix-popover-content-transform-origin) rounded-md border p-4 shadow-md outline-hidden focus-visible:ring-[3px] focus-visible:ring-ring/50',
        className,
      )}
      {...props}
    />
  );

  return <PopoverPrimitive.Portal>{content}</PopoverPrimitive.Portal>;
}

/**
 * Render an optional custom positioning anchor.
 *
 * @param props - Radix Popover anchor properties.
 * @returns Popover anchor.
 */
function PopoverAnchor({ ...props }: React.ComponentProps<typeof PopoverPrimitive.Anchor>) {
  return <PopoverPrimitive.Anchor data-slot="popover-anchor" {...props} />;
}

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor };
