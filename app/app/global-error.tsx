'use client';

/**
 * Self-contained root document used when the normal application layout fails.
 */

import { useEffect, type CSSProperties } from 'react';

import { reportClientError } from '@/lib/client-logger';

type GlobalErrorProps = {
  error: Error & { digest?: string };
  reset: () => void;
};

const GLOBAL_BODY_STYLE: CSSProperties = {
  alignItems: 'center',
  background: '#111111',
  color: '#eeeeee',
  colorScheme: 'dark',
  display: 'flex',
  fontFamily: 'ui-sans-serif, system-ui, sans-serif',
  justifyContent: 'center',
  margin: 0,
  minHeight: '100vh',
  padding: '24px',
};

const GLOBAL_CARD_STYLE: CSSProperties = {
  background: '#191919',
  border: '1px solid #3a3a3a',
  borderRadius: '12px',
  boxSizing: 'border-box',
  maxWidth: '448px',
  padding: '24px',
  width: '100%',
};

const GLOBAL_ACTIONS_STYLE: CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: '12px',
  marginTop: '24px',
};

const GLOBAL_BUTTON_STYLE: CSSProperties = {
  background: 'var(--recovery-action-background, #3e63dd)',
  border: '1px solid #3e63dd',
  borderRadius: '6px',
  color: '#ffffff',
  cursor: 'pointer',
  font: 'inherit',
  fontWeight: 600,
  padding: '10px 16px',
};

const GLOBAL_SECONDARY_BUTTON_STYLE: CSSProperties = {
  ...GLOBAL_BUTTON_STYLE,
  background: 'var(--recovery-action-background, transparent)',
  borderColor: '#484848',
  color: '#eeeeee',
};

/**
 * Report a root-layout failure and render an independent recovery document.
 *
 * @param props - Captured global error and boundary reset callback.
 * @returns Self-contained global failure document.
 */
export default function GlobalError({ error, reset }: GlobalErrorProps) {
  useEffect(
    /**
     * Emit the global-boundary terminal event once for this error object.
     */
    function reportGlobalError(): void {
      reportClientError('global_boundary', error, { digest: error.digest });
    },
    [error],
  );

  return (
    <html lang="zh-CN">
      <head>
        <title>页面错误 | LitRadar</title>
        <style>{`
          [data-recovery-action] { transition: background-color 120ms ease; }
          [data-recovery-action='primary']:hover { --recovery-action-background: #3358d4; }
          [data-recovery-action='secondary']:hover { --recovery-action-background: #222222; }
          [data-recovery-action]:active { --recovery-action-background: #435db1; }
          [data-recovery-action='secondary']:active { --recovery-action-background: #313131; }
          [data-recovery-action]:focus-visible { outline: 3px solid #9eb1ff; outline-offset: 2px; }
          @media (prefers-reduced-motion: reduce) {
            [data-recovery-action] { transition: none; }
          }
        `}</style>
      </head>
      <body style={GLOBAL_BODY_STYLE}>
        <main id="main-content" role="alert" style={GLOBAL_CARD_STYLE}>
          <h1 style={{ fontSize: '24px', margin: 0 }}>应用加载失败</h1>
          <p style={{ color: '#b4b4b4', lineHeight: 1.6, margin: '12px 0 0' }}>
            LitRadar 暂时无法加载。请重新加载，或返回首页后再试。
          </p>
          <div style={GLOBAL_ACTIONS_STYLE}>
            <button
              type="button"
              data-recovery-action="primary"
              style={GLOBAL_BUTTON_STYLE}
              onClick={reset}
            >
              重新加载
            </button>
            <form action="/" style={{ margin: 0 }}>
              <button
                type="submit"
                data-recovery-action="secondary"
                style={GLOBAL_SECONDARY_BUTTON_STYLE}
              >
                返回首页
              </button>
            </form>
          </div>
        </main>
      </body>
    </html>
  );
}
