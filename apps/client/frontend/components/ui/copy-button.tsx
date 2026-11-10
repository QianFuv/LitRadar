/**
 * Shared copy action built on Animate UI's copy button and its copy-to-check icon swap.
 */

import {
  CopyButton as AnimateCopyButton,
  CopyButtonIcon,
  type CopyButtonProps as AnimateCopyButtonProps,
} from '@animate-ui/react/primitives/buttons/copy';
import type { ComponentProps, MouseEvent, ReactNode } from 'react';

import { Check, Copy, IconTriggerProvider, useIconTrigger } from '@/components/ui/animated-icons';
import * as stylex from '@stylexjs/stylex';

import {
  buttonStyles,
  getButtonPressMotion,
  type ButtonSize,
  type ButtonVariant,
} from '@/components/ui/button';
import { colors } from '@/lib/theme.stylex';

type CopyButtonProps = Omit<ComponentProps<'button'>, 'children' | 'className' | 'onClick'> & {
  children?: ReactNode;
  /** Icon styles that override the button's icon size. */
  iconXstyle?: stylex.StyleXStyles;
  isCopied: boolean;
  onCopy: () => void;
  size?: ButtonSize;
  variant?: ButtonVariant;
  xstyle?: stylex.StyleXStyles;
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
  xstyle,
  disabled,
  iconXstyle,
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
    ...stylex.props(buttonStyles(variant, size), xstyle),
    children: (
      <>
        <CopyButtonIcon
          icon={<Copy {...stylex.props(iconXstyle)} aria-hidden="true" />}
          copiedIcon={<Check {...stylex.props(iconXstyle, styles.copiedIcon)} aria-hidden="true" />}
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

const styles = stylex.create({
  copiedIcon: {
    color: colors.successForeground,
  },
});
