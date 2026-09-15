/** Source-language notice presentation with separate reading and timeline regions. */

import { ArrowUpRight, CalendarDays, ChevronDown } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { CfpNoticeView, CfpState } from '@/lib/api';
import { CFP_DATE_LABELS, CFP_KIND_LABELS } from '@/lib/cfp-display';
import { cn } from '@/lib/utils';

const STATE_LABELS: Readonly<Record<CfpState, string>> = {
  open: '征稿中',
  upcoming: '即将开放',
  closed: '已结束',
  historical: '历史征稿',
  invitation_only: '仅限受邀投稿',
  undated: '截止时间待确认',
  uncertain: '投稿时间待确认',
};
const STATE_CLASSES: Readonly<Record<CfpState, string>> = {
  open: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300',
  upcoming: 'bg-blue-50 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300',
  closed: 'bg-muted text-muted-foreground',
  historical: 'bg-muted text-muted-foreground',
  invitation_only: 'bg-violet-50 text-violet-700 dark:bg-violet-950/50 dark:text-violet-300',
  undated: 'bg-amber-50 text-amber-800 dark:bg-amber-950/50 dark:text-amber-300',
  uncertain: 'bg-amber-50 text-amber-800 dark:bg-amber-950/50 dark:text-amber-300',
};
const DATE_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  timeZone: 'UTC',
});

/** Format a server-validated calendar date without applying the browser timezone. */
function formatDate(value: string): string {
  return DATE_FORMATTER.format(new Date(`${value}T00:00:00Z`));
}

/** Keep verification metadata and the original-source action on consistent card gutters. */
export function CfpSourceFooter({
  checkedOn,
  sourceUrl,
  label = '查看征稿原文',
}: {
  checkedOn?: string | null;
  sourceUrl?: string | null;
  label?: string;
}) {
  return (
    <footer
      data-slot="cfp-source-footer"
      className="flex flex-col gap-3 border-t px-5 py-4 sm:flex-row sm:items-center sm:gap-5 sm:px-6"
    >
      <span className="text-xs leading-5 text-muted-foreground">
        核验于 {checkedOn ? formatDate(checkedOn) : '—'}
      </span>
      {sourceUrl && (
        <Button variant="outline" size="sm" className="h-9 w-full shrink-0 sm:w-auto" asChild>
          <a href={sourceUrl} target="_blank" rel="noopener noreferrer">
            {label}
            <ArrowUpRight className="size-3.5" aria-hidden="true" />
          </a>
        </Button>
      )}
    </footer>
  );
}

/** Present the backend's initial gate and other milestones in aligned label/value rows. */
function CfpTimeline({ notice }: { notice: CfpNoticeView }) {
  const deadline = notice.entryDeadline;
  const rows = [
    {
      key: 'entry',
      label: deadline ? CFP_DATE_LABELS[deadline.stage] : '投稿截止',
      value: deadline,
      isPrimary: true,
    },
    ...notice.dates
      .filter((date) => date.date !== deadline?.date || date.stage !== deadline?.stage)
      .map((date) => ({
        key: `${date.stage}:${date.date}`,
        label: `${CFP_DATE_LABELS[date.stage]}${date.isOptional ? '（可选）' : ''}`,
        value: date,
        isPrimary: false,
      })),
  ];

  return (
    <aside
      data-slot="cfp-timeline"
      aria-label="时间安排"
      className="min-w-0 self-start rounded-lg border border-border/60 bg-muted/30 p-4"
    >
      <h4 className="flex items-center gap-2 text-xs font-semibold leading-5">
        <CalendarDays className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        时间安排
      </h4>
      <dl className="mt-2 divide-y divide-border/60">
        {rows.map((row) => (
          <div
            key={row.key}
            className="grid grid-cols-[minmax(0,1fr)_7.5rem] items-baseline gap-x-3 py-2.5"
          >
            <dt className="text-xs leading-5 text-muted-foreground">{row.label}</dt>
            <dd
              className={cn(
                'm-0 whitespace-nowrap text-sm leading-5 tabular-nums',
                row.isPrimary ? 'font-semibold text-foreground' : 'text-muted-foreground',
              )}
            >
              {row.value ? (
                <>
                  <time dateTime={row.value.date}>{formatDate(row.value.date)}</time>
                  {row.value.isExclusive && <span className="ml-1 text-xs">前</span>}
                </>
              ) : (
                '暂未确认'
              )}
            </dd>
          </div>
        ))}
      </dl>
      {notice.dates.length > 0 && (
        <p className="border-t border-border/60 pt-3 text-[11px] leading-5 text-muted-foreground [overflow-wrap:anywhere]">
          {notice.timeZone ? `时区：${notice.timeZone}` : '原文未注明时区'}
        </p>
      )}
    </aside>
  );
}

/** Show every supplied paragraph inside a fixed-height, keyboard-scrollable reading region. */
function CfpOriginalText({ label, text }: { label: string; text: string }) {
  return (
    <div
      role="region"
      aria-label={label}
      tabIndex={0}
      className="h-48 min-w-0 overflow-y-auto overscroll-contain rounded-sm pr-3 outline-none [scrollbar-gutter:stable] [scrollbar-width:thin] focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
    >
      <p className="whitespace-pre-wrap text-sm leading-6 [overflow-wrap:anywhere]">{text}</p>
    </div>
  );
}

/** Render the original title, distinct text sections and server-owned submission timeline. */
export function CfpNoticeCard({ notice }: { notice: CfpNoticeView }) {
  const hasOriginalText = Boolean(notice.scope || notice.requirements);

  return (
    <article
      data-slot="cfp-notice"
      className="min-w-0 overflow-hidden rounded-xl bg-card shadow-vercel-ring"
    >
      <header className="space-y-3 border-b border-border/70 px-5 py-5 sm:px-6">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="h-6 rounded-md px-2 leading-none">
            {CFP_KIND_LABELS[notice.kind]}
          </Badge>
          <Badge
            variant="secondary"
            className={cn('h-6 rounded-md px-2 leading-none', STATE_CLASSES[notice.state])}
          >
            <span className="size-1.5 rounded-full bg-current" aria-hidden="true" />
            {STATE_LABELS[notice.state]}
          </Badge>
        </div>
        <h3 className="text-base font-semibold leading-relaxed text-foreground sm:text-lg [overflow-wrap:anywhere]">
          {notice.title}
        </h3>
      </header>
      <div
        className={cn(
          'grid min-w-0 gap-5 px-5 py-5 sm:px-6 sm:py-6',
          hasOriginalText && 'lg:grid-cols-[minmax(0,1fr)_15rem] lg:gap-8',
        )}
      >
        <div className={cn('min-w-0', hasOriginalText && 'lg:col-start-2 lg:row-start-1')}>
          <CfpTimeline notice={notice} />
        </div>
        {hasOriginalText && (
          <div className="min-w-0 space-y-6 lg:col-start-1 lg:row-start-1 lg:pt-4">
            {notice.scope && (
              <section data-slot="cfp-scope" className="space-y-3">
                <h4 className="text-xs font-semibold leading-5 text-muted-foreground">投稿范围</h4>
                <CfpOriginalText label="投稿范围" text={notice.scope} />
              </section>
            )}
            {notice.requirements && (
              <section
                data-slot="cfp-requirements"
                className={cn('space-y-3', notice.scope && 'border-t border-border/70 pt-6')}
              >
                <h4 className="text-xs font-semibold leading-5 text-muted-foreground">投稿要求</h4>
                <CfpOriginalText label="投稿要求" text={notice.requirements} />
              </section>
            )}
          </div>
        )}
      </div>
      {notice.rawDateText && (
        <details className="group border-t border-border/70 px-5 py-4 sm:px-6">
          <summary className="flex cursor-pointer list-none items-center justify-between gap-3 text-xs font-medium leading-5 [&::-webkit-details-marker]:hidden">
            查看原文时间说明
            <ChevronDown
              className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180"
              aria-hidden="true"
            />
          </summary>
          <p className="mt-3 whitespace-pre-line text-sm leading-6 text-muted-foreground [overflow-wrap:anywhere]">
            {notice.rawDateText}
          </p>
        </details>
      )}
      <CfpSourceFooter checkedOn={notice.checkedOn} sourceUrl={notice.sourceUrl} />
    </article>
  );
}
