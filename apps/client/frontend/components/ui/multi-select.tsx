/**
 * Project-styled Base UI select primitives with Animate UI motion, for single or multiple values.
 */

import * as React from 'react';
import { Select as SelectPrimitive } from '@animate-ui/react/primitives/base-ui/select';
import { CheckIcon } from 'lucide-react';

import { PresenceBoundary } from '@/components/ui/motion';
import { RevealScope } from '@/components/ui/reveal';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

const MultiSelect = SelectPrimitive.Root;
const MultiSelectTrigger = SelectPrimitive.Trigger;
const MultiSelectValue = SelectPrimitive.Value;
const MultiSelectGroup = SelectPrimitive.Group;

type MultiSelectContentProps = React.ComponentProps<typeof SelectPrimitive.Popup> & {
  align?: 'start' | 'center' | 'end';
  sideOffset?: number;
};

/**
 * Render the positioned option popup in a portal with its own reveal surface.
 *
 * @param props - Popup properties, alignment, and offset from the trigger.
 * @returns Portaled select popup.
 */
function MultiSelectContent({
  align = 'start',
  sideOffset = 6,
  xstyle,
  children,
  ...props
}: Omit<MultiSelectContentProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Positioner
        align={align}
        sideOffset={sideOffset}
        alignItemWithTrigger={false}
        {...stylex.props(styles.multiSelectContentSelectPrimitivePositioner)}
      >
        <PresenceBoundary>
          <RevealScope>
            <SelectPrimitive.Popup {...stylex.props(styles.listPopup, xstyle)} {...props}>
              {children}
            </SelectPrimitive.Popup>
          </RevealScope>
        </PresenceBoundary>
      </SelectPrimitive.Positioner>
    </SelectPrimitive.Portal>
  );
}

/**
 * Render the scrollable option list.
 *
 * @param props - Base UI list properties.
 * @returns Scrollable option list.
 */
function MultiSelectList({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof SelectPrimitive.List>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <SelectPrimitive.List
      {...stylex.props(styles.multiSelectListSelectPrimitiveList, xstyle)}
      {...props}
    />
  );
}

/**
 * Render one group heading.
 *
 * @param props - Base UI group label properties.
 * @returns Group heading.
 */
function MultiSelectGroupLabel({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof SelectPrimitive.GroupLabel>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <SelectPrimitive.GroupLabel
      {...stylex.props(styles.multiSelectGroupLabelSelectPrimitiveGroupLabel, xstyle)}
      {...props}
    />
  );
}

type MultiSelectItemProps = React.ComponentProps<typeof SelectPrimitive.Item> & {
  /** Trailing detail such as a count, kept out of the item's typeahead text. */
  detail?: React.ReactNode;
};

/**
 * Render one option with its animated check indicator.
 *
 * @param props - Base UI item properties, label, and optional trailing detail.
 * @returns Selectable option row.
 */
function MultiSelectItem({
  xstyle,
  children,
  detail,
  ...props
}: Omit<MultiSelectItemProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <SelectPrimitive.Item {...stylex.props(styles.listItem, xstyle)} {...props}>
      <SelectPrimitive.ItemIndicator
        {...stylex.props(styles.multiSelectItemSelectPrimitiveItemIndicator)}
      >
        <CheckIcon {...stylex.props(styles.multiSelectItemCheckIcon)} aria-hidden="true" />
      </SelectPrimitive.ItemIndicator>
      <SelectPrimitive.ItemText {...stylex.props(styles.multiSelectItemSelectPrimitiveItemText)}>
        {children}
      </SelectPrimitive.ItemText>
      {detail !== undefined && (
        <span {...stylex.props(styles.multiSelectItemSpan)}>
          <span {...stylex.props(styles.multiSelectItemSpan2)}> · </span>
          {detail}
        </span>
      )}
    </SelectPrimitive.Item>
  );
}

export {
  MultiSelect,
  MultiSelectContent,
  MultiSelectGroup,
  MultiSelectGroupLabel,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectTrigger,
  MultiSelectValue,
};

const styles = stylex.create({
  multiSelectContentSelectPrimitivePositioner: {
    zIndex: 50,
  },
  listPopup: {
    display: 'flex',
    maxHeight: 'var(--available-height)',
    width: 'var(--anchor-width)',
    minWidth: '14rem',
    transformOrigin: 'var(--transform-origin)',
    flexDirection: 'column',
    overflow: 'hidden',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
  },
  multiSelectListSelectPrimitiveList: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    paddingBlock: '0.375rem',
    paddingRight: '0.75rem',
    paddingLeft: '0.375rem',
  },
  multiSelectGroupLabelSelectPrimitiveGroupLabel: {
    paddingInline: '0.625rem',
    paddingTop: '0.5rem',
    paddingBottom: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    color: colors.mutedForeground,
  },
  listItem: {
    position: 'relative',
    display: 'flex',
    minHeight: {
      default: '2.5rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    width: '100%',
    cursor: 'default',
    alignItems: 'center',
    gap: '0.5rem',
    borderRadius: '8px',
    paddingInline: '0.625rem',
    paddingBlock: '0.5rem',
    fontSize: '13px',
    lineHeight: '1.25rem',
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    userSelect: 'none',
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accent,
      },
      ':is([data-highlighted])': colors.accent,
    },
    pointerEvents: {
      default: null,
      ':is([data-disabled])': 'none',
    },
    opacity: {
      default: null,
      ':is([data-disabled])': '50%',
    },
    fontWeight: {
      default: null,
      ':is([data-selected])': 700,
    },
  },
  multiSelectItemSelectPrimitiveItemIndicator: {
    display: 'flex',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
  },
  multiSelectItemCheckIcon: {
    width: '1rem',
    height: '1rem',
  },
  multiSelectItemSelectPrimitiveItemText: {
    minWidth: '0px',
    flex: '1',
    overflowWrap: 'break-word',
  },
  multiSelectItemSpan: {
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 400,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  multiSelectItemSpan2: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
});

/** Popup and option styles shared with the combobox. */
export const listStyles = { popup: styles.listPopup, item: styles.listItem };
