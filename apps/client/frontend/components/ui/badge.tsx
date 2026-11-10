/**
 * Compact status badge with semantic color variants.
 */

import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import * as stylex from '@stylexjs/stylex';

import { colors, shadows } from '@/lib/theme.stylex';

/** Semantic color treatment of a badge. */
type BadgeVariant = keyof typeof VARIANT_STYLES;

/**
 * Render a compact status badge, or pass its styling to one child element.
 *
 * @param props - Native span properties, variant, polymorphic mode, and style overrides.
 * @returns Badge element.
 */
function Badge({
  xstyle,
  variant = 'default',
  asChild = false,
  ...props
}: Omit<React.ComponentProps<'span'>, 'className'> & {
  variant?: BadgeVariant;
  asChild?: boolean;
  xstyle?: stylex.StyleXStyles;
}) {
  const Comp = asChild ? Slot : 'span';

  return (
    <Comp
      data-slot="badge"
      data-variant={variant}
      {...stylex.props(styles.base, VARIANT_STYLES[variant], xstyle)}
      {...props}
    />
  );
}

export { Badge, type BadgeVariant };

const styles = stylex.create({
  base: {
    display: 'inline-flex',
    width: 'fit-content',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    gap: '0.25rem',
    overflow: 'hidden',
    borderRadius: 'calc(infinity * 1px)',
    paddingInline: '0.5rem',
    paddingBlock: '0.125rem',
    fontSize: '11px',
    lineHeight: '1rem',
    fontWeight: 500,
    whiteSpace: 'nowrap',
    transitionProperty: 'color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.4, 0, 0.2, 1)',
    transitionDuration: '150ms',
    boxShadow: {
      default: null,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}`,
      },
    },
    borderColor: {
      default: null,
      ':is([aria-invalid="true"])': colors.destructive,
    },
  },
  variantDefault: {
    backgroundColor: colors.info,
    color: colors.infoForeground,
  },
  variantSecondary: {
    backgroundColor: colors.secondary,
    color: colors.secondaryForeground,
  },
  variantSuccess: {
    backgroundColor: colors.success,
    color: colors.successForeground,
  },
  variantWarning: {
    backgroundColor: colors.warning,
    color: colors.warningForeground,
  },
  variantDestructive: {
    backgroundColor: colors.destructive,
    color: colors.destructiveForeground,
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px ${colors.destructiveRing}`,
    },
  },
  variantOutline: {
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, ${shadows.vercelRing}`,
      },
    },
    color: colors.foreground,
  },
  variantGhost: {},
  variantLink: {
    color: colors.primaryText,
    textUnderlineOffset: '4px',
  },
});

const VARIANT_STYLES = {
  default: styles.variantDefault,
  secondary: styles.variantSecondary,
  success: styles.variantSuccess,
  warning: styles.variantWarning,
  destructive: styles.variantDestructive,
  outline: styles.variantOutline,
  ghost: styles.variantGhost,
  link: styles.variantLink,
};
