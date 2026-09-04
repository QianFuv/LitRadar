'use client';

/**
 * Project-styled Radix Dialog primitives with placement-aware motion.
 */

import * as React from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { XIcon } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/**
 * Render the Dialog state root.
 *
 * @param props - Radix Dialog root properties.
 * @returns Dialog root.
 */
function Dialog({ ...props }: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

/**
 * Render a control that opens its Dialog.
 *
 * @param props - Radix Dialog trigger properties.
 * @returns Dialog trigger.
 */
function DialogTrigger({ ...props }: React.ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

/**
 * Render Dialog content in a portal.
 *
 * @param props - Radix Dialog portal properties.
 * @returns Dialog portal.
 */
function DialogPortal({ ...props }: React.ComponentProps<typeof DialogPrimitive.Portal>) {
  return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
}

/**
 * Render a control that closes its Dialog.
 *
 * @param props - Radix Dialog close properties.
 * @returns Dialog close control.
 */
function DialogClose({ ...props }: React.ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

/**
 * Render the modal backdrop.
 *
 * @param props - Radix Dialog overlay properties.
 * @returns Styled Dialog overlay.
 */
function DialogOverlay({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Overlay>) {
  return (
    <DialogPrimitive.Overlay
      data-slot="dialog-overlay"
      className={cn('motion-overlay fixed inset-0 z-50 bg-black/50', className)}
      {...props}
    />
  );
}

type DialogContentProps = React.ComponentProps<typeof DialogPrimitive.Content> & {
  placement?: 'center' | 'left';
};

/**
 * Render accessible modal content with a centered or left-drawer placement.
 *
 * @param props - Radix Dialog content properties, children, and placement.
 * @returns Portaled Dialog content.
 */
function DialogContent({
  className,
  children,
  placement = 'center',
  ...props
}: DialogContentProps) {
  return (
    <DialogPortal data-slot="dialog-portal">
      <DialogOverlay />
      <DialogPrimitive.Content
        data-slot="dialog-content"
        data-motion-placement={placement}
        className={cn(
          'bg-background fixed z-50 grid gap-4 overscroll-contain border p-6 shadow-lg outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50',
          placement === 'center' &&
            'motion-dialog w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] rounded-lg md:max-w-4xl',
          placement === 'left' &&
            'motion-drawer inset-y-0 left-0 h-dvh w-80 max-w-[calc(100vw-2rem)] rounded-none border-r',
          className,
        )}
        {...props}
      >
        {children}
        {placement === 'center' && (
          <DialogPrimitive.Close data-slot="dialog-close" asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              static
              aria-label="关闭"
              className="absolute right-4 top-4 z-10 size-11 text-muted-foreground hover:text-foreground md:size-10"
            >
              <XIcon className="size-4" strokeWidth={1.5} aria-hidden="true" />
            </Button>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Content>
    </DialogPortal>
  );
}

/**
 * Render the Dialog heading group.
 *
 * @param props - Header container properties.
 * @returns Styled heading group.
 */
function DialogHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="dialog-header"
      className={cn('flex flex-col gap-2 pr-12 text-center sm:text-left', className)}
      {...props}
    />
  );
}

/**
 * Render the Dialog action group.
 *
 * @param props - Footer container properties.
 * @returns Styled action group.
 */
function DialogFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn('flex flex-col-reverse gap-2 sm:flex-row sm:justify-end', className)}
      {...props}
    />
  );
}

/**
 * Render the accessible Dialog title.
 *
 * @param props - Radix Dialog title properties.
 * @returns Styled title.
 */
function DialogTitle({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn('text-balance text-lg leading-snug font-semibold', className)}
      {...props}
    />
  );
}

/**
 * Render the accessible Dialog description.
 *
 * @param props - Radix Dialog description properties.
 * @returns Styled description.
 */
function DialogDescription({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn('text-pretty text-muted-foreground text-sm', className)}
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
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
