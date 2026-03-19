import { Sidebar } from "@/components/feature/sidebar";
import { SearchBar } from "@/components/feature/search-bar";
import { ResultsList } from "@/components/feature/results-list";
import { WeeklyUpdatesFab } from "@/components/feature/weekly-updates-fab";
import { AnnouncementsDialog } from "@/components/announcements-dialog";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Menu } from "lucide-react";

export default function Home() {
  return (
    <div className="flex h-screen w-full bg-background text-foreground">
      <Sidebar className="hidden md:flex flex-shrink-0 h-screen" />
      <main className="flex-1 flex flex-col h-full overflow-hidden">
        <div className="p-6 border-b bg-background/95 backdrop-blur z-10 sticky top-0">
          <div className="flex items-center gap-3">
            <Dialog>
              <DialogTrigger asChild>
                <Button
                  variant="outline"
                  size="icon"
                  className="md:hidden shrink-0"
                  aria-label="打开筛选器"
                >
                  <Menu className="h-5 w-5" />
                </Button>
              </DialogTrigger>
              <DialogContent
                className="md:hidden h-full w-[85vw] max-w-xs p-0 gap-0 rounded-none left-0 top-0 translate-x-0 translate-y-0"
                showCloseButton
              >
                <DialogTitle className="sr-only">筛选器</DialogTitle>
                <Sidebar className="w-full h-full" />
              </DialogContent>
            </Dialog>
            <SearchBar className="max-w-4xl mx-auto w-full" />
          </div>
        </div>
        <div
          id="results-scroll-container"
          className="flex-1 overflow-y-auto p-6 scroll-smooth"
        >
          <div className="max-w-4xl mx-auto w-full space-y-4">
            <AnnouncementsDialog />
            <ResultsList />
          </div>
        </div>
        <WeeklyUpdatesFab />
      </main>
    </div>
  );
}
