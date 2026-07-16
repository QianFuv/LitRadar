'use client';

import { Sidebar } from '@/components/feature/sidebar';
import { ActiveFilterChips } from '@/components/feature/active-filter-chips';
import { SearchBar } from '@/components/feature/search-bar';
import { ResultsList } from '@/components/feature/results-list';
import { AnnouncementsDialog } from '@/components/announcements-dialog';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Menu } from 'lucide-react';
import { useState } from 'react';

/**
 * Render the article search workspace with responsive filters and results.
 *
 * @returns Protected homepage search UI.
 */
export default function Home() {
  const [isFilterOpen, setIsFilterOpen] = useState(false);

  return (
    <div className="flex h-dvh w-full bg-background text-foreground">
      <Sidebar className="hidden h-dvh flex-shrink-0 md:flex" />
      <main id="main-content" className="flex-1 flex flex-col h-full overflow-hidden">
        <div className="sticky top-0 z-30 border-b bg-background/95 p-3 backdrop-blur sm:p-6">
          <div className="flex min-w-0 items-center gap-2 sm:gap-3">
            <Dialog open={isFilterOpen} onOpenChange={setIsFilterOpen}>
              <DialogTrigger asChild>
                <Button
                  variant="outline"
                  size="icon"
                  className="z-10 shrink-0 md:hidden"
                  aria-label="打开筛选器"
                >
                  <Menu className="h-5 w-5" />
                </Button>
              </DialogTrigger>
              <DialogContent className="left-0 top-0 h-dvh w-80 max-w-[calc(100vw-2rem)] translate-x-0 translate-y-0 gap-0 overflow-hidden rounded-none border-r p-0 shadow-lg md:hidden">
                <DialogHeader className="sr-only">
                  <DialogTitle>筛选器</DialogTitle>
                  <DialogDescription>选择数据库、领域、期刊和发表时间筛选文章。</DialogDescription>
                </DialogHeader>
                <Sidebar className="h-full w-full pt-8" />
              </DialogContent>
            </Dialog>
            <SearchBar className="min-w-0 flex-1 md:mx-auto md:max-w-4xl" />
          </div>
        </div>
        <div
          id="results-scroll-container"
          className="flex-1 overflow-y-auto p-6"
          style={{
            paddingBottom:
              'calc(6rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
          }}
        >
          <div className="max-w-4xl mx-auto w-full space-y-4">
            <AnnouncementsDialog />
            <ActiveFilterChips />
            <ResultsList />
          </div>
        </div>
      </main>
    </div>
  );
}
