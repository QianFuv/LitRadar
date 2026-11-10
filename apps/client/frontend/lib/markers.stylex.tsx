/**
 * Named StyleX markers for elements whose descendants style themselves from the element's state.
 *
 * @packageDocumentation
 */

import * as stylex from '@stylexjs/stylex';

/** The article search field; its icon hides while focus is inside the field. */
export const searchMarker = stylex.defineMarker();

/** The account menu trigger; its avatar ring follows hover and the open state. */
export const accountMenuMarker = stylex.defineMarker();

/** A relation graph node button; its marker and label follow hover. */
export const nodeMarker = stylex.defineMarker();

/** A switch root; its thumb follows the switch size. */
export const switchMarker = stylex.defineMarker();

/** A settings section; a later sibling section keeps its distance from it. */
export const settingsSectionMarker = stylex.defineMarker();
