/**
 * Project-styled Radix Popover primitives with shared surface motion.
 */

import * as React from 'react';
import * as PopoverPrimitive from '@animate-ui/react/primitives/radix-ui/popover';

import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, shadows } from '@/lib/theme.stylex';

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
  xstyle,
  align = 'center',
  sideOffset = 4,
  ...props
}: Omit<React.ComponentProps<typeof PopoverPrimitive.Content>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  const content = (
    <PopoverPrimitive.Content
      data-slot="popover-content"
      align={align}
      sideOffset={sideOffset}
      {...stylex.props(styles.popoverContent, xstyle)}
      {...props}
    />
  );

  return (
    <PopoverPrimitive.Portal>
      <RevealScope>{content}</RevealScope>
    </PopoverPrimitive.Portal>
  );
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

const styles = stylex.create({
  popoverContent: {
    zIndex: 50,
    width: '18rem',
    transformOrigin: 'var(--radix-popover-content-transform-origin)',
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
