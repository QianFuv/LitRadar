/**
 * Shared filter pills: pill styles and labels, panel headers, popover facets, and database pills.
 */

import { ChevronDown, Layers } from '@/components/ui/animated-icons';
import type { ReactNode } from 'react';

import { Button } from '@/components/ui/button';
import { MotionPresence, MotionSpan, SWAP_VARIANTS } from '@/components/ui/motion';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Build the styles of one filter pill trigger.
 *
 * Pills stay open-styled for both Radix (`data-state`) and Base UI (`data-popup-open`) triggers.
 *
 * @param isActive - Whether the filter currently narrows results.
 * @returns Pill styles.
 */
export function filterPillStyles(isActive: boolean) {
  return [styles.filterPill, isActive && styles.filterPillActive];
}

/**
 * Render a pill's visible text, swapping the whole label in place when it changes.
 *
 * @param props - Text to show.
 * @returns Truncating pill label.
 */
export function FilterPillLabel({ text }: { text: string }) {
  return (
    <>
      <MotionPresence mode="wait">
        <MotionSpan
          key={text}
          {...stylex.props(styles.filterPillLabelMotionSpan)}
          variants={SWAP_VARIANTS}
          initial="hidden"
          animate="visible"
          exit="exit"
        >
          {text}
        </MotionSpan>
      </MotionPresence>
      <ChevronDown {...stylex.props(styles.filterPillLabelChevronDown)} aria-hidden="true" />
    </>
  );
}

/**
 * Build the visible text and accessible name of one filter pill.
 *
 * @param label - Facet name.
 * @param isActive - Whether the filter currently narrows results.
 * @param summary - Short visible summary of the selection.
 * @param accessibleSummary - Spoken summary when it differs from the visible one.
 * @returns Visible text and accessible name.
 */
export function getFilterPillText(
  label: string,
  isActive: boolean,
  summary?: string,
  accessibleSummary?: string,
): { visibleText: string; accessibleName: string } {
  const spokenSummary = accessibleSummary ?? summary;
  return {
    visibleText: isActive && summary ? summary : label,
    accessibleName: isActive && spokenSummary ? `${label}：${spokenSummary}` : label,
  };
}

type FilterPanelHeaderProps = {
  description: string;
  isActive: boolean;
  label: string;
  onClear?: () => void;
};

/**
 * Render a filter panel's title, description, and clear action.
 *
 * @param props - Facet name, description, applied state, and clear action.
 * @returns Panel header.
 */
export function FilterPanelHeader({
  description,
  isActive,
  label,
  onClear,
}: FilterPanelHeaderProps) {
  return (
    <div {...stylex.props(styles.filterPanelHeaderDiv)}>
      <div {...stylex.props(styles.filterPanelHeaderDiv2)}>
        <p {...stylex.props(styles.filterPanelHeaderP)}>{label}</p>
        <p {...stylex.props(styles.filterPanelHeaderP2)}>{description}</p>
      </div>
      {isActive && onClear && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          xstyle={styles.filterPanelHeaderButton}
          onClick={onClear}
        >
          清除
        </Button>
      )}
    </div>
  );
}

type FilterFacetProps = {
  accessibleSummary?: string;
  children: ReactNode;
  description: string;
  isActive: boolean;
  label: string;
  onClear?: () => void;
  summary?: string;
};

/**
 * Render one filter pill whose free-form panel opens as a popover.
 *
 * @param props - Pill label and summary, panel copy, clear action, and panel content.
 * @returns Filter pill with its popover panel.
 */
export function FilterFacet({
  accessibleSummary,
  children,
  description,
  isActive,
  label,
  onClear,
  summary,
}: FilterFacetProps) {
  const { visibleText, accessibleName } = getFilterPillText(
    label,
    isActive,
    summary,
    accessibleSummary,
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={accessibleName}
          title={accessibleName}
          data-active={isActive ? 'true' : 'false'}
          {...stylex.props(filterPillStyles(isActive))}
        >
          <FilterPillLabel text={visibleText} />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        aria-label={label}
        xstyle={styles.filterFacetPopoverContent}
      >
        <FilterPanelHeader
          description={description}
          isActive={isActive}
          label={label}
          onClear={onClear}
        />
        <div {...stylex.props(styles.filterFacetDiv)}>{children}</div>
      </PopoverContent>
    </Popover>
  );
}

type DatabasePillProps = {
  ariaLabel: string;
  databases: readonly string[];
  formatLabel?: (database: string) => string;
  isDisabled?: boolean;
  onChange: (database: string) => void;
  value: string;
};

/**
 * Remove the storage suffix from a database file name.
 *
 * @param database - Layers file name.
 * @returns Display label.
 */
function formatDatabaseFileName(database: string): string {
  return database.replace(/\.sqlite$/, '');
}

/**
 * Render the current database as a dark context pill backed by an accessible select.
 *
 * @param props - Accessible label, options, label formatter, and change handler.
 * @returns Layers select pill.
 */
export function DatabasePill({
  ariaLabel,
  databases,
  formatLabel = formatDatabaseFileName,
  isDisabled = false,
  onChange,
  value,
}: DatabasePillProps) {
  return (
    <Select value={value} onValueChange={onChange} disabled={isDisabled}>
      <SelectTrigger
        aria-label={ariaLabel}
        size="sm"
        xstyle={styles.databasePillSelectTrigger}
        iconXstyle={styles.databasePillIcon}
      >
        <Layers
          {...stylex.props(styles.databasePillLayers, styles.databasePillIcon)}
          aria-hidden="true"
        />
        <SelectValue placeholder="选择数据库" />
      </SelectTrigger>
      <SelectContent position="popper" align="start">
        {databases.map((database) => (
          <SelectItem key={database} value={database}>
            {formatLabel(database)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

const styles = stylex.create({
  filterPill: {
    display: 'inline-flex',
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    maxWidth: '15rem',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: {
      default: colors.background,
      ':is([data-popup-open])': colors.accent,
      ':is([data-state="open"])': colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    paddingInline: {
      default: '0.875rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    fontSize: '13px',
    whiteSpace: 'nowrap',
    color: `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-visible': `0 0 0 2px ${colors.ring}, inset 0 0 0 1px ${colors.border}`,
    },
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
  },
  filterPillActive: {
    backgroundColor: {
      default: colors.info,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.info,
      },
      ':is([data-popup-open])': colors.info,
      ':is([data-state="open"])': colors.info,
    },
    fontWeight: 700,
    color: colors.infoForeground,
    boxShadow: {
      default: `inset 0 0 0 1px color-mix(in oklab,${colors.primary} 35%,transparent)`,
      ':focus-visible': `0 0 0 2px ${colors.ring}, inset 0 0 0 1px color-mix(in oklab,${colors.primary} 35%,transparent)`,
    },
  },
  filterFacetPopoverContent: {
    width: '20rem',
    maxWidth: 'calc(100vw - 2rem)',
    padding: '0px',
  },
  filterPillLabelMotionSpan: {
    minWidth: '0px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  filterPillLabelChevronDown: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    opacity: '70%',
  },
  filterPanelHeaderDiv: {
    display: 'flex',
    flexShrink: 0,
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingInline: '1rem',
    paddingBlock: '0.75rem',
  },
  filterPanelHeaderDiv2: {
    minWidth: '0px',
  },
  filterPanelHeaderP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  filterPanelHeaderP2: {
    marginTop: '0.125rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  filterPanelHeaderButton: {
    marginBlock: '-0.25rem',
    height: '2rem',
    flexShrink: 0,
    paddingInline: {
      default: '0.5rem',
      ':has( > svg)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  filterFacetDiv: {
    padding: '0.5rem',
  },
  databasePillSelectTrigger: {
    height: {
      default: '2.75rem',
      ':is([data-size="default"])': '2.25rem',
      '@media (min-width: 48rem)': {
        default: null,
        ':is([data-size="sm"])': '2rem',
      },
    },
    maxWidth: '16rem',
    flexShrink: 0,
    gap: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: {
      default: colors.foreground,
      ':active': {
        default: `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
        ':hover': {
          default: null,
          '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
      },
    },
    paddingInline: {
      default: '0.875rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    fontSize: '13px',
    fontWeight: 700,
    color: {
      default: colors.background,
      ':is([data-placeholder])': colors.mutedForeground,
    },
    boxShadow: {
      default: 'none',
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}`,
      },
    },
    lineHeight: 'inherit',
  },
  databasePillLayers: {
    width: '0.875rem',
    height: '0.875rem',
  },
  databasePillIcon: {
    color: `color-mix(in oklab, ${colors.background} 70%, transparent)`,
  },
});
