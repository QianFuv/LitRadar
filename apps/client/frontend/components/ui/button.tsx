/**
 * Shared button styles and polymorphic control primitive.
 */

import * as React from 'react';
import {
  Button as PressButton,
  type ButtonProps as PressButtonProps,
} from '@animate-ui/react/primitives/buttons/button';
import { transition } from '@animate-ui/react/utils/motion';

import { IconTriggerProvider, useIconTrigger } from '@/components/ui/animated-icons';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/** Visual treatment of a shared button. */
type ButtonVariant = keyof typeof VARIANT_STYLES;

/** Size of a shared button. */
type ButtonSize = keyof typeof SIZE_STYLES;

/**
 * Press depth per button size: Animate UI's 0.97 on regular buttons, deeper on small ones so
 * the dip stays visible instead of shrinking a few tenths of a pixel.
 */
const BUTTON_TAP_SCALES: Record<ButtonSize, number> = {
  default: 0.97,
  lg: 0.97,
  sm: 0.95,
  xs: 0.9,
  icon: 0.9,
  'icon-xs': 0.9,
  'icon-sm': 0.9,
  'icon-lg': 0.9,
};

/** Front-loaded press tween without overshoot, so even quick clicks show the dip. */
const BUTTON_PRESS_TRANSITION = transition.swap;

/**
 * Build the press motion shared by every button, replacing Animate UI's transform-string press.
 *
 * Animate UI animates a full `transform` string whose `scale(1)` start is never rendered, so the
 * first press starts from `none` (read as `scale(0)`) and visibly collapses. The independent
 * `scale` value always starts at 1.
 *
 * @param hasPress - Whether the control is pressable right now.
 * @param size - Button size that selects the press depth.
 * @returns Motion properties to spread onto Animate UI's button primitive.
 */
function getButtonPressMotion(hasPress: boolean, size: ButtonSize | null = 'default') {
  return {
    initial: false,
    whileTap: hasPress ? { scale: BUTTON_TAP_SCALES[size ?? 'default'] } : undefined,
    transition: BUTTON_PRESS_TRANSITION,
  } as const;
}

/**
 * Build the styles of a shared button for one variant and size, so other controls can look like
 * buttons.
 *
 * @param variant - Visual treatment.
 * @param size - Control size.
 * @returns Styles to pass to `stylex.props` or an `xstyle` prop.
 */
function buttonStyles(variant: ButtonVariant = 'default', size: ButtonSize = 'default') {
  return [styles.base, VARIANT_STYLES[variant], SIZE_STYLES[size]];
}

/**
 * Render a shared button or pass its behavior to one child element.
 *
 * @param props - Native properties, styling, polymorphic mode, and optional static press behavior.
 * @returns Styled interactive control that presses down on Animate UI's swap tween and plays its
 * animated icons while hovered or pressed.
 */
function Button({
  xstyle,
  variant = 'default',
  size = 'default',
  asChild = false,
  static: isStatic = false,
  disabled,
  ...props
}: Omit<React.ComponentProps<'button'>, 'className'> & {
  variant?: ButtonVariant | null;
  size?: ButtonSize | null;
  asChild?: boolean;
  static?: boolean;
  xstyle?: stylex.StyleXStyles;
}) {
  const hasPress = !isStatic && !disabled && variant !== 'link';
  const { isTriggered, triggerHandlers } = useIconTrigger(props);

  const pressProps = {
    ...props,
    ...triggerHandlers,
    asChild,
    ...getButtonPressMotion(hasPress, size),
    'data-variant': variant,
    'data-size': size,
    ...stylex.props(buttonStyles(variant ?? 'default', size ?? 'default'), xstyle),
    disabled,
  } as PressButtonProps;

  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <PressButton {...pressProps} />
    </IconTriggerProvider>
  );
}

export { Button, buttonStyles, getButtonPressMotion, type ButtonSize, type ButtonVariant };

const styles = stylex.create({
  base: {
    display: 'inline-flex',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    gap: '0.5rem',
    borderRadius: '8px',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 500,
    whiteSpace: 'nowrap',
    transitionProperty: 'background-color, border-color, color, box-shadow, opacity',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}`,
      },
    },
    pointerEvents: {
      default: null,
      ':disabled': 'none',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    borderColor: {
      default: null,
      ':is([aria-invalid="true"])': colors.destructive,
    },
  },
  variantDefault: {
    backgroundColor: {
      default: colors.primary,
      ':active': {
        default: colors.primaryPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.primaryPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.primaryHover,
      },
    },
    color: colors.primaryForeground,
  },
  variantDestructive: {
    backgroundColor: {
      default: colors.destructive,
      ':active': colors.destructiveHover,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructiveHover,
      },
    },
    color: colors.destructiveForeground,
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px ${colors.destructiveRing}`,
    },
  },
  variantOutline: {
    backgroundColor: {
      default: colors.card,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, ${shadows.vercelRing}`,
      },
    },
    color: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  variantSecondary: {
    backgroundColor: {
      default: colors.secondary,
      ':active': colors.accentPressed,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentPressed,
      },
    },
    color: colors.secondaryForeground,
  },
  variantGhost: {
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    color: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  variantLink: {
    color: {
      default: colors.primaryText,
      ':active': `color-mix(in oklab, ${colors.primaryText} 80%, transparent)`,
    },
    textUnderlineOffset: '4px',
    textDecorationLine: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': 'underline',
      },
    },
  },
  sizeDefault: {
    height: '2.25rem',
    paddingInline: {
      default: '1rem',
      ':has(> svg)': '0.75rem',
    },
    paddingBlock: '0.5rem',
  },
  sizeXs: {
    height: '1.5rem',
    gap: '0.25rem',
    borderRadius: '6px',
    paddingInline: {
      default: '0.5rem',
      ':has(> svg)': '0.375rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  sizeSm: {
    height: '2rem',
    gap: '0.375rem',
    paddingInline: {
      default: '0.75rem',
      ':has(> svg)': '0.625rem',
    },
  },
  sizeLg: {
    height: '2.5rem',
    paddingInline: {
      default: '1.5rem',
      ':has(> svg)': '1rem',
    },
  },
  sizeIcon: {
    width: '2.25rem',
    height: '2.25rem',
  },
  sizeIconXs: {
    width: '1.5rem',
    height: '1.5rem',
    borderRadius: '6px',
  },
  sizeIconSm: {
    width: '2rem',
    height: '2rem',
  },
  sizeIconLg: {
    width: '2.5rem',
    height: '2.5rem',
  },
});

const VARIANT_STYLES = {
  default: styles.variantDefault,
  destructive: styles.variantDestructive,
  outline: styles.variantOutline,
  secondary: styles.variantSecondary,
  ghost: styles.variantGhost,
  link: styles.variantLink,
};

const SIZE_STYLES = {
  default: styles.sizeDefault,
  xs: styles.sizeXs,
  sm: styles.sizeSm,
  lg: styles.sizeLg,
  icon: styles.sizeIcon,
  'icon-xs': styles.sizeIconXs,
  'icon-sm': styles.sizeIconSm,
  'icon-lg': styles.sizeIconLg,
};
