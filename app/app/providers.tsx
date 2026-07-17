'use client';

/**
 * Root client providers and browser-global error listener lifecycle.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NuqsAdapter } from 'nuqs/adapters/next/app';
import { ThemeProvider } from 'next-themes';
import { useEffect, useState } from 'react';

import { AuthProvider } from '@/lib/auth-context';
import { reportClientError } from '@/lib/client-logger';

/**
 * Report an uncaught browser error without reading its message, source URL, or stack.
 *
 * @param event - Browser error event.
 */
function handleWindowError(event: ErrorEvent): void {
  reportClientError('window_error', event.error);
}

/**
 * Report an unhandled promise rejection without serializing its reason payload.
 *
 * @param event - Browser promise rejection event.
 */
function handleUnhandledRejection(event: PromiseRejectionEvent): void {
  reportClientError('unhandled_rejection', event.reason);
}

/**
 * Register browser-global error listeners and return their exact cleanup operation.
 *
 * @returns Listener cleanup function.
 */
function registerClientErrorListeners(): () => void {
  window.addEventListener('error', handleWindowError);
  window.addEventListener('unhandledrejection', handleUnhandledRejection);

  /**
   * Remove the browser-global error listeners installed by this effect instance.
   */
  function removeClientErrorListeners(): void {
    window.removeEventListener('error', handleWindowError);
    window.removeEventListener('unhandledrejection', handleUnhandledRejection);
  }

  return removeClientErrorListeners;
}

/**
 * Create the application query client once per provider instance.
 *
 * @returns Query client with application defaults.
 */
function createQueryClient(): QueryClient {
  return new QueryClient();
}

/**
 * Provide theme, URL-state, query, authentication, and global error facilities.
 *
 * @param props - Provider children.
 * @returns Root client provider tree.
 */
export default function Providers({ children }: { children: React.ReactNode }) {
  const [queryClient] = useState(createQueryClient);

  useEffect(registerClientErrorListeners, []);

  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
      <NuqsAdapter>
        <QueryClientProvider client={queryClient}>
          <AuthProvider>{children}</AuthProvider>
        </QueryClientProvider>
      </NuqsAdapter>
    </ThemeProvider>
  );
}
