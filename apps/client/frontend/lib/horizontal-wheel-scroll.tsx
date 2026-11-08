/**
 * Let a vertical mouse wheel scroll single-row horizontal strips such as filter pills and tabs.
 */

/** Pixel height of one wheel line, used when the browser reports deltas in lines. */
const WHEEL_LINE_HEIGHT = 16;

/** Share of the remaining distance covered each frame while a strip glides to its target. */
const GLIDE_FACTOR = 0.22;

/** Distance in pixels below which a gliding strip snaps onto its target. */
const GLIDE_SNAP_DISTANCE = 0.5;

type StripGlide = {
  /** Scroll offset the strip is gliding towards. */
  target: number;
  /** Pending animation frame, or null when the strip is at rest. */
  frame: number | null;
};

/** In-flight glides per strip, so quick wheel ticks add up instead of restarting. */
const STRIP_GLIDES = new WeakMap<HTMLElement, StripGlide>();

/**
 * Move a strip one frame closer to its glide target, scheduling the next frame until it arrives.
 *
 * @param element - Gliding strip.
 * @param glide - Glide state of the strip.
 */
function stepGlide(element: HTMLElement, glide: StripGlide): void {
  const distance = glide.target - element.scrollLeft;
  if (Math.abs(distance) <= GLIDE_SNAP_DISTANCE) {
    element.scrollLeft = glide.target;
    glide.frame = null;
    return;
  }
  element.scrollLeft += distance * GLIDE_FACTOR;
  glide.frame = requestAnimationFrame(() => stepGlide(element, glide));
}

/**
 * Scroll a horizontal strip by a vertical wheel gesture when the strip can still move that way.
 *
 * The strip glides to its new offset; consecutive ticks extend the same glide. Horizontal
 * gestures (trackpads, shift+wheel), pinch zoom, strips without overflow, and strips already
 * heading for the edge in the wheel's direction are left to the browser, so the page keeps
 * scrolling instead of the pointer getting stuck on the strip.
 *
 * @param element - Horizontally scrollable strip.
 * @param event - Wheel event received by the strip.
 * @returns Whether the wheel was consumed to scroll the strip.
 */
export function scrollStripOnWheel(element: HTMLElement, event: WheelEvent): boolean {
  if (event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return false;
  const maxScrollLeft = element.scrollWidth - element.clientWidth;
  if (maxScrollLeft <= 0) return false;

  const delta =
    event.deltaMode === WheelEvent.DOM_DELTA_LINE
      ? event.deltaY * WHEEL_LINE_HEIGHT
      : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
        ? event.deltaY * element.clientWidth
        : event.deltaY;
  const glide = STRIP_GLIDES.get(element);
  const origin = glide?.frame != null ? glide.target : element.scrollLeft;
  const target = Math.min(maxScrollLeft, Math.max(0, origin + delta));
  if (target === origin) return false;

  event.preventDefault();
  const activeGlide = glide ?? { target, frame: null };
  activeGlide.target = target;
  STRIP_GLIDES.set(element, activeGlide);
  if (activeGlide.frame === null) stepGlide(element, activeGlide);
  return true;
}

/** Strips that already listen for wheel input, so repeated ref calls never stack listeners. */
const ATTACHED_STRIPS = new WeakSet<HTMLElement>();

/**
 * Wheel listener shared by every strip.
 *
 * @param event - Wheel event received by a strip.
 */
function handleStripWheel(event: WheelEvent): void {
  scrollStripOnWheel(event.currentTarget as HTMLElement, event);
}

/**
 * Callback ref that turns vertical wheel input into horizontal scrolling for one strip.
 *
 * Safe to call on every render, including through merged refs that drop React's ref cleanup: each
 * element gets one listener, which goes away with the element.
 *
 * @param element - Horizontally scrollable strip, or null when it unmounts.
 */
export function attachHorizontalWheelScroll(element: HTMLElement | null): void {
  if (!element || ATTACHED_STRIPS.has(element)) return;
  ATTACHED_STRIPS.add(element);
  element.addEventListener('wheel', handleStripWheel, { passive: false });
}
