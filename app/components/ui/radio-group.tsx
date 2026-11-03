'use client';

/**
 * Project-styled Radix radio group with Animate UI indicator motion.
 */

import * as React from 'react';
import * as RadioGroupPrimitive from '@animate-ui/react/primitives/radix-ui/radio-group';
import { Check } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * Render the radio group root.
 *
 * @param props - Radix radio group properties.
 * @returns Radio group root.
 */
function RadioGroup({
  className,
  ...props
}: React.ComponentProps<typeof RadioGroupPrimitive.Root>) {
  return <RadioGroupPrimitive.Root data-slot="radio-group" className={className} {...props} />;
}

/**
 * Render one card-style choice whose animated check marks the selection.
 *
 * @param props - Radix radio item properties and the visible choice content.
 * @returns Selectable choice.
 */
function RadioGroupCard({
  className,
  children,
  ...props
}: React.ComponentProps<typeof RadioGroupPrimitive.Item>) {
  return (
    <RadioGroupPrimitive.Item
      data-slot="radio-group-card"
      className={cn(
        'motion-control flex items-center justify-between gap-2 rounded-lg px-3 text-left text-[13px] shadow-[inset_0_0_0_1px_var(--border)] outline-none transition-[background-color,box-shadow] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-info data-[state=checked]:font-bold data-[state=checked]:text-info-foreground data-[state=checked]:shadow-[inset_0_0_0_1px_var(--primary)] data-[state=checked]:hover:bg-info disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      {children}
      <RadioGroupPrimitive.Indicator className="flex size-3.5 shrink-0 items-center justify-center">
        <Check className="size-3.5" aria-hidden="true" />
      </RadioGroupPrimitive.Indicator>
    </RadioGroupPrimitive.Item>
  );
}

export { RadioGroup, RadioGroupCard };
