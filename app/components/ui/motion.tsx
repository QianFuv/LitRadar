'use client';

/**
 * Accessible application motion primitives and shared transition contracts.
 */

import {
  AnimatePresence,
  LazyMotion,
  MotionConfig,
  domAnimation,
  m,
  useReducedMotion,
  type AnimatePresenceProps,
  type Transition,
  type Variants,
} from 'motion/react';
import { createContext, useContext, useMemo, type PropsWithChildren, type ReactNode } from 'react';

export const MOTION_DURATION_SECONDS = {
  fast: 0.12,
  base: 0.18,
  slow: 0.22,
  exit: 0.14,
} as const;

export const MOTION_EASING = {
  enter: [0.16, 1, 0.3, 1],
  exit: [0.4, 0, 1, 1],
} as const;

export const FADE_VARIANTS = {
  hidden: { opacity: 0 },
  visible: { opacity: 1 },
  exit: { opacity: 0 },
} satisfies Variants;

export const FADE_UP_VARIANTS = {
  hidden: { opacity: 0, y: 6 },
  visible: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -4 },
} satisfies Variants;

export const COLLAPSE_VARIANTS = {
  hidden: { height: 0, opacity: 0 },
  visible: { height: 'auto', opacity: 1 },
  exit: { height: 0, opacity: 0 },
} satisfies Variants;

export const MotionArticle = m.article;
export const MotionDiv = m.div;
export const MotionForm = m.form;
export const MotionList = m.ul;
export const MotionListItem = m.li;
export const MotionParagraph = m.p;
export const MotionSection = m.section;
export const MotionSpan = m.span;

type MotionPhase = keyof typeof MOTION_EASING;
type ReducedMotionPreference = 'always' | 'never' | 'user';

type MotionProviderProps = {
  children: ReactNode;
  reducedMotion?: ReducedMotionPreference;
};

type MotionPreferenceContextValue = {
  isMotionReduced: boolean;
};

type MotionPresenceProps = PropsWithChildren<AnimatePresenceProps>;

const MotionPreferenceContext = createContext<MotionPreferenceContextValue>({
  isMotionReduced: false,
});

type MotionConfigurationProps = {
  children: ReactNode;
  isMotionReduced: boolean;
};

/**
 * Render the shared Motion contexts for one resolved preference.
 *
 * @param props - Application children and the resolved reduced-motion state.
 * @returns Configured lazy Motion boundary.
 */
function MotionConfiguration({ children, isMotionReduced }: MotionConfigurationProps) {
  const preferenceValue = useMemo(() => ({ isMotionReduced }), [isMotionReduced]);

  return (
    <MotionPreferenceContext.Provider value={preferenceValue}>
      <MotionConfig
        reducedMotion={isMotionReduced ? 'always' : 'never'}
        transition={
          isMotionReduced
            ? { duration: 0 }
            : { duration: MOTION_DURATION_SECONDS.base, ease: MOTION_EASING.enter }
        }
      >
        <LazyMotion features={domAnimation} strict>
          {children}
        </LazyMotion>
      </MotionConfig>
    </MotionPreferenceContext.Provider>
  );
}

/**
 * Resolve the operating-system preference before configuring Motion.
 *
 * @param props - Application children.
 * @returns Motion boundary that follows the user media preference.
 */
function UserMotionConfiguration({ children }: { children: ReactNode }) {
  const isMotionReduced = useReducedMotion() ?? false;

  return <MotionConfiguration isMotionReduced={isMotionReduced}>{children}</MotionConfiguration>;
}

/**
 * Provide the lazily loaded DOM animation feature set and user motion preference.
 *
 * @param props - Application children and an optional deterministic test override.
 * @returns Shared Motion configuration for the client application.
 */
export function MotionProvider({ children, reducedMotion = 'user' }: MotionProviderProps) {
  if (reducedMotion === 'user') {
    return <UserMotionConfiguration>{children}</UserMotionConfiguration>;
  }

  return (
    <MotionConfiguration isMotionReduced={reducedMotion === 'always'}>
      {children}
    </MotionConfiguration>
  );
}

/**
 * Coordinate keyed entry and exit lifecycles without animating the initial page render.
 *
 * @param props - Motion presence properties and keyed children.
 * @returns Presence boundary with hydration-safe initial behavior.
 */
export function MotionPresence({ initial = false, mode, ...props }: MotionPresenceProps) {
  const isMotionReduced = useIsMotionReduced();

  return <AnimatePresence initial={initial} mode={isMotionReduced ? 'sync' : mode} {...props} />;
}

/**
 * Read whether application motion is currently reduced.
 *
 * @returns True when spatial motion and transition delays must be removed.
 */
export function useIsMotionReduced(): boolean {
  return useContext(MotionPreferenceContext).isMotionReduced;
}

/**
 * Build a shared transition that becomes immediate under reduced motion.
 *
 * @param duration - Normal transition duration in seconds.
 * @param phase - Entry or exit easing phase.
 * @param delay - Optional normal-mode delay in seconds.
 * @returns Motion transition honoring the active reduced-motion preference.
 */
export function useMotionTransition(
  duration: number = MOTION_DURATION_SECONDS.base,
  phase: MotionPhase = 'enter',
  delay = 0,
): Transition {
  const isMotionReduced = useIsMotionReduced();

  return isMotionReduced
    ? { delay: 0, duration: 0 }
    : { delay, duration, ease: MOTION_EASING[phase] };
}
