'use client';

/**
 * Caller-controlled destructive confirmation dialog.
 */

import { useRef, type ReactNode, type RefObject } from 'react';

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { buttonVariants } from '@/components/ui/button';

export type ConfirmDialogProps = {
  actionLabel?: string;
  cancelLabel?: string;
  description: ReactNode;
  error?: ReactNode;
  focusReturnRef?: RefObject<HTMLElement | null>;
  isPending?: boolean;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  pendingLabel?: string;
  title: ReactNode;
};

/**
 * Render a controlled destructive confirmation while the caller owns mutation state.
 *
 * The dialog remains open after its action until the caller explicitly closes it, so
 * asynchronous failures can remain visible and be retried without losing the target.
 *
 * @param props - Labels, content, external mutation state, and confirmation callback.
 * @returns Accessible destructive confirmation dialog.
 */
export function ConfirmDialog({
  actionLabel = '确认',
  cancelLabel = '取消',
  description,
  error,
  focusReturnRef,
  isPending = false,
  onConfirm,
  onOpenChange,
  open,
  pendingLabel = '处理中…',
  title,
}: ConfirmDialogProps) {
  const returnFocusRef = useRef<HTMLElement | null>(null);

  return (
    <AlertDialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!isPending || nextOpen) {
          onOpenChange(nextOpen);
        }
      }}
    >
      <AlertDialogContent
        onOpenAutoFocus={() => {
          if (document.activeElement instanceof HTMLElement) {
            returnFocusRef.current = document.activeElement;
          }
        }}
        onCloseAutoFocus={(event) => {
          const focusTarget = focusReturnRef?.current ?? returnFocusRef.current;
          if (focusTarget) {
            event.preventDefault();
            focusTarget.focus();
          }
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isPending}>{cancelLabel}</AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: 'destructive' })}
            disabled={isPending}
            onClick={(event) => {
              event.preventDefault();
              if (!isPending) {
                onConfirm();
              }
            }}
          >
            {isPending ? pendingLabel : actionLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
