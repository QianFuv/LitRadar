/**
 * Project-styled Animate UI AlertDialog primitives on Base UI.
 */

import * as React from 'react';
import { AlertDialog as AlertDialogPrimitive } from '@animate-ui/react/primitives/base-ui/alert-dialog';

import { buttonStyles } from '@/components/ui/button';
import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Render the AlertDialog state root.
 *
 * @param props - Base UI AlertDialog root properties.
 * @returns AlertDialog root.
 */
function AlertDialog(props: React.ComponentProps<typeof AlertDialogPrimitive.Root>) {
  return <AlertDialogPrimitive.Root {...props} />;
}

/**
 * Render accessible modal confirmation content with its backdrop.
 *
 * @param props - Base UI AlertDialog popup properties, children, and style overrides.
 * @returns Portaled AlertDialog content.
 */
function AlertDialogContent({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof AlertDialogPrimitive.Popup>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <AlertDialogPrimitive.Portal data-slot="alert-dialog-portal">
      <AlertDialogPrimitive.Backdrop
        data-slot="alert-dialog-overlay"
        {...stylex.props(styles.alertDialogOverlay)}
      />
      <RevealScope>
        <AlertDialogPrimitive.Popup
          data-slot="alert-dialog-content"
          {...stylex.props(styles.alertDialogContent, xstyle)}
          {...props}
        />
      </RevealScope>
    </AlertDialogPrimitive.Portal>
  );
}

/**
 * Render the AlertDialog heading group.
 *
 * @param props - Header container properties.
 * @returns Styled heading group.
 */
function AlertDialogHeader({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div
      data-slot="alert-dialog-header"
      {...stylex.props(styles.alertDialogHeader, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the AlertDialog action group.
 *
 * @param props - Footer container properties.
 * @returns Styled action group.
 */
function AlertDialogFooter({
  xstyle,
  ...props
}: Omit<React.ComponentProps<'div'>, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <div
      data-slot="alert-dialog-footer"
      {...stylex.props(styles.alertDialogFooter, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the accessible AlertDialog title.
 *
 * @param props - Base UI AlertDialog title properties.
 * @returns Styled title.
 */
function AlertDialogTitle({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof AlertDialogPrimitive.Title>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <AlertDialogPrimitive.Title
      data-slot="alert-dialog-title"
      {...stylex.props(styles.alertDialogTitle, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the accessible AlertDialog description.
 *
 * @param props - Base UI AlertDialog description properties.
 * @returns Styled description.
 */
function AlertDialogDescription({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof AlertDialogPrimitive.Description>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <AlertDialogPrimitive.Description
      data-slot="alert-dialog-description"
      {...stylex.props(styles.alertDialogDescription, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the action that accepts the alert. It does not close the dialog by itself, so the caller
 * can keep the dialog open while the action is pending.
 *
 * @param props - Native button properties and style overrides.
 * @returns Styled action control.
 */
function AlertDialogAction({
  xstyle,
  type = 'button',
  ...props
}: Omit<React.ComponentProps<'button'>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <button
      type={type}
      {...stylex.props(buttonStyles(), xstyle)}
      data-slot="alert-dialog-action"
      {...props}
    />
  );
}

/**
 * Render the action that cancels the alert and closes the dialog.
 *
 * @param props - Base UI AlertDialog close properties and style overrides.
 * @returns Styled cancel control.
 */
function AlertDialogCancel({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof AlertDialogPrimitive.Close>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <AlertDialogPrimitive.Close
      {...stylex.props(buttonStyles('outline'), xstyle)}
      data-slot="alert-dialog-cancel"
      {...props}
    />
  );
}

export {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
};

const styles = stylex.create({
  alertDialogOverlay: {
    position: 'fixed',
    inset: '0px',
    zIndex: 50,
    backgroundColor: 'color-mix(in oklab, #000 50%, transparent)',
  },
  alertDialogContent: {
    position: 'fixed',
    zIndex: 50,
    display: 'grid',
    width: 'calc(100% - 2rem)',
    maxWidth: '32rem',
    gap: '1rem',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: colors.background,
    padding: '1.5rem',
    boxShadow: '0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)',
    outlineStyle: 'none',
    left: '50%',
    top: '50%',
    translate: '-50% -50%',
  },
  alertDialogHeader: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.5rem',
    textAlign: {
      default: 'center',
      '@media (min-width: 40rem)': 'left',
    },
  },
  alertDialogFooter: {
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
  alertDialogTitle: {
    fontSize: '1.125rem',
    lineHeight: lineHeights.textLg,
    fontWeight: 600,
  },
  alertDialogDescription: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
});
