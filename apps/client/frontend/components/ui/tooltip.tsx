/**
 * Project-styled Radix tooltip with Animate UI anchored motion.
 */

import * as React from 'react';
import * as TooltipPrimitive from '@animate-ui/react/primitives/radix-ui/tooltip';

type IconTooltipProps = {
  /** Trigger element, usually an icon-only button that already has an accessible name. */
  children: React.ReactElement;
  /** Visible hint shown on hover and keyboard focus. */
  label: React.ReactNode;
};

/**
 * Show a short hint beside an icon-only control on hover and focus.
 *
 * @param props - Hint text and the control it describes.
 * @returns Control wrapped with an animated tooltip.
 */
function IconTooltip({ children, label }: IconTooltipProps) {
  return (
    <TooltipPrimitive.Provider delayDuration={300}>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Content
            data-slot="tooltip-content"
            sideOffset={6}
            className="z-50 rounded-md bg-foreground px-2 py-1 text-xs text-background shadow-overlay"
          >
            {label}
          </TooltipPrimitive.Content>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}

export { IconTooltip };
