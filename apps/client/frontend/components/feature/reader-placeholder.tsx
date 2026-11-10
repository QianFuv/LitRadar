/**
 * Empty reading-desk state with the radar mark and keyboard hints.
 */

import type { ReactNode } from 'react';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

type ReaderPlaceholderProps = {
  description: ReactNode;
  hints?: readonly { keys: readonly string[]; label: string }[];
  title: ReactNode;
};

const DEFAULT_HINTS = [
  { keys: ['/'], label: '聚焦搜索' },
  { keys: ['J', 'K'], label: '下一篇 / 上一篇' },
  { keys: ['Enter'], label: '打开所选条目' },
] as const;

/**
 * Render the decorative radar mark used by empty reading surfaces.
 *
 * @param props - Optional size styles.
 * @returns Decorative radar illustration.
 */
export function RadarMark({ xstyle }: { xstyle?: stylex.StyleXStyles }) {
  return (
    <svg viewBox="0 0 160 160" aria-hidden="true" {...stylex.props(xstyle)}>
      <defs>
        <linearGradient id="radar-sweep-gradient" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor={colors.primary} stopOpacity="0" />
          <stop offset="100%" stopColor={colors.primary} stopOpacity="0.28" />
        </linearGradient>
      </defs>
      <g fill="none" stroke="currentColor" strokeWidth="1">
        <circle cx="80" cy="80" r="76" opacity="0.35" />
        <circle cx="80" cy="80" r="54" opacity="0.3" />
        <circle cx="80" cy="80" r="32" opacity="0.25" />
        <path d="M80 4v152M4 80h152" opacity="0.18" />
      </g>
      <g {...stylex.props(styles.radarSweep)}>
        <path d="M80 80 L156 80 A76 76 0 0 0 133.7 26.3 Z" fill="url(#radar-sweep-gradient)" />
        <path d="M80 80 L156 80" stroke={colors.primary} strokeOpacity="0.55" strokeWidth="1.25" />
      </g>
      <circle cx="112" cy="50" r="3" fill={colors.primary} />
      <circle cx="52" cy="104" r="2.5" fill="currentColor" opacity="0.45" />
      <circle cx="118" cy="112" r="2" fill="currentColor" opacity="0.35" />
      <circle cx="80" cy="80" r="3" fill="currentColor" opacity="0.6" />
    </svg>
  );
}

/**
 * Render the docked reader's empty state.
 *
 * @param props - Title, description, and optional keyboard hints.
 * @returns Centered empty reading-desk content.
 */
export function ReaderPlaceholder({
  description,
  hints = DEFAULT_HINTS,
  title,
}: ReaderPlaceholderProps) {
  return (
    <div data-slot="reader-placeholder" {...stylex.props(styles.readerPlaceholder)}>
      <RadarMark xstyle={styles.readerPlaceholderRadarMark} />
      <h2 {...stylex.props(styles.readerPlaceholderH2)}>{title}</h2>
      <p {...stylex.props(styles.readerPlaceholderP)}>{description}</p>
      {hints.length > 0 && (
        <dl {...stylex.props(styles.readerPlaceholderDl)}>
          {hints.map((hint) => (
            <div key={hint.label} {...stylex.props(styles.readerPlaceholderDiv)}>
              <dt {...stylex.props(styles.readerPlaceholderDt)}>
                {hint.keys.map((key) => (
                  <kbd key={key} {...stylex.props(styles.readerPlaceholderKbd)}>
                    {key}
                  </kbd>
                ))}
              </dt>
              <dd>{hint.label}</dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

const radarSweepKeyframes = stylex.keyframes({ to: { transform: 'rotate(360deg)' } });

const styles = stylex.create({
  radarSweep: {
    transformOrigin: '50% 50%',
    animationName: radarSweepKeyframes,
    animationDuration: '6s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  readerPlaceholder: {
    display: 'flex',
    minHeight: '100%',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    paddingInline: '2rem',
    paddingBlock: '4rem',
    textAlign: 'center',
  },
  readerPlaceholderRadarMark: {
    width: '9rem',
    height: '9rem',
    color: colors.mutedForeground,
  },
  readerPlaceholderH2: {
    marginTop: '2rem',
    fontSize: '1.125rem',
    lineHeight: lineHeights.textLg,
    fontWeight: 700,
    color: colors.foreground,
  },
  readerPlaceholderP: {
    marginTop: '0.5rem',
    maxWidth: '24rem',
    fontSize: '13px',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  readerPlaceholderDl: {
    marginTop: '2rem',
    display: 'grid',
    gap: '0.625rem',
    textAlign: 'left',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  readerPlaceholderDiv: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  readerPlaceholderDt: {
    display: 'flex',
    minWidth: '6rem',
    justifyContent: 'flex-end',
    gap: '0.25rem',
  },
  readerPlaceholderKbd: {
    display: 'inline-flex',
    height: '1.5rem',
    minWidth: '1.5rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '6px',
    backgroundColor: colors.paper,
    paddingInline: '0.375rem',
    fontFamily: "'JetBrainsLxgwNerdMono', monospace",
    fontSize: '11px',
    color: colors.foreground,
    boxShadow: shadows.vercelRing,
  },
});
