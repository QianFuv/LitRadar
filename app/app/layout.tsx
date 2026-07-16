/**
 * Root application document, metadata, fonts, and providers.
 */

import type { Metadata, Viewport } from 'next';
import { Geist, Geist_Mono } from 'next/font/google';
import Providers from './providers';
import './globals.css';

const geistSans = Geist({
  variable: '--font-geist-sans',
  subsets: ['latin'],
});

const geistMono = Geist_Mono({
  variable: '--font-geist-mono',
  subsets: ['latin'],
});

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
      <body className={`${geistSans.variable} ${geistMono.variable} antialiased`}>
        <a href="#main-content" className="skip-link">
          跳到主要内容
        </a>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
