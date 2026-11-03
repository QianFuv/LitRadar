/**
 * Shared button styles and polymorphic control primitive.
 */

import * as React from 'react';
import {
  Button as PressButton,
  type ButtonProps as PressButtonProps,
} from '@animate-ui/react/primitives/buttons/button';
import { transition } from '@animate-ui/react/utils/motion';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '@/lib/utils';

type ButtonSize = NonNullable<VariantProps<typeof buttonVariants>['size']>;

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

/** Build the supported visual and size variants for shared buttons. */
const buttonVariants = cva(
  "motion-control inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg text-sm font-medium transition-[background-color,border-color,color,box-shadow,opacity] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:ring-ring/50 focus-visible:ring-[3px] aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
  {
    variants: {
      variant: {
        default:
          'bg-primary text-primary-foreground hover:bg-primary-hover active:bg-primary-pressed',
        destructive:
          'bg-destructive text-destructive-foreground hover:bg-destructive-hover active:bg-destructive-hover focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40',
        outline:
          'bg-card shadow-vercel-ring hover:bg-accent hover:text-accent-foreground active:bg-accent-pressed',
        secondary:
          'bg-secondary text-secondary-foreground hover:bg-accent-pressed active:bg-accent-pressed',
        ghost: 'hover:bg-accent hover:text-accent-foreground active:bg-accent-pressed',
        link: 'text-primary-text underline-offset-4 hover:underline active:text-primary-text/80',
      },
      size: {
        default: 'h-9 px-4 py-2 has-[>svg]:px-3',
        xs: "h-6 gap-1 rounded-md px-2 text-xs has-[>svg]:px-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: 'h-8 gap-1.5 px-3 has-[>svg]:px-2.5',
        lg: 'h-10 px-6 has-[>svg]:px-4',
        icon: 'size-9',
        'icon-xs': "size-6 rounded-md [&_svg:not([class*='size-'])]:size-3",
        'icon-sm': 'size-8',
        'icon-lg': 'size-10',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

/**
 * Render a shared button or pass its behavior to one child element.
 *
 * @param props - Native properties, styling, polymorphic mode, and optional static press behavior.
 * @returns Styled interactive control that presses down on Animate UI's swap tween.
 */
function Button({
  className,
  variant = 'default',
  size = 'default',
  asChild = false,
  static: isStatic = false,
  disabled,
  ...props
}: React.ComponentProps<'button'> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
    static?: boolean;
  }) {
  const hasPress = !isStatic && !disabled && variant !== 'link';

  const pressProps = {
    ...props,
    asChild,
    ...getButtonPressMotion(hasPress, size),
    'data-variant': variant,
    'data-size': size,
    className: cn(buttonVariants({ variant, size }), className),
    disabled,
  } as PressButtonProps;

  return <PressButton {...pressProps} />;
}

export { Button, buttonVariants, getButtonPressMotion };
