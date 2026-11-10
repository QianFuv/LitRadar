/**
 * Project-styled Base UI tooltip with Animate UI anchored motion.
 */

import * as React from 'react';
import { Tooltip as TooltipPrimitive } from '@animate-ui/react/primitives/base-ui/tooltip';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

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
    <TooltipPrimitive.Provider delay={300}>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger render={children} />
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Positioner
            side="top"
            sideOffset={6}
            collisionPadding={0}
            collisionAvoidance={{ align: 'shift', fallbackAxisSide: 'none' }}
            {...stylex.props(styles.tooltipPositioner)}
          >
            <TooltipPrimitive.Popup
              data-slot="tooltip-content"
              {...stylex.props(styles.tooltipContent)}
            >
              {label}
            </TooltipPrimitive.Popup>
          </TooltipPrimitive.Positioner>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}

export { IconTooltip };

const styles = stylex.create({
  tooltipPositioner: {
    zIndex: 50,
  },
  tooltipContent: {
    borderRadius: '6px',
    backgroundColor: colors.foreground,
    paddingInline: '0.5rem',
    paddingBlock: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.background,
    boxShadow: shadows.overlay,
  },
});
