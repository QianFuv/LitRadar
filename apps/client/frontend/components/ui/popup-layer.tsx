/**
 * Stacking layer shared by portaled popups.
 */

import * as stylex from '@stylexjs/stylex';

const styles = stylex.create({
  portal: {
    position: 'relative',
    zIndex: 50,
  },
});

/**
 * Lift a popup's portal container into one stacking layer above the app chrome. Base UI renders
 * a modal popup's outside-click backdrop beside the positioner rather than inside it, so without
 * this layer any positioned app region with a z-index (the workspace header, the navigation
 * rail) paints above the backdrop and keeps receiving hover and clicks while the popup is open.
 */
export const popupLayerStyle = styles.portal;
