'use client';

/**
 * Shared responsive workspace layout for article-oriented views.
 */

import { Menu } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { cn } from '@/lib/utils';

type WorkspaceShellProps = {
  children: ReactNode;
  contentClassName?: string;
  sidebar: ReactNode;
  sidebarDialogDescription: string;
  sidebarDialogTitle: string;
  sidebarOpenLabel: string;
  toolbar: ReactNode;
};

/**
 * Render a fixed desktop sidebar, mobile sidebar dialog, toolbar, and scrollable article area.
 *
 * @param props - Workspace slots, accessible sidebar labels, and optional content classes.
 * @returns Shared article-workspace shell.
 */
export function WorkspaceShell({
  children,
  contentClassName,
  sidebar,
  sidebarDialogDescription,
  sidebarDialogTitle,
  sidebarOpenLabel,
  toolbar,
}: WorkspaceShellProps) {
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);

  return (
    <div className="flex h-dvh w-full bg-background text-foreground">
      <div className="hidden h-dvh shrink-0 md:flex">{sidebar}</div>
      <main id="main-content" className="flex h-full min-w-0 flex-1 flex-col overflow-hidden">
        <div className="sticky top-0 z-30 border-b bg-background/95 p-3 backdrop-blur sm:p-6">
          <div className="flex min-w-0 items-center gap-2 sm:gap-3">
            <Dialog open={isSidebarOpen} onOpenChange={setIsSidebarOpen}>
              <DialogTrigger asChild>
                <Button
                  variant="outline"
                  size="icon"
                  className="z-10 shrink-0 md:hidden"
                  aria-label={sidebarOpenLabel}
                >
                  <Menu className="h-5 w-5" />
                </Button>
              </DialogTrigger>
              <DialogContent className="left-0 top-0 h-dvh w-80 max-w-[calc(100vw-2rem)] translate-x-0 translate-y-0 gap-0 overflow-hidden rounded-none border-r p-0 shadow-lg md:hidden">
                <DialogHeader className="sr-only">
                  <DialogTitle>{sidebarDialogTitle}</DialogTitle>
                  <DialogDescription>{sidebarDialogDescription}</DialogDescription>
                </DialogHeader>
                <div className="h-full w-full pt-8 [&>*]:h-full [&>*]:w-full">{sidebar}</div>
              </DialogContent>
            </Dialog>
            {toolbar}
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
          <div className={cn('mx-auto w-full max-w-4xl space-y-4', contentClassName)}>
            {children}
          </div>
        </div>
      </main>
    </div>
  );
}
