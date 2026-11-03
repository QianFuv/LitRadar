'use client';

/**
 * Application motion roles built on Animate UI's motion tokens and reduced-motion contract.
 */

import {
  AnimatePresence,
  PresenceContext,
  motion,
  type AnimatePresenceProps,
  type Variants,
} from 'motion/react';
import {
  ReducedMotionConfig,
  ReducedMotionContext,
  blur,
  scale,
  spring,
  transition,
  useShouldReduceMotion,
  type ReducedMotion,
} from '@animate-ui/react/utils/motion';
import type { ComponentType, PropsWithChildren, ReactNode } from 'react';

const SHARP = 'blur(0px)';

/**
 * Two states of one short text or icon replacing each other in place (Animate UI rule B9, subtle level).
 */
export const SWAP_VARIANTS = {
  hidden: { opacity: 0, filter: blur.icon },
  visible: {
    opacity: 1,
    filter: SHARP,
    transition: transition.swap,
    transitionEnd: { filter: 'none' },
  },
  exit: { opacity: 0, filter: blur.icon, transition: transition.swap },
} satisfies Variants;

/**
 * Inline content arriving in a list or form: it rises 8px and fades in, then fades out in place.
 */
export const REVEAL_VARIANTS = {
  hidden: { opacity: 0, y: 8 },
  visible: { opacity: 1, y: 0, transition: { ...spring.reflow, opacity: transition.fade } },
  exit: { opacity: 0, transition: transition.fade },
} satisfies Variants;

/**
 * Whole panels that take turns in one place: the previous one blurs away before the next one clears in.
 */
export const PANEL_VARIANTS = {
  hidden: { opacity: 0, filter: blur.panel },
  visible: {
    opacity: 1,
    filter: SHARP,
    transition: transition.panelIn,
    transitionEnd: { filter: 'none' },
  },
  exit: { opacity: 0, filter: blur.panel, pointerEvents: 'none', transition: transition.panelOut },
} satisfies Variants;

/**
 * Content that unfolds and pushes what follows, both ways on the disclosure timing.
 */
export const DISCLOSURE_VARIANTS = {
  hidden: { height: 0, opacity: 0, filter: blur.disclosure, transition: transition.disclosure },
  visible: {
    height: 'auto',
    opacity: 1,
    filter: SHARP,
    transition: transition.disclosure,
    transitionEnd: { filter: 'none' },
  },
  exit: {
    height: 0,
    opacity: 0,
    filter: blur.disclosure,
    pointerEvents: 'none',
    transition: transition.disclosure,
  },
} satisfies Variants;

/**
 * A hand-built anchored card growing out of its anchor; set `transformOrigin` on the element.
 */
export const POPOVER_VARIANTS = {
  hidden: { opacity: 0, scale: scale.popover },
  visible: { opacity: 1, scale: 1, transition: spring.pop },
  exit: { opacity: 0, scale: scale.popover, transition: spring.pop },
} satisfies Variants;

/**
 * A chip or tag popping in from half its size and shrinking back out.
 */
export const CHIP_VARIANTS = {
  hidden: { opacity: 0, scale: scale.icon },
  visible: { opacity: 1, scale: 1, transition: spring.pop },
  exit: { opacity: 0, scale: scale.icon, pointerEvents: 'none', transition: transition.popOut },
} satisfies Variants;

/**
 * A list item: rows appended to a list already on screen clear in like a tab panel, and rows the
 * user removes fade out in place while shrinking slightly. Wrap the rows in `MotionPresence` so
 * the rows of the first render stay still.
 */
export const LIST_ITEM_VARIANTS = {
  hidden: { opacity: 0, filter: blur.panel },
  visible: {
    opacity: 1,
    scale: 1,
    filter: SHARP,
    transition: transition.panelIn,
    transitionEnd: { filter: 'none' },
  },
  exit: { opacity: 0, scale: scale.toast, pointerEvents: 'none', transition: transition.fade },
} satisfies Variants;

/**
 * Reduced-motion disclosure: the height snaps because unfolding is spatial movement, which
 * Motion's reduced-motion setting leaves running for non-transform values; the fade stays on
 * Animate UI's quick fade (rule A1).
 */
const REDUCED_DISCLOSURE_TRANSITION = { ...transition.fade, height: { duration: 0 } };

const REDUCED_DISCLOSURE_VARIANTS = {
  hidden: { ...DISCLOSURE_VARIANTS.hidden, transition: REDUCED_DISCLOSURE_TRANSITION },
  visible: { ...DISCLOSURE_VARIANTS.visible, transition: REDUCED_DISCLOSURE_TRANSITION },
  exit: { ...DISCLOSURE_VARIANTS.exit, transition: REDUCED_DISCLOSURE_TRANSITION },
} satisfies Variants;

/** Reduced-motion replacements for roles whose movement Motion cannot drop on its own. */
const REDUCED_VARIANTS = new Map<Variants, Variants>([
  [DISCLOSURE_VARIANTS, REDUCED_DISCLOSURE_VARIANTS],
]);

/**
 * Wrap a motion element so roles with non-transform movement switch to their reduced variants
 * while motion is reduced.
 *
 * @param Component - Motion element to wrap.
 * @returns Motion element that honors the reduced-motion contract for every role.
 */
function withReducedVariants<Props extends { variants?: Variants }>(
  Component: ComponentType<Props>,
) {
  /**
   * Render the motion element with the variants that match the reduced-motion preference.
   *
   * @param props - Motion element properties.
   * @returns Motion element.
   */
  function ReducedAwareMotion(props: Props) {
    const shouldReduceMotion = useShouldReduceMotion();
    const variants =
      shouldReduceMotion && props.variants
        ? (REDUCED_VARIANTS.get(props.variants) ?? props.variants)
        : props.variants;
    return <Component {...props} variants={variants} />;
  }
  return ReducedAwareMotion;
}

export const MotionArticle = withReducedVariants(motion.article);
export const MotionDiv = withReducedVariants(motion.div);
export const MotionForm = withReducedVariants(motion.form);
export const MotionParagraph = withReducedVariants(motion.p);
export const MotionSection = withReducedVariants(motion.section);
export const MotionSpan = withReducedVariants(motion.span);

type MotionProviderProps = {
  children: ReactNode;
  reducedMotion?: ReducedMotion;
};

type MotionPresenceProps = PropsWithChildren<AnimatePresenceProps>;

/**
 * Apply Animate UI's reduced-motion contract to the application tree.
 *
 * @param props - Application children and an optional deterministic preference override.
 * @returns Motion boundary that drops movement but keeps fades and blurs when motion is reduced.
 */
export function MotionProvider({ children, reducedMotion = 'user' }: MotionProviderProps) {
  return (
    <ReducedMotionContext value={reducedMotion}>
      <ReducedMotionConfig>{children}</ReducedMotionConfig>
    </ReducedMotionContext>
  );
}

/**
 * Coordinate keyed entry and exit lifecycles without animating the initial page render.
 *
 * @param props - Motion presence properties and keyed children.
 * @returns Presence boundary with hydration-safe initial behavior.
 */
export function MotionPresence({ initial = false, ...props }: MotionPresenceProps) {
  return <AnimatePresence initial={initial} {...props} />;
}

/**
 * Start a fresh motion subtree that ignores any enclosing presence.
 *
 * A `MotionPresence` keeps telling everything mounted under it to skip its `initial` state until
 * the presence itself re-renders, so popups and pressable controls that mount later would jump
 * straight to their resting state: menus lose their entrance and Animate UI presses start from an
 * unrendered transform. Wrapping such islands restores their own `initial`.
 *
 * @param props - Subtree that animates independently of its surroundings.
 * @returns Subtree outside every enclosing presence.
 */
export function PresenceBoundary({ children }: { children: ReactNode }) {
  return <PresenceContext value={null}>{children}</PresenceContext>;
}

/**
 * Read whether spatial motion is currently reduced.
 *
 * @returns True when movement must be skipped; fades and blurs still play.
 */
export function useIsMotionReduced(): boolean {
  return useShouldReduceMotion();
}
