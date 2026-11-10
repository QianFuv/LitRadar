/**
 * Project-styled Animate UI Popover primitives on Base UI with shared surface motion.
 */

import * as React from 'react';
import { Popover as PopoverPrimitive } from '@animate-ui/react/primitives/base-ui/popover';

import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, shadows } from '@/lib/theme.stylex';

/**
 * Render the Popover state root.
 *
 * @param props - Base UI Popover root properties.
 * @returns Popover root.
 */
function Popover(props: React.ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root {...props} />;
}

/**
 * Render a control that opens its Popover.
 *
 * @param props - Base UI Popover trigger properties.
 * @returns Popover trigger.
 */
function PopoverTrigger(props: React.ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

type PopoverContentProps = Omit<React.ComponentProps<typeof PopoverPrimitive.Popup>, 'className'> &
  Pick<
    React.ComponentProps<typeof PopoverPrimitive.Positioner>,
    'align' | 'side' | 'sideOffset' | 'anchor'
  > & {
    xstyle?: stylex.StyleXStyles;
  };

/**
 * Render positioned Popover content in a portal.
 *
 * @param props - Base UI Popover popup properties, positioning, and style overrides.
 * @returns Portaled Popover content.
 */
function PopoverContent({
  xstyle,
  align = 'center',
  side = 'bottom',
  sideOffset = 4,
  anchor,
  ...props
}: PopoverContentProps) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Positioner
        align={align}
        side={side}
        sideOffset={sideOffset}
        anchor={anchor}
        collisionPadding={0}
        collisionAvoidance={{ align: 'shift', fallbackAxisSide: 'none' }}
        {...stylex.props(styles.popoverPositioner)}
      >
        <RevealScope>
          <PopoverPrimitive.Popup
            data-slot="popover-content"
            {...stylex.props(styles.popoverContent, xstyle)}
            {...props}
          />
        </RevealScope>
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverTrigger, PopoverContent };

const styles = stylex.create({
  popoverPositioner: {
    zIndex: 50,
  },
  popoverContent: {
    width: '18rem',
    transformOrigin: 'var(--transform-origin)',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    padding: '1rem',
    color: colors.popoverForeground,
    boxShadow: {
      default: shadows.overlay,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.overlay}`,
    },
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
  },
});
