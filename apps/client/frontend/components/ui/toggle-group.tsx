/**
 * Project-styled Radix toggle group whose selected surface slides between items (Animate UI).
 */

import * as React from 'react';
import * as ToggleGroupPrimitive from '@animate-ui/react/primitives/radix-ui/toggle-group';

import { PresenceBoundary } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Render the segmented control root.
 *
 * @param props - Radix toggle group properties.
 * @returns Segmented control root.
 */
function ToggleGroup({
  xstyle,
  ...props
}: React.ComponentProps<typeof ToggleGroupPrimitive.Root> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      {...stylex.props(styles.toggleGroup, xstyle)}
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
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof ToggleGroupPrimitive.Item>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <PresenceBoundary>
      <ToggleGroupPrimitive.Item
        data-slot="toggle-group-item"
        {...stylex.props(styles.toggleGroupItem, xstyle)}
        {...props}
      />
    </PresenceBoundary>
  );
}

export { ToggleGroup, ToggleGroupItem };

const styles = stylex.create({
  toggleGroup: {
    display: 'flex',
    width: 'fit-content',
    alignItems: 'center',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    padding: '0.125rem',
  },
  toggleGroupItem: {
    display: 'flex',
    height: {
      default: '2.5rem',
      '@media (min-width: 48rem)': '1.75rem',
    },
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: '6px',
    paddingInline: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: {
      default: colors.mutedForeground,
      ':is([data-state="on"])': colors.background,
      ':hover': {
        default: null,
        ':is([data-state="on"])': null,
        '@media (hover: hover)': {
          default: colors.foreground,
          ':is([data-state="on"])': colors.background,
        },
      },
    },
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
    backgroundColor: {
      default: null,
      ':is([data-state="on"])': colors.foreground,
    },
  },
});
