'use client';

/**
 * Shared responsive frame for query-driven sectioned dialogs.
 */

import type { LucideIcon } from 'lucide-react';
import type { ReactNode, RefObject } from 'react';

import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';

/** One-shot marker used to return focus from a sectioned dialog to its persistent trigger. */
export const SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE = 'data-sectioned-dialog-return-focus';

/** One navigable category rendered by a sectioned dialog. */
export type SectionedDialogSectionDefinition<SectionId extends string> = {
  description: string;
  icon: LucideIcon;
  id: SectionId;
  label: string;
};

type SectionedDialogFrameProps<SectionId extends string> = {
  activeSection: SectionId;
  centerSubtitle: string;
  centerTitle: string;
  children: ReactNode;
  contentLabelSuffix: string;
  dialogDescription: string;
  isBusy?: boolean;
  isDismissBlocked?: boolean;
  isNavigationDisabled?: boolean;
  navigationLabel: string;
  onOpenChange: (open: boolean) => void;
  onSelectSection: (section: SectionId) => void;
  onSessionClosed: () => void;
  open: boolean;
  returnFocusRef: RefObject<HTMLElement | null>;
  sections: readonly [
    SectionedDialogSectionDefinition<SectionId>,
    ...SectionedDialogSectionDefinition<SectionId>[],
  ];
};

type SectionedDialogNavigationProps<SectionId extends string> = {
  activeSection: SectionId;
  className?: string;
  isDisabled: boolean;
  label: string;
  onSelect: (section: SectionId) => void;
  sections: readonly SectionedDialogSectionDefinition<SectionId>[];
};

/**
 * Resolve a stable focus target when a dialog opens from a transient menu item.
 *
 * @param activeElement - Element focused immediately before Dialog auto-focus, if any.
 * @returns A marked trigger, controlling menu trigger, active element, or null.
 */
function resolveReturnFocusTarget(activeElement: HTMLElement | null): HTMLElement | null {
  const markedTarget = document.querySelector<HTMLElement>(
    `[${SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE}]`,
  );
  markedTarget?.removeAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);
  if (markedTarget?.isConnected) {
    return markedTarget;
  }
  if (!activeElement) {
    return null;
  }

  const menu = activeElement.closest<HTMLElement>('[role="menu"]');
  if (!menu?.id) {
    return activeElement;
  }

  const menuTrigger = Array.from(document.querySelectorAll<HTMLElement>('[aria-controls]')).find(
    (element) => element.getAttribute('aria-controls') === menu.id,
  );
  return menuTrigger ?? activeElement;
}

/**
 * Render section navigation for desktop or mobile layouts.
 *
 * @param props - Section definitions, active state, selection action, and layout class.
 * @returns Accessible category navigation.
 */
function SectionedDialogNavigation<SectionId extends string>({
  activeSection,
  className,
  isDisabled,
  label,
  onSelect,
  sections,
}: SectionedDialogNavigationProps<SectionId>) {
  return (
    <nav aria-label={label} className={className}>
      {sections.map((section) => {
        const Icon = section.icon;
        const isActive = section.id === activeSection;
        return (
          <button
            key={section.id}
            type="button"
            aria-current={isActive ? 'page' : undefined}
            disabled={isDisabled}
            className={cn(
              'flex shrink-0 items-center gap-3 rounded-md px-3 py-2.5 text-left text-sm font-medium outline-none transition-colors hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50',
              isActive && 'bg-accent text-accent-foreground',
            )}
            onClick={() => onSelect(section.id)}
          >
            <Icon className="size-4" />
            <span>{section.label}</span>
          </button>
        );
      })}
    </nav>
  );
}

/**
 * Render a responsive category dialog with shared navigation, content, and focus behavior.
 *
 * @param props - Dialog identity, sections, lifecycle handlers, and active content.
 * @returns Controlled sectioned dialog frame.
 */
export function SectionedDialogFrame<SectionId extends string>({
  activeSection,
  centerSubtitle,
  centerTitle,
  children,
  contentLabelSuffix,
  dialogDescription,
  isBusy = false,
  isDismissBlocked = false,
  isNavigationDisabled = false,
  navigationLabel,
  onOpenChange,
  onSelectSection,
  onSessionClosed,
  open,
  returnFocusRef,
  sections,
}: SectionedDialogFrameProps<SectionId>) {
  const activeDefinition = sections.find((section) => section.id === activeSection) ?? sections[0];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-busy={isBusy || undefined}
        className="flex h-dvh w-screen max-w-none translate-x-[-50%] translate-y-[-50%] gap-0 overflow-hidden rounded-none border-0 p-0 shadow-none [&>[data-slot=dialog-close]]:top-5 [&>[data-slot=dialog-close]]:right-5 [&>[data-slot=dialog-close]]:flex [&>[data-slot=dialog-close]]:size-10 [&>[data-slot=dialog-close]]:items-center [&>[data-slot=dialog-close]]:justify-center [&>[data-slot=dialog-close]]:rounded-md [&>[data-slot=dialog-close]]:border [&>[data-slot=dialog-close]]:bg-background [&>[data-slot=dialog-close]]:opacity-100 [&>[data-slot=dialog-close]]:hover:bg-accent md:h-[min(90dvh,52rem)] md:w-[min(calc(100vw-2rem),72rem)] md:max-w-6xl md:rounded-lg md:border md:shadow-lg md:[&>[data-slot=dialog-close]]:right-auto md:[&>[data-slot=dialog-close]]:left-5"
        onOpenAutoFocus={() => {
          if (!returnFocusRef.current) {
            const activeElement =
              document.activeElement instanceof HTMLElement &&
              document.activeElement !== document.body
                ? document.activeElement
                : null;
            returnFocusRef.current = resolveReturnFocusTarget(activeElement);
          }
        }}
        onCloseAutoFocus={(event) => {
          const focusTarget = returnFocusRef.current;
          returnFocusRef.current = null;
          if (focusTarget?.isConnected) {
            event.preventDefault();
            focusTarget.focus();
          }
          onSessionClosed();
        }}
        onEscapeKeyDown={(event) => {
          if (isDismissBlocked) {
            event.preventDefault();
          }
        }}
        onInteractOutside={(event) => {
          if (isDismissBlocked) {
            event.preventDefault();
          }
        }}
      >
        <DialogTitle className="sr-only">{centerTitle}</DialogTitle>
        <DialogDescription className="sr-only">{dialogDescription}</DialogDescription>

        <aside className="hidden w-60 shrink-0 flex-col border-r bg-muted/20 px-3 pb-5 pt-16 md:flex">
          <div className="px-3 pb-5">
            <div className="text-lg font-semibold">{centerTitle}</div>
            <p className="mt-1 text-xs text-muted-foreground">{centerSubtitle}</p>
          </div>
          <SectionedDialogNavigation
            activeSection={activeSection}
            className="flex flex-col gap-1"
            isDisabled={isNavigationDisabled}
            label={navigationLabel}
            onSelect={onSelectSection}
            sections={sections}
          />
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="shrink-0 border-b bg-background px-5 pb-4 pt-5 pr-14 md:px-8 md:py-5 md:pr-8">
            <div className="md:hidden">
              <div className="text-lg font-semibold">{centerTitle}</div>
              <SectionedDialogNavigation
                activeSection={activeSection}
                className="-mx-1 mt-4 flex gap-1 overflow-x-auto px-1 pb-1"
                isDisabled={isNavigationDisabled}
                label={navigationLabel}
                onSelect={onSelectSection}
                sections={sections}
              />
            </div>
            <div className="mt-4 md:mt-0">
              <h2 className="text-xl font-semibold">{activeDefinition.label}</h2>
              <p className="mt-1 text-sm text-muted-foreground">{activeDefinition.description}</p>
            </div>
          </header>

          <div
            role="region"
            aria-label={`${activeDefinition.label}${contentLabelSuffix}`}
            className="min-h-0 flex-1 overflow-y-auto px-5 py-6 pb-[calc(1.5rem+var(--safe-area-inset-bottom,env(safe-area-inset-bottom,0px)))] md:px-8"
          >
            {children}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
