import type { Metadata } from 'next';
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
  title: 'Paper Scanner | QianFuv',
  description: 'Paper Scanner by QianFuv',
  icons: {
    icon: 'https://cdn.sa.net/2026/01/29/6uRXpHqQfC89kF7.png',
  },
};

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
