/** Shared single-line input with theme-aware pointer and keyboard feedback. */

import * as React from 'react';

import { cn } from '@/lib/utils';

/** Render an input while preserving native fields, sizing, and invalid states. */
function Input({ className, type, ...props }: React.ComponentProps<'input'>) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        'file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground hover:bg-accent/50 hover:shadow-[0_0_0_1px_var(--input-hover)] h-9 w-full min-w-0 rounded-md shadow-vercel-ring bg-card px-3 py-1 text-base motion-control transition-[background-color,color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-card file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm',
        'focus-visible:ring-ring/50 focus-visible:ring-[3px]',
        'aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive',
        className,
      )}
      {...props}
    />
  );
}

export { Input };
