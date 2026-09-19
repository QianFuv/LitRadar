'use client';

/** Checkbox colors and feedback shared by filters and configuration forms. */

import * as React from 'react';
import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import { CheckIcon } from 'lucide-react';

import { cn } from '@/lib/utils';

/** Render checkbox borders and focus feedback inside the control so paint-contained lists cannot clip them. */
function Checkbox({ className, ...props }: React.ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        'motion-control peer bg-card data-[state=unchecked]:hover:bg-accent data-[state=checked]:hover:bg-primary-hover data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive size-4 shrink-0 rounded-[4px] shadow-[inset_0_0_0_1px_var(--input)] transition-[background-color,color,box-shadow] outline-none focus-visible:ring-[3px] focus-visible:ring-inset disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="grid place-content-center text-current transition-none"
      >
        <CheckIcon className="size-3.5" />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
