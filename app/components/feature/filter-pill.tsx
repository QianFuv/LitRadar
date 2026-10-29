'use client';

/**
 * Shared filter pills: popover or bottom-sheet facets, option rows, and database pills.
 */

import { ChevronDown, Database } from 'lucide-react';
import type { ReactNode } from 'react';

import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { POPOVER_MEDIA_QUERY, useMediaQuery } from '@/lib/use-media-query';
import { cn } from '@/lib/utils';

/** Neutral filter pill styles. */
const FILTER_PILL_CLASS =
  'motion-control inline-flex h-11 shrink-0 items-center gap-1.5 rounded-full bg-background px-3.5 md:h-8 md:px-3 text-[13px] whitespace-nowrap text-foreground/85 shadow-[inset_0_0_0_1px_var(--border)] outline-none transition-[background-color,color,box-shadow] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-accent';

/** Applied filter pill styles. */
const FILTER_PILL_ACTIVE_CLASS =
  'bg-info font-bold text-info-foreground shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--primary)_35%,transparent)] hover:bg-info data-[state=open]:bg-info';

type FilterFacetProps = {
  accessibleSummary?: string;
  children: ReactNode;
  contentClassName?: string;
  description: string;
  isActive: boolean;
  label: string;
  onClear?: () => void;
  onOpenChange?: (open: boolean) => void;
  open?: boolean;
  summary?: string;
  tone?: 'filter' | 'selection';
};

/**
 * Render one filter pill whose panel opens as a popover on wide screens and a sheet on phones.
 *
 * @param props - Pill label and summary, panel copy, clear action, and panel content.
 * @returns Filter pill with its panel.
 */
export function FilterFacet({
  accessibleSummary,
  children,
  contentClassName,
  description,
  isActive,
  label,
  onClear,
  onOpenChange,
  open,
  summary,
  tone = 'filter',
}: FilterFacetProps) {
  const isPopover = useMediaQuery(POPOVER_MEDIA_QUERY);
  const visibleText = isActive && summary ? summary : label;
  const spokenSummary = accessibleSummary ?? summary;
  const accessibleName = isActive && spokenSummary ? `${label}：${spokenSummary}` : label;
  const trigger = (
    <button
      type="button"
      aria-label={accessibleName}
      title={accessibleName}
      data-active={isActive ? 'true' : 'false'}
      className={cn(
        FILTER_PILL_CLASS,
        isActive && tone === 'filter' && FILTER_PILL_ACTIVE_CLASS,
        isActive && tone === 'selection' && 'font-bold text-foreground',
        'max-w-[15rem]',
      )}
    >
      <span className="truncate">{visibleText}</span>
      <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
    </button>
  );
  const header = (titleNode: ReactNode, descriptionNode: ReactNode) => (
    <div className="flex items-start justify-between gap-3 border-b px-4 py-3">
      <div className="min-w-0">
        {titleNode}
        {descriptionNode}
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

  if (isPopover) {
    return (
      <Popover open={open} onOpenChange={onOpenChange}>
        <PopoverTrigger asChild>{trigger}</PopoverTrigger>
        <PopoverContent
          align="start"
          sideOffset={6}
          aria-label={label}
          className={cn('w-80 max-w-[calc(100vw-2rem)] p-0', contentClassName)}
        >
          {header(
            <p className="text-sm font-bold">{label}</p>,
            <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>,
          )}
          <div className="p-2">{children}</div>
        </PopoverContent>
      </Popover>
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent placement="bottom" className="flex flex-col gap-0 overflow-hidden">
        <div className="mx-auto mt-2 h-1 w-10 shrink-0 rounded-full bg-border" aria-hidden="true" />
        {header(
          <DialogTitle className="text-base">{label}</DialogTitle>,
          <DialogDescription className="mt-0.5 text-xs">{description}</DialogDescription>,
        )}
        <div className="min-h-0 flex-1 overflow-y-auto p-2">{children}</div>
        <div className="shrink-0 border-t px-4 py-3">
          <DialogClose asChild>
            <Button type="button" className="h-11 w-full rounded-xl">
              完成
            </Button>
          </DialogClose>
        </div>
      </DialogContent>
    </Dialog>
  );
}

type FilterOptionRowProps = {
  ariaLabel?: string;
  count?: number;
  id: string;
  isChecked: boolean;
  label: ReactNode;
  onCheckedChange: (isChecked: boolean) => void;
  title?: string;
};

/**
 * Render one checkbox option inside a filter panel.
 *
 * @param props - Input identity, label, checked state, optional count, and change handler.
 * @returns Filter option row.
 */
export function FilterOptionRow({
  ariaLabel,
  count,
  id,
  isChecked,
  label,
  onCheckedChange,
  title,
}: FilterOptionRowProps) {
  return (
    <label
      htmlFor={id}
      title={title}
      className="motion-control content-visibility-filter-row flex min-h-10 cursor-pointer items-center gap-3 rounded-lg px-2.5 py-2 text-[13px] leading-5 transition-colors hover:bg-accent md:min-h-9"
    >
      <Checkbox
        id={id}
        aria-label={ariaLabel}
        checked={isChecked}
        onCheckedChange={(checked) => onCheckedChange(checked === true)}
      />
      <span className={cn('min-w-0 flex-1 break-words', isChecked && 'font-bold')}>{label}</span>
      {count !== undefined && (
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{count}</span>
      )}
    </label>
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
