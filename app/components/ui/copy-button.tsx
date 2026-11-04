'use client';

/**
 * Shared copy action built on Animate UI's copy button and its copy-to-check icon swap.
 */

import {
  CopyButton as AnimateCopyButton,
  CopyButtonIcon,
  type CopyButtonProps as AnimateCopyButtonProps,
} from '@animate-ui/react/primitives/buttons/copy';
import type { VariantProps } from 'class-variance-authority';
import type { ComponentProps, MouseEvent, ReactNode } from 'react';

import { Check, Copy, IconTriggerProvider, useIconTrigger } from '@/components/ui/animated-icons';
import { buttonVariants, getButtonPressMotion } from '@/components/ui/button';
import { cn } from '@/lib/utils';

type CopyButtonProps = Omit<ComponentProps<'button'>, 'children' | 'onClick'> &
  VariantProps<typeof buttonVariants> & {
    children?: ReactNode;
    iconClassName?: string;
    isCopied: boolean;
    onCopy: () => void;
  };

/**
 * Render a styled copy button whose icon swaps to a check while the copy is confirmed.
 *
 * The caller owns the clipboard write so failures stay visible; Animate UI only drives the
 * press and the icon swap from the controlled `isCopied` state.
 *
 * @param props - Button styling, controlled copied state, copy action, and optional label.
 * @returns Copy button that presses on Animate UI's swap tween and swaps its icon on success.
 */
export function CopyButton({
  children,
  className,
  disabled,
  iconClassName = 'size-4',
  isCopied,
  onCopy,
  size = 'default',
  variant = 'default',
  ...props
}: CopyButtonProps) {
  const { isTriggered, triggerHandlers } = useIconTrigger<HTMLButtonElement>(props);
  const copyProps = {
    ...props,
    ...triggerHandlers,
    content: '',
    copied: isCopied,
    disabled,
    ...getButtonPressMotion(!disabled, size),
    'data-variant': variant,
    'data-size': size,
    className: cn(buttonVariants({ variant, size }), className),
    children: (
      <>
        <CopyButtonIcon
          icon={<Copy className={iconClassName} aria-hidden="true" />}
          copiedIcon={
            <Check className={cn(iconClassName, 'text-success-foreground')} aria-hidden="true" />
          }
        />
        {children}
      </>
    ),
    onClick: (event: MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      onCopy();
    },
  } as AnimateCopyButtonProps;

  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <AnimateCopyButton {...copyProps} />
    </IconTriggerProvider>
  );
}
