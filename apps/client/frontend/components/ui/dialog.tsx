/**
 * Project-styled Animate UI Dialog primitives on Base UI: centered dialogs tip forward, edge
 * placements slide in.
 */

import * as React from 'react';
import { Dialog as DialogPrimitive } from '@animate-ui/react/primitives/base-ui/dialog';
import { X as XIcon } from '@/components/ui/animated-icons';

import { Button } from '@/components/ui/button';
import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/**
 * Render the Dialog state root.
 *
 * @param props - Base UI Dialog root properties.
 * @returns Dialog root.
 */
function Dialog(props: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root {...props} />;
}

/**
 * Render a control that opens its Dialog.
 *
 * @param props - Base UI Dialog trigger properties.
 * @returns Dialog trigger.
 */
function DialogTrigger(props: React.ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

/**
 * Render a control that closes its Dialog.
 *
 * @param props - Base UI Dialog close properties.
 * @returns Dialog close control.
 */
function DialogClose(props: React.ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

type DialogPlacement = 'center' | 'left' | 'right' | 'bottom' | 'takeover';

type DialogContentProps = Omit<
  React.ComponentProps<typeof DialogPrimitive.Popup>,
  'className' | 'from'
> & {
  placement?: DialogPlacement;
  xstyle?: stylex.StyleXStyles;
};

/**
 * Screen edge each placement slides from; centered dialogs have none and tip forward instead.
 */
const PLACEMENT_SIDES: Record<DialogPlacement, 'left' | 'right' | 'bottom' | undefined> = {
  center: undefined,
  left: 'left',
  right: 'right',
  bottom: 'bottom',
  takeover: 'bottom',
};

/**
 * Render accessible modal content as a centered dialog, side sheet, bottom sheet, or takeover.
 *
 * @param props - Base UI Dialog popup properties, children, placement, and style overrides.
 * @returns Portaled Dialog content with its backdrop.
 */
function DialogContent({ xstyle, children, placement = 'center', ...props }: DialogContentProps) {
  return (
    <DialogPrimitive.Portal data-slot="dialog-portal">
      <DialogPrimitive.Backdrop
        data-slot="dialog-overlay"
        {...stylex.props(styles.dialogOverlay)}
      />
      <RevealScope>
        <DialogPrimitive.Popup
          data-slot="dialog-content"
          data-motion-placement={placement}
          data-side={PLACEMENT_SIDES[placement]}
          {...stylex.props(
            styles.dialogContent,
            placement === 'center' && styles.dialogContentCenter,
            placement === 'left' && styles.dialogContentLeft,
            placement === 'right' && styles.dialogContentRight,
            placement === 'bottom' && styles.dialogContentBottom,
            placement === 'takeover' && styles.dialogContentTakeover,
            xstyle,
          )}
          {...props}
        >
          {children}
          {placement === 'center' && (
            <DialogPrimitive.Close
              data-slot="dialog-close"
              render={
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  static
                  aria-label="关闭"
                  xstyle={styles.dialogContentButton}
                />
              }
            >
              <XIcon xstyle={styles.dialogContentXIcon} strokeWidth={1.5} aria-hidden="true" />
            </DialogPrimitive.Close>
          )}
        </DialogPrimitive.Popup>
      </RevealScope>
    </DialogPrimitive.Portal>
  );
}

/**
 * Render the Dialog heading group.
 *
 * @param props - Header container properties.
 * @returns Styled heading group.
 */
function DialogHeader({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div data-slot="dialog-header" {...stylex.props(styles.dialogHeader, xstyle)} {...props} />
  );
}

/**
 * Render the Dialog action group.
 *
 * @param props - Footer container properties.
 * @returns Styled action group.
 */
function DialogFooter({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div data-slot="dialog-footer" {...stylex.props(styles.dialogFooter, xstyle)} {...props} />
  );
}

/**
 * Render the accessible Dialog title.
 *
 * @param props - Base UI Dialog title properties and style overrides.
 * @returns Styled title.
 */
function DialogTitle({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof DialogPrimitive.Title>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      {...stylex.props(styles.dialogTitle, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the accessible Dialog description.
 *
 * @param props - Base UI Dialog description properties and style overrides.
 * @returns Styled description.
 */
function DialogDescription({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof DialogPrimitive.Description>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      {...stylex.props(styles.dialogDescription, xstyle)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
};

const styles = stylex.create({
  dialogOverlay: {
    position: 'fixed',
    inset: '0px',
    zIndex: 50,
    backgroundColor: 'color-mix(in oklab, #000 40%, transparent)',
  },
  dialogContent: {
    position: 'fixed',
    zIndex: 50,
    display: 'grid',
    gap: '1rem',
    overscrollBehavior: 'contain',
    backgroundColor: colors.background,
    padding: '1.5rem',
    boxShadow: {
      default: shadows.overlay,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.overlay}`,
    },
    outlineStyle: 'none',
  },
  dialogContentCenter: {
    width: 'calc(100% - 2rem)',
    maxWidth: {
      default: 'calc(100% - 2rem)',
      '@media (min-width: 48rem)': '36rem',
    },
    borderRadius: '16px',
    left: '50%',
    top: '50%',
    translate: '-50% -50%',
  },
  dialogContentLeft: {
    insetBlock: '0px',
    left: '0px',
    height: '100dvh',
    width: '20rem',
    maxWidth: 'calc(100vw - 2rem)',
    borderRadius: 0,
  },
  dialogContentRight: {
    insetBlock: '0px',
    right: '0px',
    height: '100dvh',
    width: {
      default: '100%',
      '@media (min-width: 48rem)': 'min(46rem, calc(100vw - 5rem))',
    },
    maxWidth: '100%',
    borderRadius: 0,
    padding: '0px',
  },
  dialogContentBottom: {
    insetInline: '0px',
    bottom: '0px',
    maxHeight: '85dvh',
    width: '100%',
    borderTopLeftRadius: '16px',
    borderTopRightRadius: '16px',
    borderBottomRightRadius: 0,
    borderBottomLeftRadius: 0,
    padding: '0px',
    paddingBottom: 'var(--safe-area-inset-bottom, env(safe-area-inset-bottom,0px))',
  },
  dialogContentTakeover: {
    inset: '0px',
    height: '100dvh',
    width: '100vw',
    maxWidth: 'none',
    borderRadius: 0,
    padding: '0px',
    boxShadow: {
      default: shadows.overlay,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.overlay}`,
    },
  },
  dialogContentButton: {
    position: 'absolute',
    top: '1rem',
    right: '1rem',
    zIndex: 10,
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
  },
  dialogContentXIcon: {
    width: '1rem',
    height: '1rem',
  },
  dialogHeader: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.5rem',
    paddingRight: '3rem',
    textAlign: {
      default: 'center',
      '@media (min-width: 40rem)': 'left',
    },
  },
  dialogFooter: {
    display: 'flex',
    flexDirection: {
      default: 'column-reverse',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.5rem',
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
  dialogTitle: {
    fontSize: '1.125rem',
    lineHeight: 1.375,
    fontWeight: 700,
    textWrap: 'balance',
  },
  dialogDescription: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    textWrap: 'pretty',
    color: colors.mutedForeground,
  },
});
