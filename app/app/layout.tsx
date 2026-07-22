/**
 * Root application document, metadata, global styles, and providers.
 */

import type { Metadata, Viewport } from 'next';
import Providers from './providers';
import './globals.css';

export const metadata: Metadata = {
  title: {
    default: 'LitRadar | QianFuv',
    template: '%s | LitRadar',
  },
  description: '检索、收藏并追踪学术文献。',
  icons: {
    icon: '/litradar-logo.png',
  },
};

export const viewport: Viewport = {
  colorScheme: 'light dark',
  themeColor: [
    { media: '(prefers-color-scheme: light)', color: '#ffffff' },
    { media: '(prefers-color-scheme: dark)', color: '#000000' },
  ],
};

/**
 * Render the application document, providers, and skip navigation link.
 *
 * @param props - Root layout children.
 * @returns Application document.
 */
export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN" suppressHydrationWarning>
      <body className="antialiased">
        <a href="#main-content" className="skip-link">
          跳到主要内容
        </a>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
