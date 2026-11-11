/**
 * Keyframes shared across components, exposed as StyleX variables so other files can use them.
 */

import * as stylex from '@stylexjs/stylex';

/** Shared animation names. */
export const animations = stylex.defineVars({
  /** One full clockwise turn, for spinners and sweeps. */
  spin: stylex.keyframes({ to: { transform: 'rotate(360deg)' } }),
});
