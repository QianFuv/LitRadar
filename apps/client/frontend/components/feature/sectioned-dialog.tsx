/**
 * Shared responsive frame for query-driven sectioned dialogs.
 */

import { Highlight, HighlightItem } from '@animate-ui/react/primitives/effects/highlight';
import { ScrollFade } from '@animate-ui/react/primitives/effects/scroll-fade';
import {
  ArrowLeft,
  IconTriggerProvider,
  useIconTrigger,
  X,
  type AppIcon,
} from '@/components/ui/animated-icons';
import {
  useLayoutEffect,
  useRef,
  type ComponentProps,
  type ReactNode,
  type RefObject,
} from 'react';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';
import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';
import { attachHorizontalWheelScroll } from '@/lib/horizontal-wheel-scroll';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

/** One-shot marker used to return focus from a sectioned dialog to its persistent trigger. */
export const SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE = 'data-sectioned-dialog-return-focus';

/** One navigable category rendered by a sectioned dialog. */
export type SectionedDialogSectionDefinition<SectionId extends string> = {
  description: string;
  icon: AppIcon;
  id: SectionId;
  label: string;
};

type SectionedDialogFrameProps<SectionId extends string> = {
  activeSection: SectionId;
  centerIcon: AppIcon;
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
  xstyle?: stylex.StyleXStyles;
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
 * Render one category button whose icon plays on hover, on touch, and when it becomes active.
 *
 * @param props - Button properties and whether the category is active.
 * @returns Category button that drives its animated icon.
 */
function SectionNavigationButton({
  isActive,
  ...props
}: ComponentProps<'button'> & { isActive: boolean }) {
  const { isTriggered, triggerHandlers } = useIconTrigger<HTMLButtonElement>(props, isActive);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <button
        type="button"
        {...props}
        {...triggerHandlers}
        aria-current={isActive ? 'page' : undefined}
        data-section-active={isActive ? 'true' : 'false'}
      />
    </IconTriggerProvider>
  );
}

/**
 * Render section navigation as horizontal pills on phones and a vertical list on wider screens,
 * with Animate UI's highlight sliding between the active categories.
 *
 * @param props - Section definitions, active state, selection action, and layout styles.
 * @returns Accessible category navigation.
 */
function SectionedDialogNavigation<SectionId extends string>({
  activeSection,
  xstyle,
  isDisabled,
  label,
  onSelect,
  sections,
}: SectionedDialogNavigationProps<SectionId>) {
  return (
    <ScrollFade orientation="horizontal" shrink={0} asChild>
      <nav ref={attachHorizontalWheelScroll} aria-label={label} {...stylex.props(xstyle)}>
        <Highlight
          controlledItems
          value={activeSection}
          click={false}
          {...stylex.props(styles.sectionedDialogNavigationHighlight)}
        >
          {sections.map((section) => {
            const Icon = section.icon;
            const isActive = section.id === activeSection;
            return (
              <HighlightItem
                key={section.id}
                value={section.id}
                {...stylex.props(styles.sectionedDialogNavigationHighlightItem)}
              >
                <SectionNavigationButton
                  isActive={isActive}
                  disabled={isDisabled}
                  {...stylex.props(
                    styles.sectionedDialogNavigationSectionNavigationButton,
                    isActive
                      ? styles.sectionedDialogNavigationSectionNavigationButtonActive
                      : styles.sectionedDialogNavigationSectionNavigationButtonNotActive,
                  )}
                  onClick={() => onSelect(section.id)}
                >
                  <Icon
                    {...stylex.props(styles.sectionedDialogNavigationIcon)}
                    aria-hidden="true"
                  />
                  <span>{section.label}</span>
                </SectionNavigationButton>
              </HighlightItem>
            );
          })}
        </Highlight>
      </nav>
    </ScrollFade>
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
  const contentRef = useRef<HTMLDivElement>(null);
  const isOpenRef = useRef(open);
  useLayoutEffect(() => {
    isOpenRef.current = open;
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        placement="takeover"
        aria-busy={isBusy || undefined}
        xstyle={styles.sectionedDialogFrameDialogContent}
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
          if (isOpenRef.current) {
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
        <DialogTitle xstyle={styles.sectionedDialogFrameDialogTitle}>{centerTitle}</DialogTitle>
        <DialogDescription xstyle={styles.sectionedDialogFrameDialogTitle}>
          {dialogDescription}
        </DialogDescription>

        <aside
          data-slot="sectioned-dialog-sidebar"
          {...stylex.props(styles.sectionedDialogSidebar)}
        >
          <DialogClose asChild>
            <Button
              type="button"
              variant="ghost"
              static
              aria-label="关闭"
              xstyle={styles.sectionedDialogFrameButton}
            >
              <X {...stylex.props(styles.sectionedDialogFrameX)} aria-hidden="true" />
              <ArrowLeft
                {...stylex.props(styles.sectionedDialogFrameArrowLeft)}
                aria-hidden="true"
              />
              <span {...stylex.props(styles.sectionedDialogFrameSpan)}>返回</span>
            </Button>
          </DialogClose>
          <div {...stylex.props(styles.sectionedDialogFrameDiv)}>
            <span aria-hidden="true" {...stylex.props(styles.sectionedDialogFrameSpan2)}>
              <CenterIcon {...stylex.props(styles.sectionedDialogFrameCenterIcon)} />
            </span>
            <div {...stylex.props(styles.sectionedDialogFrameDiv2)}>
              <p {...stylex.props(styles.sectionedDialogFrameP)}>{centerTitle}</p>
              <p {...stylex.props(styles.sectionedDialogFrameP2)}>{centerSubtitle}</p>
            </div>
          </div>
          <div {...stylex.props(styles.sectionedDialogFrameDiv3)} data-mobile-overflow-cue="true">
            <SectionedDialogNavigation
              activeSection={activeSection}
              xstyle={styles.sectionedDialogFrameSectionedDialogNavigation}
              isDisabled={isNavigationDisabled}
              label={navigationLabel}
              onSelect={onSelectSection}
              sections={sections}
            />
          </div>
          {sidebarFooter && (
            <div {...stylex.props(styles.sectionedDialogFrameDiv4)}>{sidebarFooter}</div>
          )}
        </aside>

        <div data-slot="sectioned-dialog-scroll" {...stylex.props(styles.sectionedDialogScroll)}>
          <div
            {...stylex.props(
              styles.sectionedDialogFrameDiv5,
              contentWidth === 'wide'
                ? styles.sectionedDialogFrameDivWide
                : styles.sectionedDialogFrameDivNotWide,
            )}
          >
            <div {...stylex.props(styles.sectionedDialogFrameDiv6)}>
              <h2 {...stylex.props(styles.sectionedDialogFrameDialogTitle)}>
                {activeDefinition.label}
              </h2>
              <MotionPresence mode="wait">
                <MotionDiv
                  key={activeDefinition.id}
                  aria-hidden="true"
                  data-motion-section-header={activeDefinition.id}
                  variants={PANEL_VARIANTS}
                  initial="hidden"
                  animate="visible"
                  exit="exit"
                >
                  <div {...stylex.props(styles.sectionedDialogFrameDiv7)}>
                    {activeDefinition.label}
                  </div>
                  <p {...stylex.props(styles.sectionedDialogFrameP3)}>
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

const styles = stylex.create({
  sectionedDialogNavigationHighlight: {
    borderRadius: {
      default: 'calc(infinity * 1px)',
      '@media (min-width: 48rem)': '8px',
    },
    backgroundColor: {
      default: colors.foreground,
      '@media (min-width: 48rem)': colors.accent,
    },
  },
  sectionedDialogNavigationHighlightItem: {
    flexShrink: 0,
    width: {
      default: null,
      '@media (min-width: 48rem)': '100%',
    },
  },
  sectionedDialogNavigationSectionNavigationButton: {
    display: 'flex',
    height: {
      default: '2.25rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    flexShrink: 0,
    alignItems: 'center',
    gap: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    borderRadius: {
      default: 'calc(infinity * 1px)',
      '@media (min-width: 48rem)': '8px',
    },
    paddingInline: {
      default: '0.875rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    fontSize: {
      default: '13px',
      '@media (min-width: 48rem)': '0.875rem',
    },
    whiteSpace: 'nowrap',
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
    pointerEvents: {
      default: null,
      ':disabled': 'none',
    },
    opacity: {
      default: null,
      ':disabled': '50%',
    },
    width: {
      default: null,
      '@media (min-width: 48rem)': '100%',
    },
    lineHeight: {
      default: null,
      '@media (min-width: 48rem)': lineHeights.textSm,
    },
  },
  sectionedDialogNavigationSectionNavigationButtonActive: {
    fontWeight: 700,
    color: {
      default: colors.background,
      '@media (min-width: 48rem)': colors.foreground,
    },
  },
  sectionedDialogNavigationSectionNavigationButtonNotActive: {
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  sectionedDialogNavigationIcon: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
  },
  sectionedDialogFrameDialogContent: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 48rem)': 'row',
    },
    gap: '0px',
    overflow: 'hidden',
    backgroundColor: colors.background,
    boxShadow: shadows.overlay,
  },
  sectionedDialogFrameDialogTitle: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  sectionedDialogSidebar: {
    position: 'relative',
    flexShrink: 0,
    borderBottomStyle: {
      default: 'solid',
      '@media (min-width: 48rem)': 'solid',
    },
    borderBottomWidth: {
      default: '1px',
      '@media (min-width: 48rem)': '0px',
    },
    backgroundColor: colors.background,
    paddingTop: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '1rem',
    },
    paddingBottom: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '0px',
    },
    display: {
      default: null,
      '@media (min-width: 48rem)': 'flex',
    },
    width: {
      default: null,
      '@media (min-width: 48rem)': '16rem',
      '@media (min-width: 64rem)': '18rem',
    },
    flexDirection: {
      default: null,
      '@media (min-width: 48rem)': 'column',
    },
    borderRightStyle: {
      default: null,
      '@media (min-width: 48rem)': 'solid',
    },
    borderRightWidth: {
      default: null,
      '@media (min-width: 48rem)': '1px',
    },
  },
  sectionedDialogFrameButton: {
    position: {
      default: 'absolute',
      '@media (min-width: 48rem)': 'static',
    },
    top: '0.375rem',
    right: '0.5rem',
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': 'auto',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    padding: '0px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    marginInline: {
      default: null,
      '@media (min-width: 48rem)': '0.75rem',
    },
    justifyContent: {
      default: 'center',
      '@media (min-width: 48rem)': 'flex-start',
    },
    gap: {
      default: '0.5rem',
      '@media (min-width: 48rem)': '0.375rem',
    },
    alignSelf: {
      default: null,
      '@media (min-width: 48rem)': 'flex-start',
    },
    paddingInline: {
      default: null,
      ':has( > svg)': '0.75rem',
      '@media (min-width: 48rem)': {
        default: '0.625rem',
        ':has( > svg)': '0.75rem',
      },
    },
  },
  sectionedDialogFrameX: {
    width: '1.25rem',
    height: '1.25rem',
    display: {
      default: null,
      '@media (min-width: 48rem)': 'none',
    },
  },
  sectionedDialogFrameArrowLeft: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'block',
    },
    width: '1rem',
    height: '1rem',
  },
  sectionedDialogFrameSpan: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'inline',
    },
    fontSize: '13px',
  },
  sectionedDialogFrameDiv: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
    paddingRight: {
      default: '3.5rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
    paddingTop: {
      default: null,
      '@media (min-width: 48rem)': '1.5rem',
    },
    paddingBottom: {
      default: null,
      '@media (min-width: 48rem)': '1.25rem',
    },
  },
  sectionedDialogFrameSpan2: {
    display: {
      default: 'flex',
      '@media (min-width: 48rem)': 'none',
    },
    width: '2rem',
    height: '2rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.foreground,
    color: colors.background,
  },
  sectionedDialogFrameCenterIcon: {
    width: '1rem',
    height: '1rem',
  },
  sectionedDialogFrameDiv2: {
    minWidth: '0px',
  },
  sectionedDialogFrameP: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: {
      default: '1rem',
      '@media (min-width: 48rem)': '1.25rem',
    },
    lineHeight: {
      default: '1.5rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
    fontWeight: 700,
  },
  sectionedDialogFrameP2: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'block',
    },
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
    marginTop: {
      default: null,
      '@media (min-width: 48rem)': '0.25rem',
    },
  },
  sectionedDialogFrameDiv3: {
    position: 'relative',
    marginTop: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '0px',
    },
  },
  sectionedDialogFrameSectionedDialogNavigation: {
    display: 'flex',
    gap: {
      default: '0.375rem',
      '@media (min-width: 48rem)': '0.25rem',
    },
    overflowX: {
      default: 'auto',
      '@media (min-width: 48rem)': 'visible',
    },
    overflowY: {
      default: null,
      '@media (min-width: 48rem)': 'visible',
    },
    paddingInline: {
      default: '1rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    scrollbarWidth: 'none',
    flexDirection: {
      default: null,
      '@media (min-width: 48rem)': 'column',
    },
    maskImage: {
      default: null,
      '@media (min-width: 48rem)': 'none',
    },
    '::-webkit-scrollbar': {
      display: 'none',
    },
  },
  sectionedDialogFrameDiv4: {
    marginTop: 'auto',
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'block',
    },
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingInline: '1rem',
    paddingBlock: '1rem',
  },
  sectionedDialogScroll: {
    minHeight: '0px',
    minWidth: '0px',
    flex: '1',
    scrollPaddingBottom: '7rem',
    overflowY: 'auto',
  },
  sectionedDialogFrameDiv5: {
    marginInline: 'auto',
    width: '100%',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 48rem)': '2.5rem',
    },
    paddingTop: {
      default: '1.5rem',
      '@media (min-width: 48rem)': '3rem',
    },
    paddingBottom: 'calc(2rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom,0px)))',
  },
  sectionedDialogFrameDivWide: {
    maxWidth: '64rem',
  },
  sectionedDialogFrameDivNotWide: {
    maxWidth: '48rem',
  },
  sectionedDialogFrameDiv6: {
    marginBottom: {
      default: '1.5rem',
      '@media (min-width: 48rem)': '2rem',
    },
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingBottom: '1.5rem',
  },
  sectionedDialogFrameDiv7: {
    fontSize: '1.5rem',
    lineHeight: lineHeights.text2xl,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  sectionedDialogFrameP3: {
    marginTop: '0.375rem',
    fontSize: '13px',
    color: colors.mutedForeground,
  },
});
