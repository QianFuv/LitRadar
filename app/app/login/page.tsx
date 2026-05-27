import { Suspense } from 'react';
import { LoginWorkspace } from '@/components/desktop/login-workspace';

/**
 * Render the login route.
 *
 * @returns Login page.
 */
export default function LoginPage() {
  return (
    <Suspense>
      <LoginWorkspace />
    </Suspense>
  );
}
