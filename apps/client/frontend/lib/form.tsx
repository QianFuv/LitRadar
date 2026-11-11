/**
 * TanStack Form setup with LitRadar field components.
 *
 * Forms call `useAppForm`, declare Zod validators, and render `form.AppField` children with the
 * field components below; each renders a Base UI `Field` around the shared input and, once the
 * field is edited or the form is submitted, shows its first validation message.
 */

import { createFormHook, createFormHookContexts } from '@tanstack/react-form';
import type * as stylex from '@stylexjs/stylex';
import type { ComponentProps } from 'react';
import { z } from 'zod';

import { Field, FieldError, NumberInput } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';

const { fieldContext, formContext, useFieldContext } = createFormHookContexts();

/** Placement styles of the field wrapper and its error message. */
type FieldLayoutProps = {
  fieldXstyle?: stylex.StyleXStyles;
  errorXstyle?: stylex.StyleXStyles;
};

/** A validation error: a message, or a schema issue carrying one. */
const FIELD_ERROR_SCHEMA = z.union([
  z.string().min(1),
  z.looseObject({ message: z.string().min(1) }).transform((issue) => issue.message),
]);

/**
 * Build a rule for text that must contain more than whitespace; the value itself stays untrimmed.
 *
 * @param message - Error shown while the text is blank.
 * @returns String schema.
 */
export function requiredText(message: string) {
  return z.string().refine((value) => value.trim().length > 0, message);
}

/**
 * Read the first message shown for a field: none until the user has edited it or tried to submit,
 * then the first of its validation errors, which are strings or schema issues.
 *
 * @param meta - Field meta with its touched state and errors, nested one level per validator.
 * @returns First message, or null when the field is valid or untouched.
 */
function visibleErrorMessage(meta: {
  isTouched: boolean;
  errors: readonly unknown[];
}): string | null {
  if (!meta.isTouched) {
    return null;
  }
  for (const error of meta.errors.flat()) {
    const parsed = FIELD_ERROR_SCHEMA.safeParse(error);
    if (parsed.success) {
      return parsed.data;
    }
  }
  return null;
}

type TextFieldProps = Omit<
  ComponentProps<typeof Input>,
  'value' | 'defaultValue' | 'onChange' | 'onValueChange' | 'onBlur'
> &
  FieldLayoutProps;

/**
 * Text input bound to a string form field.
 *
 * @param props - Input attributes and layout styles.
 * @returns Field with its input and validation message.
 */
function TextField({ fieldXstyle, errorXstyle, ...inputProps }: TextFieldProps) {
  const field = useFieldContext<string>();
  const error = visibleErrorMessage(field.state.meta);
  return (
    <Field invalid={error !== null} xstyle={fieldXstyle}>
      <Input
        {...inputProps}
        name={inputProps.name ?? field.name}
        value={field.state.value}
        onValueChange={(value) => field.handleChange(value)}
        onBlur={field.handleBlur}
      />
      {error !== null && <FieldError xstyle={errorXstyle}>{error}</FieldError>}
    </Field>
  );
}

type TextareaFieldProps = Omit<
  ComponentProps<typeof Textarea>,
  'value' | 'defaultValue' | 'onChange' | 'onValueChange' | 'onBlur'
> &
  FieldLayoutProps;

/**
 * Multiline input bound to a string form field.
 *
 * @param props - Textarea attributes and layout styles.
 * @returns Field with its textarea and validation message.
 */
function TextareaField({ fieldXstyle, errorXstyle, ...textareaProps }: TextareaFieldProps) {
  const field = useFieldContext<string>();
  const error = visibleErrorMessage(field.state.meta);
  return (
    <Field invalid={error !== null} xstyle={fieldXstyle}>
      <Textarea
        {...textareaProps}
        name={textareaProps.name ?? field.name}
        value={field.state.value}
        onValueChange={(value) => field.handleChange(value)}
        onBlur={field.handleBlur}
      />
      {error !== null && <FieldError xstyle={errorXstyle}>{error}</FieldError>}
    </Field>
  );
}

type NumberFieldProps = Omit<
  ComponentProps<typeof NumberInput>,
  'value' | 'defaultValue' | 'onValueChange'
> &
  FieldLayoutProps;

/**
 * Number input bound to a numeric form field, empty as null.
 *
 * @param props - Number field attributes and layout styles.
 * @returns Field with its number input and validation message.
 */
function NumberField({ fieldXstyle, errorXstyle, ...numberProps }: NumberFieldProps) {
  const field = useFieldContext<number | null>();
  const error = visibleErrorMessage(field.state.meta);
  return (
    <Field invalid={error !== null} xstyle={fieldXstyle}>
      <NumberInput
        {...numberProps}
        name={numberProps.name ?? field.name}
        value={field.state.value}
        onValueChange={(value) => field.handleChange(value)}
      />
      {error !== null && <FieldError xstyle={errorXstyle}>{error}</FieldError>}
    </Field>
  );
}

/** Form hook whose fields render the LitRadar field components. */
export const { useAppForm } = createFormHook({
  fieldContext,
  formContext,
  fieldComponents: { TextField, TextareaField, NumberField },
  formComponents: {},
});
