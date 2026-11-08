/**
 * Shared multiline text input primitive.
 */

import * as React from 'react';

import { cn } from '@/lib/utils';

/**
 * Render a multiline input with the same states and responsive text sizing as Input.
 *
 * @param props - Native textarea props and optional class names.
 * @returns Styled textarea element.
 */
function Textarea({ className, ...props }: React.ComponentProps<'textarea'>) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        'placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground hover:bg-accent/50 hover:shadow-[0_0_0_1px_var(--input-hover)] min-h-20 w-full min-w-0 resize-y rounded-md bg-card px-3 py-2 text-base shadow-vercel-ring motion-control transition-[background-color,color,box-shadow] outline-none disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm',
        'focus-visible:ring-ring/50 focus-visible:ring-[3px]',
        'aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive',
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
