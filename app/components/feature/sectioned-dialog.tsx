'use client';

/**
 * Shared responsive frame for query-driven sectioned dialogs.
 */

import { ArrowLeft, X, type LucideIcon } from 'lucide-react';
import { useRef, type ReactNode, type RefObject } from 'react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
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
  centerIcon: LucideIcon;
  /** Optional content pinned to the bottom of the desktop category sidebar. */
  sidebarFooter?: ReactNode;
  /** Content column width: `narrow` for forms, `wide` for tables and dashboards. */
  contentWidth?: 'narrow' | 'wide';
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
 * Render section navigation as horizontal pills on phones and a vertical list on wider screens.
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
            data-section-active={isActive ? 'true' : 'false'}
            disabled={isDisabled}
            className={cn(
              'motion-control flex h-9 shrink-0 items-center gap-2 rounded-full px-3.5 text-[13px] whitespace-nowrap outline-none transition-[background-color,color,box-shadow] focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 md:h-10 md:w-full md:gap-3 md:rounded-lg md:px-3 md:text-sm',
              isActive
                ? 'bg-foreground font-bold text-background md:bg-accent md:text-foreground'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground active:bg-accent-pressed',
            )}
            onClick={() => onSelect(section.id)}
          >
            <Icon className="size-4 shrink-0" aria-hidden="true" />
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
  centerIcon: CenterIcon,
  sidebarFooter,
  contentWidth = 'narrow',
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
  const headerTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const contentRef = useRef<HTMLDivElement>(null);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        placement="takeover"
        aria-busy={isBusy || undefined}
        className="flex flex-col gap-0 overflow-hidden bg-background focus-visible:ring-0 md:flex-row"
        ref={contentRef}
        onOpenAutoFocus={(event) => {
          if (!returnFocusRef.current) {
            const activeElement =
              document.activeElement instanceof HTMLElement &&
              document.activeElement !== document.body
                ? document.activeElement
                : null;
            returnFocusRef.current = resolveReturnFocusTarget(activeElement);
          }
          event.preventDefault();
          contentRef.current?.focus({ preventScroll: true });
        }}
        onCloseAutoFocus={(event) => {
          if (open) {
            event.preventDefault();
            return;
          }
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

        <aside
          data-slot="sectioned-dialog-sidebar"
          className="relative shrink-0 border-b bg-background pt-3 pb-2 md:flex md:w-64 md:flex-col md:border-r md:border-b-0 md:pt-4 md:pb-0 lg:w-72"
        >
          <DialogClose asChild>
            <Button
              type="button"
              variant="ghost"
              static
              aria-label="关闭"
              className="absolute top-1.5 right-2 size-11 p-0 text-muted-foreground hover:text-foreground md:static md:mx-3 md:h-10 md:w-auto md:justify-start md:gap-1.5 md:self-start md:px-2.5"
            >
              <X className="size-5 md:hidden" aria-hidden="true" />
              <ArrowLeft className="hidden size-4 md:block" aria-hidden="true" />
              <span className="hidden text-[13px] md:inline">返回</span>
            </Button>
          </DialogClose>
          <div className="flex items-center gap-3 px-4 pr-14 md:px-6 md:pt-6 md:pr-6 md:pb-5">
            <span
              aria-hidden="true"
              className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background md:hidden"
            >
              <CenterIcon className="size-4" />
            </span>
            <div className="min-w-0">
              <p className="truncate text-base leading-6 font-bold md:text-xl">{centerTitle}</p>
              <p className="hidden text-xs leading-5 text-muted-foreground md:mt-1 md:block">
                {centerSubtitle}
              </p>
            </div>
          </div>
          <div className="relative mt-3 md:mt-0" data-mobile-overflow-cue="true">
            <SectionedDialogNavigation
              activeSection={activeSection}
              className="scrollbar-none fade-x-edges flex gap-1.5 overflow-x-auto px-4 md:flex-col md:gap-1 md:overflow-visible md:px-3 md:[mask-image:none]"
              isDisabled={isNavigationDisabled}
              label={navigationLabel}
              onSelect={onSelectSection}
              sections={sections}
            />
          </div>
          {sidebarFooter && (
            <div className="mt-auto hidden border-t px-4 py-4 md:block">{sidebarFooter}</div>
          )}
        </aside>

        <div
          data-slot="sectioned-dialog-scroll"
          className="min-h-0 min-w-0 flex-1 scroll-pb-28 overflow-y-auto"
        >
          <div
            className={cn(
              'mx-auto w-full px-5 pt-6 pb-[calc(2rem+var(--safe-area-inset-bottom,env(safe-area-inset-bottom,0px)))] md:px-10 md:pt-12',
              contentWidth === 'wide' ? 'max-w-5xl' : 'max-w-3xl',
            )}
          >
            <div className="mb-6 border-b pb-6 md:mb-8">
              <h2 className="sr-only">{activeDefinition.label}</h2>
              <MotionPresence mode="wait">
                <MotionDiv
                  key={activeDefinition.id}
                  aria-hidden="true"
                  data-motion-section-header={activeDefinition.id}
                  variants={FADE_UP_VARIANTS}
                  initial="hidden"
                  animate="visible"
                  exit={{ opacity: 0, pointerEvents: 'none', y: -3 }}
                  transition={headerTransition}
                >
                  <div className="text-2xl font-bold tracking-tight">{activeDefinition.label}</div>
                  <p className="mt-1.5 text-[13px] text-muted-foreground">
                    {activeDefinition.description}
                  </p>
                </MotionDiv>
              </MotionPresence>
            </div>
            <div
              role="region"
              aria-label={`${activeDefinition.label}${contentLabelSuffix}`}
              data-slot="sectioned-dialog-panel"
            >
              {children}
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
