/**
 * Public authentication route and metadata.
 */

import type { Metadata } from 'next';
import { Suspense } from 'react';
import LoginClient from './login-client';

export const metadata: Metadata = {
  title: '登录',
  description: '登录或注册 LitRadar 账号。',
};

/**
 * Render the login client inside the search-parameter suspense boundary.
 *
 * @returns Login route content.
 */
export default function LoginPage() {
  return (
    <Suspense>
      <LoginClient />
    </Suspense>
  );
}
