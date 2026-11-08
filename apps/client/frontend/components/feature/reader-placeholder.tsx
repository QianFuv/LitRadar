/**
 * Empty reading-desk state with the radar mark and keyboard hints.
 */

import type { ReactNode } from 'react';

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
 * @param props - Optional size class.
 * @returns Decorative radar illustration.
 */
export function RadarMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 160 160" aria-hidden="true" className={className}>
      <defs>
        <linearGradient id="radar-sweep-gradient" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="var(--primary)" stopOpacity="0" />
          <stop offset="100%" stopColor="var(--primary)" stopOpacity="0.28" />
        </linearGradient>
      </defs>
      <g fill="none" stroke="currentColor" strokeWidth="1">
        <circle cx="80" cy="80" r="76" opacity="0.35" />
        <circle cx="80" cy="80" r="54" opacity="0.3" />
        <circle cx="80" cy="80" r="32" opacity="0.25" />
        <path d="M80 4v152M4 80h152" opacity="0.18" />
      </g>
      <g className="radar-sweep">
        <path d="M80 80 L156 80 A76 76 0 0 0 133.7 26.3 Z" fill="url(#radar-sweep-gradient)" />
        <path d="M80 80 L156 80" stroke="var(--primary)" strokeOpacity="0.55" strokeWidth="1.25" />
      </g>
      <circle cx="112" cy="50" r="3" fill="var(--primary)" />
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
    <div
      data-slot="reader-placeholder"
      className="flex min-h-full flex-col items-center justify-center px-8 py-16 text-center"
    >
      <RadarMark className="size-36 text-muted-foreground" />
      <h2 className="mt-8 text-lg font-bold text-foreground">{title}</h2>
      <p className="mt-2 max-w-sm text-[13px] leading-relaxed text-muted-foreground">
        {description}
      </p>
      {hints.length > 0 && (
        <dl className="mt-8 grid gap-2.5 text-left text-xs text-muted-foreground">
          {hints.map((hint) => (
            <div key={hint.label} className="flex items-center gap-3">
              <dt className="flex min-w-24 justify-end gap-1">
                {hint.keys.map((key) => (
                  <kbd
                    key={key}
                    className="inline-flex h-6 min-w-6 items-center justify-center rounded-md bg-paper px-1.5 font-mono text-[11px] text-foreground shadow-vercel-ring"
                  >
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
