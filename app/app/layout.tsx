import type { Metadata } from 'next';
import './globals.css';
import Providers from './providers';

export const metadata: Metadata = {
  title: 'Paper Scanner',
  description: 'Research article discovery and tracking workspace.',
  icons: {
    icon: 'https://cdn.sa.net/2026/01/29/6uRXpHqQfC89kF7.png',
  },
};

/**
 * Render the root document shell for the Paper Scanner frontend.
 *
 * @param props - Root layout props.
 * @returns The application document.
 */
export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN" suppressHydrationWarning>
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
