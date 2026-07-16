'use client';

/**
 * Self-contained root document used when the normal application layout fails.
 */

import { useEffect, type CSSProperties } from 'react';

type GlobalErrorProps = {
  error: Error & { digest?: string };
  reset: () => void;
};

const GLOBAL_BODY_STYLE: CSSProperties = {
  alignItems: 'center',
  background: '#0a0a0a',
  color: '#f5f5f5',
  colorScheme: 'dark',
  display: 'flex',
  fontFamily: 'ui-sans-serif, system-ui, sans-serif',
  justifyContent: 'center',
  margin: 0,
  minHeight: '100vh',
  padding: '24px',
};

const GLOBAL_CARD_STYLE: CSSProperties = {
  background: '#111111',
  border: '1px solid #333333',
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
  background: '#f5f5f5',
  border: '1px solid #f5f5f5',
  borderRadius: '6px',
  color: '#111111',
  cursor: 'pointer',
  font: 'inherit',
  fontWeight: 600,
  padding: '10px 16px',
};

const GLOBAL_SECONDARY_BUTTON_STYLE: CSSProperties = {
  ...GLOBAL_BUTTON_STYLE,
  background: 'transparent',
  color: '#f5f5f5',
};

/**
 * Report a root-layout failure and render an independent recovery document.
 *
 * @param props - Captured global error and boundary reset callback.
 * @returns Self-contained global failure document.
 */
export default function GlobalError({ error, reset }: GlobalErrorProps) {
  useEffect(() => {
    console.error('LitRadar global error', {
      digest: error.digest,
      name: error.name,
    });
  }, [error]);

  return (
    <html lang="zh-CN">
      <head>
        <title>页面错误 | LitRadar</title>
      </head>
      <body style={GLOBAL_BODY_STYLE}>
        <main id="main-content" role="alert" style={GLOBAL_CARD_STYLE}>
          <h1 style={{ fontSize: '24px', margin: 0 }}>应用加载失败</h1>
          <p style={{ color: '#b3b3b3', lineHeight: 1.6, margin: '12px 0 0' }}>
            LitRadar 暂时无法加载。请重新加载，或返回首页后再试。
          </p>
          <div style={GLOBAL_ACTIONS_STYLE}>
            <button type="button" style={GLOBAL_BUTTON_STYLE} onClick={reset}>
              重新加载
            </button>
            <form action="/" style={{ margin: 0 }}>
              <button type="submit" style={GLOBAL_SECONDARY_BUTTON_STYLE}>
                返回首页
              </button>
            </form>
          </div>
        </main>
      </body>
    </html>
  );
}
