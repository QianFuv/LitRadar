'use client';

/**
 * Shared filter pills: pill styles and labels, panel headers, popover facets, and database pills.
 */

import { Database } from 'lucide-react';
import { ChevronDown } from '@/components/ui/animated-icons';
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
import { cn } from '@/lib/utils';

/** Neutral filter pill styles, open for both Radix (`data-state`) and Base UI (`data-popup-open`). */
const FILTER_PILL_CLASS =
  'motion-control inline-flex h-11 max-w-[15rem] shrink-0 items-center gap-1.5 rounded-full bg-background px-3.5 md:h-8 md:px-3 text-[13px] whitespace-nowrap text-foreground/85 shadow-[inset_0_0_0_1px_var(--border)] outline-none transition-[background-color,color,box-shadow] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-accent data-[popup-open]:bg-accent';

/** Applied filter pill styles. */
const FILTER_PILL_ACTIVE_CLASS =
  'bg-info font-bold text-info-foreground shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--primary)_35%,transparent)] hover:bg-info data-[state=open]:bg-info data-[popup-open]:bg-info';

/**
 * Build the class list of one filter pill trigger.
 *
 * @param isActive - Whether the filter currently narrows results.
 * @returns Pill class list.
 */
export function getFilterPillClassName(isActive: boolean): string {
  return cn(FILTER_PILL_CLASS, isActive && FILTER_PILL_ACTIVE_CLASS);
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
          className="min-w-0 truncate"
          variants={SWAP_VARIANTS}
          initial="hidden"
          animate="visible"
          exit="exit"
        >
          {text}
        </MotionSpan>
      </MotionPresence>
      <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
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
    <div className="flex shrink-0 items-start justify-between gap-3 border-b px-4 py-3">
      <div className="min-w-0">
        <p className="text-sm font-bold">{label}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
      </div>
      {isActive && onClear && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="-my-1 h-8 shrink-0 px-2 text-xs text-muted-foreground"
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
  contentClassName?: string;
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
  contentClassName,
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
          className={getFilterPillClassName(isActive)}
        >
          <FilterPillLabel text={visibleText} />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        aria-label={label}
        className={cn('w-80 max-w-[calc(100vw-2rem)] p-0', contentClassName)}
      >
        <FilterPanelHeader
          description={description}
          isActive={isActive}
          label={label}
          onClear={onClear}
        />
        <div className="p-2">{children}</div>
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
 * @param database - Database file name.
 * @returns Display label.
 */
function formatDatabaseFileName(database: string): string {
  return database.replace(/\.sqlite$/, '');
}

/**
 * Render the current database as a dark context pill backed by an accessible select.
 *
 * @param props - Accessible label, options, label formatter, and change handler.
 * @returns Database select pill.
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
        className="h-11 max-w-[16rem] shrink-0 gap-1.5 rounded-full bg-foreground px-3.5 text-[13px] font-bold data-[size=sm]:h-11 md:px-3 md:data-[size=sm]:h-8 text-background !shadow-none hover:bg-foreground/85 active:bg-foreground/80 [&_svg:not([class*='text-'])]:text-background/70"
      >
        <Database className="size-3.5" aria-hidden="true" />
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
