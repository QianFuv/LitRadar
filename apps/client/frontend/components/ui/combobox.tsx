/**
 * Project-styled Base UI combobox primitives with Animate UI motion: a trigger opens a popup whose
 * search input filters the option list.
 */

import * as React from 'react';
import { Combobox as ComboboxPrimitive } from '@animate-ui/react/primitives/base-ui/combobox';
import { CheckIcon, Search } from 'lucide-react';

import { listStyles } from '@/components/ui/multi-select';
import { PresenceBoundary } from '@/components/ui/motion';
import { RevealScope } from '@/components/ui/reveal';
import { popupLayerStyle } from '@/components/ui/popup-layer';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

const Combobox = ComboboxPrimitive.Root;
const ComboboxTrigger = ComboboxPrimitive.Trigger;
/** Read the root's items after filtering, in display order; used to render virtualized lists. */
const useComboboxFilteredItems = ComboboxPrimitive.useFilteredItems;
type ComboboxContentProps = React.ComponentProps<typeof ComboboxPrimitive.Popup> & {
  align?: 'start' | 'center' | 'end';
  sideOffset?: number;
};

/**
 * Render the positioned popup in a portal with its own reveal surface.
 *
 * @param props - Popup properties, alignment, and offset from the trigger.
 * @returns Portaled combobox popup.
 */
function ComboboxContent({
  align = 'start',
  sideOffset = 6,
  xstyle,
  children,
  ...props
}: Omit<ComboboxContentProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <ComboboxPrimitive.Portal {...stylex.props(popupLayerStyle)}>
      <ComboboxPrimitive.Positioner
        align={align}
        sideOffset={sideOffset}
        {...stylex.props(styles.comboboxContentComboboxPrimitivePositioner)}
      >
        <PresenceBoundary>
          <RevealScope>
            <ComboboxPrimitive.Popup {...stylex.props(listStyles.popup, xstyle)} {...props}>
              {children}
            </ComboboxPrimitive.Popup>
          </RevealScope>
        </PresenceBoundary>
      </ComboboxPrimitive.Positioner>
    </ComboboxPrimitive.Portal>
  );
}

/**
 * Render the search field at the top of the popup.
 *
 * @param props - Base UI input properties.
 * @returns Search input with a leading icon.
 */
function ComboboxInput({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof ComboboxPrimitive.Input>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <div {...stylex.props(styles.comboboxInputDiv)}>
      <Search {...stylex.props(styles.comboboxInputSearch)} aria-hidden="true" />
      <ComboboxPrimitive.Input
        autoComplete="off"
        spellCheck={false}
        {...stylex.props(styles.comboboxInputComboboxPrimitiveInput, xstyle)}
        {...props}
      />
    </div>
  );
}

/**
 * Render the scrollable option list.
 *
 * @param props - Base UI list properties.
 * @returns Scrollable option list.
 */
function ComboboxList({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof ComboboxPrimitive.List>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <ComboboxPrimitive.List
      {...stylex.props(styles.comboboxListComboboxPrimitiveList, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the message shown when no option matches the search.
 *
 * @param props - Base UI empty-state properties.
 * @returns Empty-state message.
 */
function ComboboxEmpty({
  xstyle,
  ...props
}: Omit<React.ComponentProps<typeof ComboboxPrimitive.Empty>, 'className'> & {
  xstyle?: stylex.StyleXStyles;
}) {
  return (
    <ComboboxPrimitive.Empty
      {...stylex.props(styles.comboboxEmptyComboboxPrimitiveEmpty, xstyle)}
      {...props}
    />
  );
}

type ComboboxItemProps = React.ComponentProps<typeof ComboboxPrimitive.Item> & {
  /** Trailing detail such as a count. */
  detail?: React.ReactNode;
};

/**
 * Render one option with its animated check indicator.
 *
 * @param props - Base UI item properties, label, and optional trailing detail.
 * @returns Selectable option row.
 */
function ComboboxItem({
  xstyle,
  children,
  detail,
  ...props
}: Omit<ComboboxItemProps, 'className'> & { xstyle?: stylex.StyleXStyles }) {
  return (
    <ComboboxPrimitive.Item {...stylex.props(listStyles.item, xstyle)} {...props}>
      <ComboboxPrimitive.ItemIndicator
        {...stylex.props(styles.comboboxItemComboboxPrimitiveItemIndicator)}
      >
        <CheckIcon {...stylex.props(styles.comboboxItemCheckIcon)} aria-hidden="true" />
      </ComboboxPrimitive.ItemIndicator>
      <span {...stylex.props(styles.comboboxItemSpan)}>{children}</span>
      {detail !== undefined && (
        <span {...stylex.props(styles.comboboxItemSpan2)}>
          <span {...stylex.props(styles.comboboxItemSpan3)}> · </span>
          {detail}
        </span>
      )}
    </ComboboxPrimitive.Item>
  );
}

export {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
  useComboboxFilteredItems,
};

const styles = stylex.create({
  comboboxContentComboboxPrimitivePositioner: {
    zIndex: 50,
  },
  comboboxInputDiv: {
    position: 'relative',
    flexShrink: 0,
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    padding: '0.5rem',
  },
  comboboxInputSearch: {
    pointerEvents: 'none',
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    left: '1.125rem',
    width: '1rem',
    height: '1rem',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    color: colors.mutedForeground,
  },
  comboboxInputComboboxPrimitiveInput: {
    height: '2.25rem',
    width: '100%',
    minWidth: '0px',
    borderRadius: '8px',
    backgroundColor: 'transparent',
    paddingRight: '0.75rem',
    paddingLeft: '2.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    boxShadow: {
      default: shadows.vercelRing,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), ${shadows.vercelRing}`,
    },
    outlineStyle: 'none',
    '::placeholder': {
      color: colors.mutedForeground,
    },
  },
  comboboxListComboboxPrimitiveList: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    paddingBlock: '0.375rem',
    paddingRight: '0.75rem',
    paddingLeft: '0.375rem',
    display: {
      default: null,
      ':empty': 'none',
    },
  },
  comboboxEmptyComboboxPrimitiveEmpty: {
    paddingInline: '1rem',
    paddingBlock: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    display: {
      default: null,
      ':empty': 'none',
    },
  },
  comboboxItemComboboxPrimitiveItemIndicator: {
    display: 'flex',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
  },
  comboboxItemCheckIcon: {
    width: '1rem',
    height: '1rem',
  },
  comboboxItemSpan: {
    minWidth: '0px',
    flex: '1',
    overflowWrap: 'break-word',
  },
  comboboxItemSpan2: {
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 400,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  comboboxItemSpan3: {
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
