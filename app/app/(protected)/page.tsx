'use client';

import { Sidebar } from '@/components/feature/sidebar';
import { SearchBar } from '@/components/feature/search-bar';
import { ResultsList } from '@/components/feature/results-list';
import { WeeklyUpdatesFab } from '@/components/feature/weekly-updates-fab';
import { AnnouncementsDialog } from '@/components/announcements-dialog';
import { Button } from '@/components/ui/button';
import { Menu, X } from 'lucide-react';
import { useState } from 'react';

export default function Home() {
  const [isFilterOpen, setIsFilterOpen] = useState(false);

  return (
    <div className="flex h-screen w-full bg-background text-foreground">
      <Sidebar className="hidden md:flex flex-shrink-0 h-screen" />
      <main id="main-content" className="flex-1 flex flex-col h-full overflow-hidden">
        <div className="sticky top-0 z-30 border-b bg-background/95 p-3 backdrop-blur sm:p-6">
          <div className="flex min-w-0 items-center gap-2 sm:gap-3">
            <Button
              variant="outline"
              size="icon"
              className="z-10 shrink-0 md:hidden"
              aria-label="打开筛选器"
              onClick={() => setIsFilterOpen(true)}
            >
              <Menu className="h-5 w-5" />
            </Button>
            <SearchBar className="min-w-0 flex-1 md:mx-auto md:max-w-4xl" />
          </div>
        </div>
        <div id="results-scroll-container" className="flex-1 overflow-y-auto p-6 scroll-smooth">
          <div className="max-w-4xl mx-auto w-full space-y-4">
            <AnnouncementsDialog />
            <ResultsList />
          </div>
        </div>
        <WeeklyUpdatesFab />
      </main>
      {isFilterOpen && (
        <>
          <button
            type="button"
            className="fixed inset-0 z-50 bg-black/50 md:hidden"
            aria-label="关闭筛选器"
            onClick={() => setIsFilterOpen(false)}
          />
          <div
            role="dialog"
            aria-modal="true"
            aria-label="筛选器"
            className="fixed left-0 top-0 z-50 h-dvh w-80 max-w-[calc(100vw-2rem)] overflow-hidden border-r bg-background shadow-lg md:hidden"
          >
            <Button
              variant="ghost"
              size="icon-sm"
              className="absolute right-3 top-3 z-20 bg-background/95 shadow-vercel-ring"
              aria-label="关闭筛选器"
              onClick={() => setIsFilterOpen(false)}
            >
              <X className="h-4 w-4" />
            </Button>
            <Sidebar className="h-full w-full pt-8" />
          </div>
        </>
      )}
    </div>
  );
}
