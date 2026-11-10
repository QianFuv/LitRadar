/** Source-language notice presentation with separate reading and timeline regions. */

import { CalendarDays, ChevronDown, ExternalLink } from '@/components/ui/animated-icons';

import {
  Root as Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@animate-ui/react/primitives/radix-ui/collapsible';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { CfpNoticeView, CfpState } from '@/lib/api';
import { CFP_DATE_LABELS, CFP_KIND_LABELS } from '@/lib/cfp-display';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const STATE_LABELS: Readonly<Record<CfpState, string>> = {
  open: '征稿中',
  upcoming: '即将开放',
  closed: '已结束',
  historical: '历史征稿',
  invitation_only: '仅限受邀投稿',
  undated: '截止时间待确认',
  uncertain: '投稿时间待确认',
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
    <footer data-slot="cfp-source-footer" {...stylex.props(styles.cfpSourceFooter)}>
      <span {...stylex.props(styles.cfpSourceFooterSpan)}>
        核验于 {checkedOn ? formatDate(checkedOn) : '—'}
      </span>
      {sourceUrl && (
        <Button variant="outline" size="sm" xstyle={styles.cfpSourceFooterButton} asChild>
          <a href={sourceUrl} target="_blank" rel="noopener noreferrer">
            {label}
            <ExternalLink
              {...stylex.props(styles.cfpSourceFooterExternalLink)}
              aria-hidden="true"
            />
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
    <aside data-slot="cfp-timeline" aria-label="时间安排" {...stylex.props(styles.cfpTimeline)}>
      <h4 {...stylex.props(styles.cfpTimelineH4)}>
        <CalendarDays {...stylex.props(styles.cfpTimelineCalendarDays)} aria-hidden="true" />
        时间安排
      </h4>
      <dl {...stylex.props(styles.cfpTimelineDl)}>
        {rows.map((row) => (
          <div key={row.key} {...stylex.props(styles.cfpTimelineDlItem, styles.cfpTimelineDiv)}>
            <dt {...stylex.props(styles.cfpSourceFooterSpan)}>{row.label}</dt>
            <dd
              {...stylex.props(
                styles.cfpTimelineDd,
                row.isPrimary ? styles.cfpTimelineDdPrimary : styles.cfpTimelineDdNotPrimary,
              )}
            >
              {row.value ? (
                <>
                  <time dateTime={row.value.date}>{formatDate(row.value.date)}</time>
                  {row.value.isExclusive && (
                    <span {...stylex.props(styles.cfpTimelineSpan)}>前</span>
                  )}
                </>
              ) : (
                '暂未确认'
              )}
            </dd>
          </div>
        ))}
      </dl>
      {notice.dates.length > 0 && (
        <p {...stylex.props(styles.cfpTimelineP)}>
          {notice.timeZone ? `时区：${notice.timeZone}` : '原文未注明时区'}
        </p>
      )}
    </aside>
  );
}

/** Show every supplied paragraph inside a fixed-height, keyboard-scrollable reading region. */
function CfpOriginalText({ label, text }: { label: string; text: string }) {
  return (
    <div role="region" aria-label={label} tabIndex={0} {...stylex.props(styles.cfpOriginalTextDiv)}>
      <p {...stylex.props(styles.cfpOriginalTextP)}>{text}</p>
    </div>
  );
}

/** Render the original title, distinct text sections and server-owned submission timeline. */
export function CfpNoticeCard({ notice }: { notice: CfpNoticeView }) {
  const hasOriginalText = Boolean(notice.scope || notice.requirements);

  return (
    <article data-slot="cfp-notice" {...stylex.props(styles.cfpNotice)}>
      <header>
        <div {...stylex.props(styles.cfpNoticeCardHeaderItem, styles.cfpNoticeCardDiv)}>
          <Badge variant="outline" xstyle={styles.cfpNoticeCardBadge}>
            {CFP_KIND_LABELS[notice.kind]}
          </Badge>
          <Badge variant="secondary" xstyle={[styles.stateBadge, STATE_STYLES[notice.state]]}>
            <span {...stylex.props(styles.cfpNoticeCardSpan)} aria-hidden="true" />
            {STATE_LABELS[notice.state]}
          </Badge>
        </div>
        <h3 {...stylex.props(styles.cfpNoticeCardHeaderItem, styles.cfpNoticeCardH3)}>
          {notice.title}
        </h3>
      </header>
      <div
        {...stylex.props(
          styles.cfpNoticeCardDiv2,
          hasOriginalText && styles.cfpNoticeCardDivOriginalText,
        )}
      >
        <div
          {...stylex.props(
            styles.cfpNoticeCardDiv3,
            hasOriginalText && styles.cfpNoticeCardDivOriginalText2,
          )}
        >
          <CfpTimeline notice={notice} />
        </div>
        {hasOriginalText && (
          <div {...stylex.props(styles.cfpNoticeCardDiv4)}>
            {notice.scope && (
              <section
                data-slot="cfp-scope"
                {...stylex.props(styles.cfpNoticeCardDivItem, styles.cfpScope)}
              >
                <h4 {...stylex.props(styles.cfpNoticeCardH4)}>投稿范围</h4>
                <CfpOriginalText label="投稿范围" text={notice.scope} />
              </section>
            )}
            {notice.requirements && (
              <section
                data-slot="cfp-requirements"
                {...stylex.props(
                  styles.cfpNoticeCardDivItem,
                  styles.cfpScope,
                  Boolean(notice.scope) && styles.cfpRequirementsScope,
                )}
              >
                <h4 {...stylex.props(styles.cfpNoticeCardH4)}>投稿要求</h4>
                <CfpOriginalText label="投稿要求" text={notice.requirements} />
              </section>
            )}
          </div>
        )}
      </div>
      {notice.rawDateText && (
        <Collapsible {...stylex.props(styles.cfpNoticeCardCollapsible)}>
          <CollapsibleTrigger
            {...stylex.props(styles.cfpNoticeCardCollapsibleTrigger, stylex.defaultMarker())}
          >
            查看原文时间说明
            <ChevronDown {...stylex.props(styles.cfpNoticeCardChevronDown)} aria-hidden="true" />
          </CollapsibleTrigger>
          <CollapsibleContent>
            <p {...stylex.props(styles.cfpNoticeCardP)}>{notice.rawDateText}</p>
          </CollapsibleContent>
        </Collapsible>
      )}
      <CfpSourceFooter checkedOn={notice.checkedOn} sourceUrl={notice.sourceUrl} />
    </article>
  );
}

const styles = stylex.create({
  stateBadge: {
    height: '1.5rem',
    paddingInline: '0.625rem',
    lineHeight: 1,
  },
  stateOpen: {
    backgroundColor: colors.success,
    color: colors.successForeground,
  },
  stateUpcoming: {
    backgroundColor: colors.info,
    color: colors.infoForeground,
  },
  stateClosed: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  stateHistorical: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  stateInvitationOnly: {
    backgroundColor: colors.info,
    color: colors.infoForeground,
  },
  stateUndated: {
    backgroundColor: colors.warning,
    color: colors.warningForeground,
  },
  stateUncertain: {
    backgroundColor: colors.warning,
    color: colors.warningForeground,
  },
  cfpSourceFooter: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    borderTopStyle: 'dashed',
    borderTopWidth: '1px',
    paddingTop: '1rem',
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
  },
  cfpSourceFooterSpan: {
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
  },
  cfpSourceFooterButton: {
    height: '2.25rem',
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
    flexShrink: 0,
    borderRadius: '8px',
  },
  cfpSourceFooterExternalLink: {
    width: '0.875rem',
    height: '0.875rem',
  },
  cfpTimeline: {
    minWidth: '0px',
    alignSelf: 'flex-start',
    borderRadius: '12px',
    backgroundColor: colors.muted,
    padding: '1rem',
  },
  cfpTimelineH4: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    fontWeight: 700,
  },
  cfpTimelineCalendarDays: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    color: colors.mutedForeground,
  },
  cfpTimelineDlItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
    },
    borderColor: {
      default: null,
      ':not(:last-child)': `color-mix(in oklab, ${colors.border} 60%, transparent)`,
    },
  },
  cfpTimelineDl: {
    marginTop: '0.5rem',
  },
  cfpTimelineDiv: {
    display: 'grid',
    gridTemplateColumns: 'minmax(0,1fr) 7.5rem',
    alignItems: 'baseline',
    columnGap: '0.75rem',
    paddingBlock: '0.625rem',
  },
  cfpTimelineDd: {
    margin: '0px',
    fontSize: '0.875rem',
    lineHeight: '1.25rem',
    whiteSpace: 'nowrap',
    fontVariantNumeric: 'tabular-nums',
  },
  cfpTimelineDdPrimary: {
    fontWeight: 700,
    color: colors.foreground,
  },
  cfpTimelineDdNotPrimary: {
    color: colors.mutedForeground,
  },
  cfpTimelineSpan: {
    marginLeft: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  cfpTimelineP: {
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.border} 60%, transparent)`,
    paddingTop: '0.75rem',
    fontSize: '11px',
    lineHeight: '1.25rem',
    overflowWrap: 'anywhere',
    color: colors.mutedForeground,
  },
  cfpOriginalTextDiv: {
    maxHeight: '18rem',
    minWidth: '0px',
    scrollbarWidth: 'thin',
    scrollbarGutter: 'stable',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    borderRadius: '4px',
    paddingRight: '0.75rem',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
  },
  cfpOriginalTextP: {
    fontSize: '13.5px',
    lineHeight: 1.8,
    overflowWrap: 'anywhere',
    whiteSpace: 'pre-wrap',
    color: `color-mix(in oklab, ${colors.foreground} 90%, transparent)`,
  },
  cfpNotice: {
    minWidth: '0px',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '1.75rem',
    paddingBottom: '2rem',
  },
  cfpNoticeCardHeaderItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  cfpNoticeCardDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  cfpNoticeCardBadge: {
    height: '1.5rem',
    paddingInline: '0.625rem',
    lineHeight: 1,
  },
  cfpNoticeCardSpan: {
    width: '0.375rem',
    height: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: 'currentcolor',
  },
  cfpNoticeCardH3: {
    fontSize: '19px',
    lineHeight: 1.45,
    fontWeight: 700,
    overflowWrap: 'anywhere',
    color: colors.foreground,
  },
  cfpNoticeCardDiv2: {
    display: 'grid',
    minWidth: '0px',
    gap: '1.5rem',
    paddingBlock: '1.5rem',
  },
  cfpNoticeCardDivOriginalText: {
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 96rem)': 'minmax(0,1fr) 14rem',
    },
    gap: {
      default: '1.5rem',
      '@media (min-width: 96rem)': '2rem',
    },
  },
  cfpNoticeCardDiv3: {
    minWidth: '0px',
  },
  cfpNoticeCardDivOriginalText2: {
    gridColumnStart: {
      default: null,
      '@media (min-width: 96rem)': 2,
    },
    gridRowStart: {
      default: null,
      '@media (min-width: 96rem)': 1,
    },
  },
  cfpNoticeCardDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1.5rem',
    },
  },
  cfpNoticeCardDiv4: {
    minWidth: '0px',
    gridColumnStart: {
      default: null,
      '@media (min-width: 96rem)': 1,
    },
    gridRowStart: {
      default: null,
      '@media (min-width: 96rem)': 1,
    },
  },
  cfpScope: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  cfpNoticeCardH4: {
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    fontWeight: 700,
    letterSpacing: '0.08em',
    color: colors.mutedForeground,
  },
  cfpRequirementsScope: {
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '1.5rem',
  },
  cfpNoticeCardCollapsible: {
    borderTopStyle: 'dashed',
    borderTopWidth: '1px',
    paddingBlock: '1rem',
  },
  cfpNoticeCardCollapsibleTrigger: {
    display: 'flex',
    width: '100%',
    cursor: 'pointer',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    textAlign: 'left',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    fontWeight: 500,
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  cfpNoticeCardChevronDown: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.mutedForeground,
    transitionProperty: 'transform, translate, scale, rotate',
    transitionTimingFunction: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
    transitionDuration: '350ms',
    rotate: {
      default: null,
      [stylex.when.ancestor('[data-state="open"]')]: '180deg',
    },
  },
  cfpNoticeCardP: {
    paddingTop: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    overflowWrap: 'anywhere',
    whiteSpace: 'pre-line',
    color: colors.mutedForeground,
  },
});

const STATE_STYLES: Readonly<Record<CfpState, stylex.StyleXStyles>> = {
  open: styles.stateOpen,
  upcoming: styles.stateUpcoming,
  closed: styles.stateClosed,
  historical: styles.stateHistorical,
  invitation_only: styles.stateInvitationOnly,
  undated: styles.stateUndated,
  uncertain: styles.stateUncertain,
};
