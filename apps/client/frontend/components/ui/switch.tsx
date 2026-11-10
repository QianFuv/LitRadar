/**
 * Project-styled Base UI Switch with restrained state motion.
 */

import * as React from 'react';
import { Switch as SwitchPrimitive } from '@animate-ui/react/primitives/base-ui/switch';
import * as stylex from '@stylexjs/stylex';
import { switchMarker } from '@/lib/markers.stylex';
import { colors, shadows } from '@/lib/theme.stylex';

/**
 * Render a binary control with a sliding state indicator.
 *
 * @param props - Base UI Switch properties, control size, and style overrides.
 * @returns Styled Switch control.
 */
function Switch({
  xstyle,
  size = 'default',
  ...props
}: Omit<
  React.ComponentProps<typeof SwitchPrimitive.Root> & {
    size?: 'sm' | 'default';
  },
  'className'
> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <SwitchPrimitive.Root
      nativeButton
      data-slot="switch"
      data-size={size}
      {...stylex.props(styles.switch, stylex.defaultMarker(), switchMarker, xstyle)}
      {...props}
    >
      <SwitchPrimitive.Thumb data-slot="switch-thumb" {...stylex.props(styles.switchThumb)} />
    </SwitchPrimitive.Root>
  );
}

export { Switch };

const styles = stylex.create({
  switch: {
    display: 'inline-flex',
    flexShrink: 0,
    alignItems: 'center',
    borderRadius: 'calc(infinity * 1px)',
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
    },
    transitionProperty: 'background-color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    cursor: {
      default: null,
      ':disabled': 'not-allowed',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    height: {
      default: null,
      ':is([data-size="default"])': '1.25rem',
      ':is([data-size="sm"])': '0.875rem',
    },
    width: {
      default: null,
      ':is([data-size="default"])': '2.25rem',
      ':is([data-size="sm"])': '1.5rem',
    },
    backgroundColor: {
      default: null,
      ':is([data-checked])': colors.primary,
      ':hover': {
        default: null,
        ':is([data-checked])': null,
        '@media (hover: hover)': {
          default: null,
          ':is([data-checked])': colors.primaryHover,
          ':is([data-unchecked])': colors.inputHover,
        },
        ':is([data-unchecked])': null,
      },
      ':is([data-unchecked])': colors.input,
    },
  },
  switchThumb: {
    pointerEvents: 'none',
    display: 'block',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.primaryForeground,
    boxShadow: '0 0 0 0px currentcolor',
    width: {
      default: null,
      [stylex.when.ancestor('[data-size="default"]', switchMarker)]: '1.125rem',
      [stylex.when.ancestor('[data-size="sm"]', switchMarker)]: '0.75rem',
    },
    height: {
      default: null,
      [stylex.when.ancestor('[data-size="default"]', switchMarker)]: '1.125rem',
      [stylex.when.ancestor('[data-size="sm"]', switchMarker)]: '0.75rem',
    },
    translate: {
      default: null,
      ':is([data-checked])': 'calc(100% - 2px) 0',
      ':is([data-unchecked])': '0px 0',
    },
  },
});
