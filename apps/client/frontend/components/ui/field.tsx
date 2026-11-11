/**
 * Base UI field parts and number input styled for LitRadar forms.
 *
 * A `Field` links its control, description, and error for assistive technology; the error text
 * comes from the form library, so `FieldError` always shows what it is given.
 */

import { Field as BaseField } from '@base-ui/react/field';
import { NumberField as BaseNumberField } from '@base-ui/react/number-field';
import * as stylex from '@stylexjs/stylex';

import { inputStyle } from '@/components/ui/input';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Group one control with its description and error.
 *
 * @param props - Base UI field root props and optional styles.
 * @returns Field container.
 */
export function Field({
  xstyle,
  ...props
}: Omit<BaseField.Root.Props, 'className' | 'style'> & { xstyle?: stylex.StyleXStyles }) {
  return <BaseField.Root {...stylex.props(styles.field, xstyle)} {...props} />;
}

/**
 * Show a validation message linked to the enclosing field's control.
 *
 * @param props - Message content and optional styles.
 * @returns Error message announced with the control.
 */
export function FieldError({
  xstyle,
  ...props
}: Omit<BaseField.Error.Props, 'className' | 'style' | 'match'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return <BaseField.Error match role="alert" {...stylex.props(styles.error, xstyle)} {...props} />;
}

/** Show numbers without digit grouping, since these fields hold counts, limits, and seconds. */
const PLAIN_NUMBER_FORMAT: Intl.NumberFormatOptions = { useGrouping: false };

/**
 * Render a numeric text input with Base UI number parsing, stepping, and range clamping.
 *
 * @param props - Base UI number field props, the input id and placeholder, and optional styles.
 * @returns Number field.
 */
export function NumberInput({
  id,
  placeholder,
  xstyle,
  ...props
}: Omit<BaseNumberField.Root.Props, 'className' | 'style' | 'id'> & {
  id?: string;
  placeholder?: string;
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <BaseNumberField.Root format={PLAIN_NUMBER_FORMAT} {...props}>
      <BaseNumberField.Group>
        <BaseNumberField.Input
          id={id}
          placeholder={placeholder}
          {...stylex.props(inputStyle, xstyle)}
        />
      </BaseNumberField.Group>
    </BaseNumberField.Root>
  );
}

const styles = stylex.create({
  field: {
    minWidth: 0,
  },
  error: {
    marginTop: '0.375rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.destructive,
  },
});
