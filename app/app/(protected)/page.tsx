import { Suspense } from 'react';
import { SearchWorkspace } from '@/components/desktop/search-workspace';

/**
 * Render the protected search route.
 *
 * @returns Search page.
 */
export default function Home() {
  return (
    <Suspense fallback={<div className="desktop-loading">正在打开检索工作台...</div>}>
      <SearchWorkspace />
    </Suspense>
  );
}
